//! The collision of the game objects of one or more prefabs: the collision meshes their colliders
//! use, and the record of each game object that carries a collider or a navigation component.

use std::collections::{BTreeMap, BTreeSet};

use sekai_convert::model::{Node, Prefab, mesh_triangles};
use sekai_convert::{Context, ConvertError, Object, ObjectId, Value};
use serde_json::{Map, Value as Json, json};

use super::scripts::is_script;
use crate::scripts::malformed;
use crate::stored::{array, field, flag, number, stored_code};

/// The class ID of `Mesh`.
const MESH: i32 = 43;
/// The class IDs of the colliders.
const MESH_COLLIDER: i32 = 64;
const BOX_COLLIDER: i32 = 65;
const SPHERE_COLLIDER: i32 = 135;
const CAPSULE_COLLIDER: i32 = 136;
/// The class ID of `NavMeshAgent`.
const NAV_MESH_AGENT: i32 = 195;
/// The class ID of `NavMeshObstacle`.
const NAV_MESH_OBSTACLE: i32 = 208;
/// The script class of a navigation modifier.
const NAV_MESH_MODIFIER: &str = ".NavMeshModifier";
/// The keys of a collider that only some kinds use, `null` for the others.
const SHAPE_KEYS: [&str; 9] = [
    "convex",
    "cookingOptions",
    "mesh",
    "geometryId",
    "center",
    "size",
    "radius",
    "height",
    "direction",
];

/// What the mesh of a mesh collider must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColliderMeshes {
    /// A mesh of a bundle: a collider without a mesh is `malformed`, and one with a mesh of the
    /// engine's own resources is the error of the missing object.
    Required,
    /// Whatever the collider stores. A collider without a mesh, or with a mesh of the engine's
    /// own resources, has a record whose `mesh` and `geometryId` are `null`, and every mesh
    /// collider has the key `engineMesh`: the engine's mesh, or `null`.
    AsAuthored,
}

/// The collision of the game objects of one or more prefabs.
#[derive(Debug)]
pub(crate) struct Collision {
    /// The geometry of each collision mesh, by geometry id.
    geometry: BTreeMap<String, Json>,
    /// The record of each game object that has one.
    records: BTreeMap<ObjectId, Json>,
    meshes: ColliderMeshes,
}

impl Collision {
    /// No collision yet, with the meshes of mesh colliders taken as `meshes` says.
    pub(crate) fn new(meshes: ColliderMeshes) -> Self {
        Self {
            geometry: BTreeMap::new(),
            records: BTreeMap::new(),
            meshes,
        }
    }

    /// Reads the collision of every game object of `prefab`, in which every mesh collider has a
    /// mesh.
    pub(crate) fn read(prefab: &Prefab, cx: &dyn Context) -> Result<Self, ConvertError> {
        let mut collision = Self::new(ColliderMeshes::Required);
        collision.add(prefab, cx)?;
        Ok(collision)
    }

    /// Adds the collision of every game object of `prefab`. A collision mesh that a game object
    /// read before already uses is not added again.
    pub(crate) fn add(&mut self, prefab: &Prefab, cx: &dyn Context) -> Result<(), ConvertError> {
        self.walk(prefab.nodes(), 0, true, cx)
    }

    /// The record of the game object `game_object`, when it has one.
    pub(crate) fn record(&self, game_object: ObjectId) -> Option<&Json> {
        self.records.get(&game_object)
    }

    /// The collision document: the geometry of every collision mesh, in order of geometry id.
    pub(crate) fn document(&self) -> Json {
        json!({ "geometry": self.geometry.values().collect::<Vec<_>>() })
    }

    /// Adds the records of the node at `index` and of every node below it, whose parent is
    /// active in the hierarchy when `parent_active` is set.
    fn walk(
        &mut self,
        nodes: &[Node],
        index: usize,
        parent_active: bool,
        cx: &dyn Context,
    ) -> Result<(), ConvertError> {
        let node = &nodes[index];
        let active = parent_active && node.active();
        if let Some(record) = self.node_record(node, active, cx)? {
            self.records.insert(node.object(), record);
        }
        for &child in node.children() {
            self.walk(nodes, child, active, cx)?;
        }
        Ok(())
    }

