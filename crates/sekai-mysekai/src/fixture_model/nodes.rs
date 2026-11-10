//! The keys of every node: the source identity of its game object, its components and children,
//! its renderer, its road cell and its collision record, and, on the root, the collision document.

use std::collections::BTreeMap;

use sekai_convert::model::{Node, NodeKeys, Prefab};
use sekai_convert::{Context, ConvertError, Object, ObjectId, Value};
use serde_json::{Value as Json, json};

use super::collision::Collision;
use super::scripts::is_script;
use crate::scripts::malformed;
use crate::stored::{array, field, number, stored_code};

/// The class ID of `MeshRenderer`.
const MESH_RENDERER: i32 = 23;
/// The class ID of `SkinnedMeshRenderer`.
const SKINNED_MESH_RENDERER: i32 = 137;
/// The script class of the behaviour of the game object the game places.
pub(super) const FIXTURE_VIEW: &str = ".FixtureView";
/// The script class of the behaviour of a road.
const ROAD_VIEW: &str = ".RoadView";

/// The keys of the nodes of a piece.
pub(super) struct PieceNodes<'a> {
    prefab: &'a Prefab,
    collision: &'a Collision,
    /// The road cell type of each game object that is a road cell.
    cells: BTreeMap<ObjectId, i64>,
}

impl<'a> PieceNodes<'a> {
    /// The keys of the nodes of `prefab`, whose collision is `collision`.
    pub(super) fn read(
        prefab: &'a Prefab,
        collision: &'a Collision,
        cx: &dyn Context,
    ) -> Result<Self, ConvertError> {
        Ok(Self {
            prefab,
            collision,
            cells: road_cells(prefab, cx)?,
        })
    }
}

impl NodeKeys for PieceNodes<'_> {
    fn keys(
        &self,
        object: ObjectId,
        cx: &dyn Context,
    ) -> Result<BTreeMap<String, Json>, ConvertError> {
        let node = self
            .prefab
            .node(object)
            .ok_or_else(|| malformed("a node is not a game object of the prefab"))?;
        let mut keys = BTreeMap::from([
            ("sourceObject".to_owned(), source_object(node, cx)?),
            (
                "sourceComponentIds".to_owned(),
                decimal_ids(node.component_ids()),
            ),
            ("sourceChildOrder".to_owned(), self.child_order(node)),
        ]);
        if let Some(renderer) = node.components().iter().find(|c| is_renderer(c)) {
            keys.extend(renderer_keys(renderer)?);
        }
        if let Some(&cell) = self.cells.get(&object) {
            keys.insert("roadCellType".to_owned(), json!(cell));
        }
        if let Some(record) = self.collision.record(object) {
            keys.insert("sourceCollision".to_owned(), record.clone());
        }
        if self.is_root(node) {
            keys.insert("fixtureViewRoot".to_owned(), json!(true));
            keys.insert("fixtureCollision".to_owned(), self.collision.document());
        }
        Ok(keys)
    }
}

impl PieceNodes<'_> {
    fn is_root(&self, node: &Node) -> bool {
        self.prefab
            .nodes()
            .first()
            .is_some_and(|root| root.object() == node.object())
    }

    /// The transform of each child of `node`, in the order its transform stores them.
    fn child_order(&self, node: &Node) -> Json {
        let nodes = self.prefab.nodes();
        let children = node
            .children()
            .iter()
            .map(|&child| nodes[child].transform());
        decimal_ids(&children.collect::<Vec<_>>())
    }
}

/// Fails unless the root game object of `prefab` carries a `FixtureView` and no game object below
/// it does: the root is the game object the game places, and its collision is read from it down.
pub(super) fn check_placement(prefab: &Prefab) -> Result<(), ConvertError> {
    let has_view = |node: &Node| node.components().iter().any(|c| is_script(c, FIXTURE_VIEW));
    match prefab.nodes().split_first() {
        Some((root, _)) if !has_view(root) => {
            Err(malformed("the root game object has no FixtureView"))
        }
        Some((_, below)) if below.iter().any(has_view) => {
            Err(malformed("a FixtureView is below the root game object"))
        }
        Some(_) => Ok(()),
        None => Err(malformed("the prefab has no game object")),
    }
}

/// The source identity of `node`: the serialized file of its game object, and the ids of the game
/// object and its transform.
pub(super) fn source_object(node: &Node, cx: &dyn Context) -> Result<Json, ConvertError> {
    Ok(json!({
        "file": cx.file_name(node.object())?,
        "gameObjectId": node.object().path_id.to_string(),
        "transformId": node.transform().path_id.to_string(),
    }))
}

/// Object ids as decimal strings, in order.
fn decimal_ids(ids: &[ObjectId]) -> Json {
    json!(
        ids.iter()
            .map(|id| id.path_id.to_string())
            .collect::<Vec<_>>()
    )
}

fn is_renderer(component: &Object) -> bool {
    matches!(component.class_id, MESH_RENDERER | SKINNED_MESH_RENDERER)
}

/// The keys of a renderer: the mode it casts shadows in, and whether it draws with a material,
/// that is whether some material slot names a material.
fn renderer_keys(renderer: &Object) -> Result<[(String, Json); 2], ConvertError> {
    let fields = &renderer.fields;
    let shadows = number(field(fields, "m_CastShadows")?, "m_CastShadows")?;
    let materialed = array(field(fields, "m_Materials")?, "m_Materials")?
        .iter()
        .filter_map(Value::as_pptr)
        .any(|material| material.path_id != 0);
    Ok([
        ("shadowCastingMode".to_owned(), shadows),
        ("materialed".to_owned(), json!(materialed)),
    ])
}

/// The road cell type of each game object that is a cell of a `RoadView` of `prefab`. Each entry
/// of a road's list names its cell, the cell names the renderer that draws it, and the renderer's
/// game object is the cell's node.
fn road_cells(prefab: &Prefab, cx: &dyn Context) -> Result<BTreeMap<ObjectId, i64>, ConvertError> {
    let roads = prefab
        .nodes()
        .iter()
        .flat_map(Node::components)
        .filter(|component| is_script(component, ROAD_VIEW));
    let mut cells = BTreeMap::new();
    for road in roads {
        for entry in array(
            field(&road.fields, "_roadViewDataList")?,
            "_roadViewDataList",
        )? {
            let cell = target(cx, road.id, field(entry, "_cell")?, "_cell")?;
            let renderer = target(cx, cell.id, field(&cell.fields, "_renderer")?, "_renderer")?;
            let owner = target(
                cx,
                renderer.id,
                field(&renderer.fields, "m_GameObject")?,
                "m_GameObject",
            )?;
            let cell_type = stored_code(field(entry, "_cellType")?, "_cellType")?;
            cells.insert(owner.id, cell_type);
        }
    }
    Ok(cells)
}

/// The object the reference `stored`, stored in the object `from` as `name`, points to.
fn target(
    cx: &dyn Context,
    from: ObjectId,
    stored: &Value,
    name: &str,
) -> Result<Object, ConvertError> {
    let reference = stored
        .as_pptr()
        .ok_or_else(|| malformed(format!("{name} is not a reference")))?;
    let id = cx
        .resolve(from, reference)?
        .ok_or_else(|| malformed(format!("{name} is null")))?;
    cx.object(id)
}
