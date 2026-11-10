//! The `site-model` form on objects built in memory.

use sekai_convert::{
    Asset, Chunk, Content, Context, ConvertError, Converter, EngineObject, Object, ObjectId,
    ObjectIndex, PPtr, Value,
};
use sekai_mysekai::site_model::SiteModel;
use serde_json::{Value as Json, json};

type Fields = Vec<(&'static str, Value)>;

/// The file index that references to the engine's own resources use in these objects.
const ENGINE: i32 = 2;
const ENGINE_FILE: &str = "Library/unity default resources";

/// The container directory of the packages of the places.
const SITES: &str = "assets/sekai/assetbundle/resources/ondemand/mysekai/site/";

/// One bundle with one file, holding the given objects, with the given container.
struct File {
    objects: Vec<Object>,
    container: Vec<(String, i64)>,
}

fn id(path_id: i64) -> ObjectId {
    ObjectId {
        bundle: 0,
        file: 0,
        path_id,
    }
}

impl Context for File {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        self.objects
            .iter()
            .find(|object| object.id == id)
            .cloned()
            .ok_or_else(|| ConvertError::new("missing_object", format!("no object {}", id.path_id)))
    }

    fn resolve(&self, _from: ObjectId, reference: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        match reference.file_id {
            0 => Ok((reference.path_id != 0).then(|| id(reference.path_id))),
            _ => Err(ConvertError::new(
                "missing_object",
                "an object of no bundle",
            )),
        }
    }

    fn resource(
        &self,
        _from: ObjectId,
        path: &str,
        _: u64,
        _: u64,
    ) -> Result<Chunk<'_>, ConvertError> {
        Err(ConvertError::new("missing_resource", path))
    }

    fn container(&self, _from: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        Ok(self
            .container
            .iter()
            .map(|(path, object)| (path.clone(), id(*object)))
            .collect())
    }

    fn file_name(&self, _from: ObjectId) -> Result<&str, ConvertError> {
        Ok("CAB-site")
    }

    fn engine_object(
        &self,
        _from: ObjectId,
        reference: PPtr,
    ) -> Result<Option<EngineObject>, ConvertError> {
        Ok((reference.file_id == ENGINE).then(|| EngineObject {
            file: ENGINE_FILE.to_owned(),
            path_id: reference.path_id,
        }))
    }
}

impl File {
    fn convert(&self, root: i64) -> Result<Option<Asset>, ConvertError> {
        SiteModel.convert(id(root), self)
    }

    fn asset(&self, root: i64) -> Asset {
        self.convert(root)
            .expect("the package converts")
            .expect("the root is the principal prefab")
    }
}

fn s(fields: Fields) -> Value {
    Value::Struct(fields.into_iter().map(|(n, v)| (n.into(), v)).collect())
}

fn f(v: f32) -> Value {
    Value::Float(v.into())
}

fn i(v: i64) -> Value {
    Value::Int(v)
}

fn u(v: u64) -> Value {
    Value::UInt(v)
}

fn text(v: &str) -> Value {
    Value::String(v.into())
}

fn list(items: Vec<Value>) -> Value {
    Value::Array(items)
}

/// A reference to the object with the path ID `path_id`, or a null reference for 0.
fn r(path_id: i64) -> Value {
    Value::PPtr(PPtr {
        file_id: 0,
        path_id,
    })
}

/// A reference to the object `path_id` of the engine's own resources.
fn engine(path_id: i64) -> Value {
    Value::PPtr(PPtr {
        file_id: ENGINE,
        path_id,
    })
}

fn xyz(x: f32, y: f32, z: f32) -> Value {
    s(vec![("x", f(x)), ("y", f(y)), ("z", f(z))])
}

fn object(path_id: i64, class_id: i32, class: &str, fields: Value) -> Object {
    Object {
        id: id(path_id),
        class_id,
        class: class.into(),
        script: None,
        fields,
    }
}

fn game_object(path_id: i64, name: &str, components: &[i64]) -> Object {
    let entries = components
        .iter()
        .map(|&component| s(vec![("component", r(component))]))
        .collect();
    object(
        path_id,
        1,
        "GameObject",
        s(vec![
            ("m_Component", list(entries)),
            ("m_Layer", i(0)),
            ("m_Name", text(name)),
            ("m_IsActive", Value::Bool(true)),
        ]),
    )
}