    /// The record of `node`, or `None` when it carries neither a collider nor a navigation
    /// component.
    fn node_record(
        &mut self,
        node: &Node,
        active_in_hierarchy: bool,
        cx: &dyn Context,
    ) -> Result<Option<Json>, ConvertError> {
        let mut found = Components::default();
        for component in node.components() {
            match component.class_id {
                MESH_COLLIDER | BOX_COLLIDER | SPHERE_COLLIDER | CAPSULE_COLLIDER => {
                    found.colliders.push(self.collider(component, cx)?);
                }
                NAV_MESH_AGENT => found.agents.push(agent(component)?),
                NAV_MESH_OBSTACLE => found.obstacles.push(obstacle(component)?),
                _ if is_script(component, NAV_MESH_MODIFIER) => {
                    found.modifiers.push(modifier(component)?);
                }
                _ => {}
            }
        }
        if found.is_empty() {
            return Ok(None);
        }
        let game_object = cx.object(node.object())?;
        let geometry_ids: BTreeSet<&str> = found
            .colliders
            .iter()
            .filter_map(|collider| collider["geometryId"].as_str())
            .collect();
        Ok(Some(json!({
            "gameObjectId": node.object().path_id.to_string(),
            "transformId": node.transform().path_id.to_string(),
            "activeSelf": node.active(),
            "activeInHierarchy": active_in_hierarchy,
            "authoredLayer": stored_code(field(&game_object.fields, "m_Layer")?, "m_Layer")?,
            "colliders": found.colliders,
            "geometryIds": geometry_ids,
            "modifiers": found.modifiers,
            "hasNavMeshAgent": !found.agents.is_empty(),
            "navMeshAgents": found.agents,
            "navMeshObstacles": found.obstacles,
        })))
    }

    /// The record of a collider. The geometry of a mesh collider's mesh is added to the document
    /// the first time a collider uses it.
    fn collider(&mut self, component: &Object, cx: &dyn Context) -> Result<Json, ConvertError> {
        let fields = &component.fields;
        let (kind, shape) = match component.class_id {
            MESH_COLLIDER => ("MeshCollider", self.mesh_shape(component, cx)?),
            BOX_COLLIDER => ("BoxCollider", box_shape(fields)?),
            SPHERE_COLLIDER => ("SphereCollider", sphere_shape(fields)?),
            _ => ("CapsuleCollider", capsule_shape(fields)?),
        };
        let unused = SHAPE_KEYS
            .iter()
            .filter(|key| !shape.contains_key(**key))
            .map(|key| (key.to_string(), Json::Null))
            .collect::<Vec<_>>();
        let mut record = Map::from_iter([
            (
                "componentId".to_owned(),
                json!(component.id.path_id.to_string()),
            ),
            ("kind".to_owned(), json!(kind)),
            ("enabled".to_owned(), json!(enabled(fields)?)),
            (
                "isTrigger".to_owned(),
                json!(is_set(fields, "m_IsTrigger")?),
            ),
            (
                "includeLayers".to_owned(),
                layers(fields, "m_IncludeLayers")?,
            ),
            (
                "excludeLayers".to_owned(),
                layers(fields, "m_ExcludeLayers")?,
            ),
            (
                "layerOverridePriority".to_owned(),
                optional(fields, "m_LayerOverridePriority", |v, n| {
                    Ok(json!(stored_code(v, n)?))
                })?,
            ),
            (
                "providesContacts".to_owned(),
                optional(fields, "m_ProvidesContacts", |v, n| Ok(json!(flag(v, n)?)))?,
            ),
        ]);
        record.extend(shape);
        record.extend(unused);
        Ok(Json::Object(record))
    }