fn transform(path_id: i64, owner: i64, father: i64, children: &[i64]) -> Object {
    object(
        path_id,
        4,
        "Transform",
        s(vec![
            ("m_GameObject", r(owner)),
            (
                "m_LocalRotation",
                s(vec![
                    ("x", f(0.0)),
                    ("y", f(0.0)),
                    ("z", f(0.0)),
                    ("w", f(1.0)),
                ]),
            ),
            ("m_LocalPosition", xyz(1.0, 0.0, 0.0)),
            ("m_LocalScale", xyz(1.0, 1.0, 1.0)),
            ("m_Children", list(children.iter().map(|&c| r(c)).collect())),
            ("m_Father", r(father)),
        ]),
    )
}

/// A `MeshRenderer` on the game object `owner` with an empty material slot.
fn renderer(path_id: i64, owner: i64) -> Object {
    object(
        path_id,
        23,
        "MeshRenderer",
        s(vec![
            ("m_GameObject", r(owner)),
            ("m_Enabled", u(1)),
            ("m_CastShadows", u(0)),
            ("m_Materials", list(vec![r(0)])),
        ]),
    )
}

fn mesh_filter(path_id: i64, owner: i64, mesh: Value) -> Object {
    object(
        path_id,
        33,
        "MeshFilter",
        s(vec![("m_GameObject", r(owner)), ("m_Mesh", mesh)]),
    )
}

fn mesh_collider(path_id: i64, owner: i64, mesh: Value) -> Object {
    object(
        path_id,
        64,
        "MeshCollider",
        s(vec![
            ("m_GameObject", r(owner)),
            ("m_Material", r(0)),
            ("m_IsTrigger", Value::Bool(false)),
            ("m_Enabled", Value::Bool(true)),
            ("m_Convex", Value::Bool(false)),
            ("m_CookingOptions", i(30)),
            ("m_Mesh", mesh),
        ]),
    )
}

/// A vertex channel of the stored layout.
fn channel(offset: u64, format: u64, dimension: u64) -> Value {
    s(vec![
        ("stream", u(0)),
        ("offset", u(offset)),
        ("format", u(format)),
        ("dimension", u(dimension)),
    ])
}

/// A mesh named `name` of one triangle, with positions only.
fn mesh(path_id: i64, name: &str) -> Object {
    let indices: Vec<u8> = [0u16, 2, 1].iter().flat_map(|v| v.to_le_bytes()).collect();
    let mut channels = vec![channel(0, 0, 3)];
    channels.extend((1..12).map(|_| channel(0, 0, 0)));
    let positions: Vec<u8> = [[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]]
        .iter()
        .flatten()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    object(
        path_id,
        43,
        "Mesh",
        s(vec![
            ("m_Name", text(name)),
            (
                "m_SubMeshes",
                list(vec![s(vec![
                    ("firstByte", u(0)),
                    ("indexCount", u(3)),
                    ("topology", i(0)),
                    ("baseVertex", u(0)),
                ])]),
            ),
            ("m_BindPose", list(Vec::new())),
            (
                "m_Shapes",
                s(vec![
                    ("vertices", list(Vec::new())),
                    ("shapes", list(Vec::new())),
                    ("channels", list(Vec::new())),
                    ("fullWeights", list(Vec::new())),
                ]),
            ),
            (
                "m_VariableBoneCountWeights",
                s(vec![("m_Data", list(Vec::new()))]),
            ),
            ("m_IndexFormat", i(0)),
            ("m_IndexBuffer", Value::Bytes(indices)),
            (
                "m_VertexData",
                s(vec![
                    ("m_VertexCount", u(3)),
                    ("m_Channels", list(channels)),
                    ("m_DataSize", Value::Bytes(positions)),
                ]),
            ),
            (
                "m_CompressedMesh",
                s(vec![("m_Vertices", s(vec![("m_NumItems", u(0))]))]),
            ),
        ]),
    )
}

fn path(rest: &str) -> String {
    format!("{SITES}{rest}")
}

/// The package `field/home`: the field `home` with a ground child and a walk surface whose
/// collider uses the ground mesh, the obstacles `rank1`, and the source model of the ground, which
/// is not a prefab.
fn home() -> File {
    File {
        objects: vec![
            game_object(1, "home", &[2]),
            transform(2, 1, 0, &[4, 9]),
            game_object(3, "ground", &[4, 5, 6]),
            transform(4, 3, 2, &[]),
            renderer(5, 3),
            mesh_filter(6, 3, r(7)),
            mesh(7, "ground"),
            game_object(8, "mdl_site_base_homesite_nav_ground", &[9, 10]),
            transform(9, 8, 2, &[]),
            mesh_collider(10, 8, r(7)),
            game_object(21, "rank1", &[22, 23, 24]),
            transform(22, 21, 0, &[]),
            renderer(23, 21),
            mesh_filter(24, 21, r(7)),
            game_object(31, "mdl_site_base_homesite_ground01", &[32]),
            transform(32, 31, 0, &[]),
        ],
        container: vec![
            (path("field/home/obstacle/rank1.prefab"), 21),
            (
                path("field/home/fbx/mdl_site_base_homesite_ground01.fbx"),
                31,
            ),
            (path("field/home/home.prefab"), 1),
            (path("field/home/texture/tex_ground.png"), 7),
        ],
    }
}

/// The model of an asset, read back from its GLB file.
fn model(asset: &Asset) -> Json {
    let bytes = member(asset, "model");
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    assert_eq!(&bytes[..4], b"glTF");
    serde_json::from_slice(&bytes[20..20 + word(12)]).expect("the JSON chunk")
}

fn member<'a>(asset: &'a Asset, role: &str) -> &'a [u8] {
    let member = asset
        .members
        .iter()
        .find(|member| member.role == role)
        .unwrap_or_else(|| panic!("the asset has a {role} member"));
    let Content::File { bytes, .. } = &member.content else {
        panic!("the {role} member is a file")
    };
    bytes
}

/// The extras of the node named `name`.
fn extras(model: &Json, name: &str) -> Json {
    let nodes = model["nodes"].as_array().expect("nodes");
    let node = nodes
        .iter()
        .find(|node| node["name"] == name)
        .unwrap_or_else(|| panic!("the node {name} exists"));
    node.get("extras").cloned().unwrap_or(Json::Null)
}

fn object_index() -> ObjectIndex {
    ObjectIndex {
        path_id: 1,
        class_id: 1,
        class: "GameObject".into(),
        name: None,
        script: None,
        script_name: None,
        references: Vec::new(),
        size: 0,
        sha256: [0; 32],
        error: None,
    }
}

fn is_root(path: &str) -> bool {
    SiteModel.is_root(&object_index(), None, &[path.to_owned()])
}

#[test]
fn every_prefab_of_the_places_is_a_root_but_the_map_and_source_models() {
    assert!(is_root(&path("field/home/home.prefab")));
    assert!(is_root(&path("field/home/obstacle/rank1.prefab")));
    assert!(is_root(&path("HOUSE/LV_01/MDL_STATIC_FLOOR.PREFAB")));
    assert!(!is_root(&path("field/home/fbx/ground01.fbx")));
    assert!(!is_root(&path("sitemap/prefab/open_cloud.prefab")));
    assert!(!is_root(
        "assets/sekai/assetbundle/resources/ondemand/mysekai/fixture/a/a.prefab"
    ));
}

#[test]
fn each_prefab_is_a_scene_and_the_principal_prefab_is_shown_first() {
    let model = model(&home().asset(1));
    assert_eq!(
        model["scenes"],
        json!([
            { "name": "home", "nodes": [0] },
            { "name": "rank1", "nodes": [3] },
        ])
    );
    assert_eq!(model["scene"], json!(0));
    assert_eq!(model["nodes"][0]["mesh"], Json::Null);
    assert_eq!(model["nodes"][1]["mesh"], model["nodes"][3]["mesh"]);
    assert_eq!(model["meshes"][0]["name"], json!("ground"));
}