    /// The keys of a mesh collider, adding its mesh's geometry when no collider used it before.
    fn mesh_shape(
        &mut self,
        component: &Object,
        cx: &dyn Context,
    ) -> Result<Map<String, Json>, ConvertError> {
        let fields = &component.fields;
        let stored = field(fields, "m_Mesh")?
            .as_pptr()
            .ok_or_else(|| malformed("m_Mesh is not a reference"))?;
        let cooking = (
            "cookingOptions".to_owned(),
            json!(stored_code(
                field(fields, "m_CookingOptions")?,
                "m_CookingOptions"
            )?),
        );
        let convex = ("convex".to_owned(), json!(is_set(fields, "m_Convex")?));
        let as_authored = self.meshes == ColliderMeshes::AsAuthored;
        let unresolved = |engine_mesh: Json| {
            Map::from_iter([
                convex.clone(),
                cooking.clone(),
                ("mesh".to_owned(), Json::Null),
                ("geometryId".to_owned(), Json::Null),
                ("engineMesh".to_owned(), engine_mesh),
            ])
        };
        if as_authored && let Some(engine) = cx.engine_object(component.id, stored)? {
            let file = engine.file;
            let path_id = engine.path_id.to_string();
            return Ok(unresolved(json!({ "file": file, "pathId": path_id })));
        }
        let Some(mesh) = cx.resolve(component.id, stored)? else {
            return if as_authored {
                Ok(unresolved(Json::Null))
            } else {
                Err(malformed("a mesh collider has no mesh"))
            };
        };
        if cx.object(mesh)?.class_id != MESH {
            return Err(malformed("the mesh of a mesh collider is not a Mesh"));
        }
        let geometry_id = format!("{}:{}", cx.file_name(mesh)?, mesh.path_id);
        if !self.geometry.contains_key(&geometry_id) {
            let geometry = geometry(&geometry_id, mesh, cx)?;
            self.geometry.insert(geometry_id.clone(), geometry);
        }
        let mut keys = Map::from_iter([
            convex,
            cooking,
            (
                "mesh".to_owned(),
                json!({ "fileId": stored.file_id, "pathId": stored.path_id.to_string() }),
            ),
            ("geometryId".to_owned(), json!(geometry_id)),
        ]);
        if as_authored {
            keys.insert("engineMesh".to_owned(), Json::Null);
        }
        Ok(keys)
    }
}

/// The components of a game object that its record lists.
#[derive(Default)]
struct Components {
    colliders: Vec<Json>,
    modifiers: Vec<Json>,
    agents: Vec<Json>,
    obstacles: Vec<Json>,
}

impl Components {
    fn is_empty(&self) -> bool {
        self.colliders.is_empty()
            && self.modifiers.is_empty()
            && self.agents.is_empty()
            && self.obstacles.is_empty()
    }
}

/// The geometry of the collision mesh `mesh`.
fn geometry(geometry_id: &str, mesh: ObjectId, cx: &dyn Context) -> Result<Json, ConvertError> {
    let triangles = mesh_triangles(mesh, cx)?;
    // Adding zero writes a reflected zero as 0 rather than -0.
    let positions = triangles
        .positions
        .iter()
        .map(|position| position.map(|axis| axis + 0.0))
        .collect::<Vec<_>>();
    Ok(json!({
        "geometryId": geometry_id,
        "positions": positions,
        "triangles": triangles.triangles,
    }))
}

fn box_shape(fields: &Value) -> Result<Map<String, Json>, ConvertError> {
    Ok(Map::from_iter([
        (
            "center".to_owned(),
            json!(runtime(vector3(fields, "m_Center")?)),
        ),
        ("size".to_owned(), json!(vector3(fields, "m_Size")?)),
    ]))
}

fn sphere_shape(fields: &Value) -> Result<Map<String, Json>, ConvertError> {
    Ok(Map::from_iter([
        (
            "center".to_owned(),
            json!(runtime(vector3(fields, "m_Center")?)),
        ),
        (
            "radius".to_owned(),
            number(field(fields, "m_Radius")?, "m_Radius")?,
        ),
    ]))
}

fn capsule_shape(fields: &Value) -> Result<Map<String, Json>, ConvertError> {
    Ok(Map::from_iter([
        (
            "center".to_owned(),
            json!(runtime(vector3(fields, "m_Center")?)),
        ),
        (
            "radius".to_owned(),
            number(field(fields, "m_Radius")?, "m_Radius")?,
        ),
        (
            "height".to_owned(),
            number(field(fields, "m_Height")?, "m_Height")?,
        ),
        (
            "direction".to_owned(),
            json!(stored_code(field(fields, "m_Direction")?, "m_Direction")?),
        ),
    ]))
}

/// The record of a navigation modifier.
fn modifier(component: &Object) -> Result<Json, ConvertError> {
    let fields = &component.fields;
    let affected = array(field(fields, "m_AffectedAgents")?, "m_AffectedAgents")?
        .iter()
        .map(|agent| stored_code(agent, "m_AffectedAgents"))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "componentId": component.id.path_id.to_string(),
        "enabled": enabled(fields)?,
        "overrideArea": is_set(fields, "m_OverrideArea")?,
        "area": stored_code(field(fields, "m_Area")?, "m_Area")?,
        "ignoreFromBuild": is_set(fields, "m_IgnoreFromBuild")?,
        "affectedAgents": affected,
    }))
}

/// The record of a navigation agent.
fn agent(component: &Object) -> Result<Json, ConvertError> {
    let fields = &component.fields;
    Ok(json!({
        "componentId": component.id.path_id.to_string(),
        "enabled": enabled(fields)?,
        "agentTypeId": stored_code(field(fields, "m_AgentTypeID")?, "m_AgentTypeID")?,
        "radius": number(field(fields, "m_Radius")?, "m_Radius")?,
        "height": number(field(fields, "m_Height")?, "m_Height")?,
        "baseOffset": number(field(fields, "m_BaseOffset")?, "m_BaseOffset")?,
    }))
}

/// The record of a navigation obstacle.
fn obstacle(component: &Object) -> Result<Json, ConvertError> {
    let fields = &component.fields;
    Ok(json!({
        "componentId": component.id.path_id.to_string(),
        "enabled": enabled(fields)?,
        "shape": stored_code(field(fields, "m_Shape")?, "m_Shape")?,
        "center": runtime(vector3(fields, "m_Center")?),
        "extents": vector3(fields, "m_Extents")?,
        "carve": is_set(fields, "m_Carve")?,
        "onlyStationary": is_set(fields, "m_CarveOnlyStationary")?,
        "moveThreshold": number(field(fields, "m_MoveThreshold")?, "m_MoveThreshold")?,
        "stationaryTime": number(field(fields, "m_TimeToStationary")?, "m_TimeToStationary")?,
    }))
}

/// The stored flag `m_Enabled`.
fn enabled(fields: &Value) -> Result<bool, ConvertError> {
    is_set(fields, "m_Enabled")
}

fn is_set(fields: &Value, name: &str) -> Result<bool, ConvertError> {
    flag(field(fields, name)?, name)
}

/// A stored vector of three finite numbers.
fn vector3(fields: &Value, name: &str) -> Result<[f64; 3], ConvertError> {
    let vector = field(fields, name)?;
    let component = |axis| {
        number(field(vector, axis)?, name)?
            .as_f64()
            .ok_or_else(|| malformed(format!("{name} is not a vector")))
    };
    Ok([component("x")?, component("y")?, component("z")?])
}

/// A point in the runtime frame: the source's with x reflected. A zero stays positive.
fn runtime([x, y, z]: [f64; 3]) -> [f64; 3] {
    [0.0 - x, y, z]
}

/// The layer mask `m_Bits` of the field `name`, or `null` when the engine does not store the
/// field.
fn layers(fields: &Value, name: &str) -> Result<Json, ConvertError> {
    optional(fields, name, |mask, name| {
        Ok(json!(stored_code(field(mask, "m_Bits")?, name)?))
    })
}

/// The field `name` read by `read`, or `null` when the engine does not store the field.
fn optional(
    fields: &Value,
    name: &str,
    read: impl FnOnce(&Value, &str) -> Result<Json, ConvertError>,
) -> Result<Json, ConvertError> {
    fields
        .get(name)
        .map_or(Ok(Json::Null), |value| read(value, name))
}