#[test]
fn only_the_principal_prefab_writes_the_package() {
    let file = home();
    assert!(file.convert(21).expect("no error").is_none());
    assert_eq!(SiteModel.inputs(id(1), &file), vec![id(21)]);
    assert!(SiteModel.inputs(id(21), &file).is_empty());
}

#[test]
fn a_package_without_a_prefab_named_after_it_shows_its_first_prefab() {
    let mut file = home();
    file.container = vec![
        (path("house/lv_01/mdl_static_wall.prefab"), 1),
        (path("house/lv_01/mdl_static_floor.prefab"), 21),
    ];
    assert!(file.convert(1).expect("no error").is_none());
    let model = model(&file.asset(21));
    assert_eq!(model["scenes"][0]["name"], json!("rank1"));
    assert_eq!(model["scenes"][1]["name"], json!("home"));
    assert_eq!(model["scene"], json!(0));
}

#[test]
fn nodes_have_the_source_keys_and_none_of_furniture() {
    let model = model(&home().asset(1));
    let ground = extras(&model, "ground");
    assert_eq!(
        ground["sourceObject"],
        json!({ "file": "CAB-site", "gameObjectId": "3", "transformId": "4" })
    );
    assert_eq!(ground["sourceComponentIds"], json!(["4", "5", "6"]));
    assert_eq!(ground["shadowCastingMode"], json!(0));
    assert_eq!(ground["materialed"], json!(false));
    assert_eq!(
        extras(&model, "home")["sourceChildOrder"],
        json!(["4", "9"])
    );
    let walk = extras(&model, "mdl_site_base_homesite_nav_ground")["sourceCollision"].clone();
    assert!(walk.get("fixtureNavLayer").is_none());
    for key in ["fixtureViewRoot", "fixtureCollision", "roadCellType"] {
        assert!(
            model["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|n| n["extras"].get(key).is_none())
        );
    }
}

#[test]
fn the_collision_meshes_are_a_member_beside_the_file() {
    let asset = home().asset(1);
    let collision: Json = serde_json::from_slice(member(&asset, "collision")).expect("JSON");
    let geometry = collision["geometry"].as_array().expect("geometry");
    assert_eq!(geometry.len(), 1);
    assert_eq!(geometry[0]["geometryId"], json!("CAB-site:7"));
    assert_eq!(geometry[0]["triangles"], json!([[0, 1, 2]]));
    let model = model(&asset);
    let collider =
        &extras(&model, "mdl_site_base_homesite_nav_ground")["sourceCollision"]["colliders"][0];
    assert_eq!(collider["geometryId"], json!("CAB-site:7"));
    assert_eq!(collider["engineMesh"], Json::Null);
}

#[test]
fn a_mesh_collider_without_a_mesh_is_recorded_as_authored() {
    let mut file = home();
    file.objects[9] = mesh_collider(10, 8, r(0));
    let asset = file.asset(1);
    let collider = &extras(&model(&asset), "mdl_site_base_homesite_nav_ground")["sourceCollision"]
        ["colliders"][0];
    assert_eq!(collider["mesh"], Json::Null);
    assert_eq!(collider["geometryId"], Json::Null);
    let collision: Json = serde_json::from_slice(member(&asset, "collision")).expect("JSON");
    assert_eq!(collision, json!({ "geometry": [] }));
}

#[test]
fn a_mesh_of_the_engine_is_named_and_not_drawn() {
    let mut file = home();
    file.objects[5] = mesh_filter(6, 3, engine(10210));
    file.objects[9] = mesh_collider(10, 8, engine(10209));
    let model = model(&file.asset(1));
    let ground = extras(&model, "ground");
    assert_eq!(
        ground["engineMesh"],
        json!({ "file": ENGINE_FILE, "pathId": "10210" })
    );
    assert_eq!(model["nodes"][1]["mesh"], Json::Null);
    let collider =
        &extras(&model, "mdl_site_base_homesite_nav_ground")["sourceCollision"]["colliders"][0];
    assert_eq!(
        collider["engineMesh"],
        json!({ "file": ENGINE_FILE, "pathId": "10209" })
    );
    assert_eq!(collider["geometryId"], Json::Null);
}
