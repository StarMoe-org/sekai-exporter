//! The `fixture-model` form on objects built in memory.

use sekai_convert::{
    Asset, Chunk, Content, Context, ConvertError, Converter, Object, ObjectId, ObjectIndex, PPtr,
    Value,
};
use sekai_mysekai::fixture_model::FixtureModel;
use serde_json::{Value as Json, json};

type Fields = Vec<(&'static str, Value)>;

/// One bundle with one file, holding the given objects.
struct File(Vec<Object>);

fn id(path_id: i64) -> ObjectId {
    ObjectId {
        bundle: 0,
        file: 0,
        path_id,
    }
}

impl Context for File {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        self.0
            .iter()
            .find(|object| object.id == id)
            .cloned()
            .ok_or_else(|| ConvertError::new("missing_object", format!("no object {}", id.path_id)))
    }

    fn resolve(&self, _from: ObjectId, reference: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        assert_eq!(reference.file_id, 0);
        Ok((reference.path_id != 0).then(|| id(reference.path_id)))
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
        Ok(Vec::new())
    }

    fn file_name(&self, _from: ObjectId) -> Result<&str, ConvertError> {
        Ok("CAB-test")
    }
}

impl File {
    fn object_mut(&mut self, path_id: i64) -> &mut Object {
        self.0
            .iter_mut()
            .find(|object| object.id.path_id == path_id)
            .unwrap_or_else(|| panic!("object {path_id} exists"))
    }

    fn convert(&self) -> Result<Asset, ConvertError> {
        FixtureModel
            .convert(id(1), self)?
            .ok_or_else(|| ConvertError::new("malformed", "the form has no asset"))
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

fn xyz(x: f32, y: f32, z: f32) -> Value {
    s(vec![("x", f(x)), ("y", f(y)), ("z", f(z))])
}

fn object(path_id: i64, class_id: i32, class: &str, script: Option<&str>, fields: Value) -> Object {
    Object {
        id: id(path_id),
        class_id,
        class: class.into(),
        script: script.map(str::to_owned),
        fields,
    }
}

fn game_object(path_id: i64, name: &str, active: bool, components: &[i64]) -> Object {
    let entries = components
        .iter()
        .map(|&component| s(vec![("component", r(component))]))
        .collect();
    object(
        path_id,
        1,
        "GameObject",
        None,
        s(vec![
            ("m_Component", list(entries)),
            ("m_Layer", i(0)),
            ("m_Name", text(name)),
            ("m_IsActive", Value::Bool(active)),
        ]),
    )
}

fn transform(path_id: i64, owner: i64, position: [f32; 3], children: &[i64]) -> Object {
    let [x, y, z] = position;
    object(
        path_id,
        4,
        "Transform",
        None,
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
            ("m_LocalPosition", xyz(x, y, z)),
            ("m_LocalScale", xyz(1.0, 1.0, 1.0)),
            ("m_Children", list(children.iter().map(|&c| r(c)).collect())),
            ("m_Father", r(0)),
        ]),
    )
}

/// A `MonoBehaviour` of `script` on the game object `owner`.
fn behaviour(path_id: i64, owner: i64, script: &str, fields: Fields) -> Object {
    let mut all = vec![
        ("m_GameObject", r(owner)),
        ("m_Enabled", u(1)),
        ("m_Script", r(0)),
        ("m_Name", text("")),
    ];
    all.extend(fields);
    object(
        path_id,
        114,
        "MonoBehaviour",
        Some(&format!("Assembly-CSharp/Sekai.Mysekai.{script}")),
        s(all),
    )
}

/// The components every collider has beside its shape, as a version new enough to store them does.
fn collider_fields() -> Fields {
    vec![
        ("m_Material", r(0)),
        ("m_IsTrigger", Value::Bool(false)),
        ("m_Enabled", Value::Bool(true)),
        ("m_IncludeLayers", s(vec![("m_Bits", u(0x0000_0000))])),
        ("m_ExcludeLayers", s(vec![("m_Bits", u(0x0000_03ff))])),
        ("m_LayerOverridePriority", i(-1)),
        ("m_ProvidesContacts", Value::Bool(false)),
    ]
}

fn collider(path_id: i64, owner: i64, class_id: i32, class: &str, shape: Fields) -> Object {
    let mut all = vec![("m_GameObject", r(owner))];
    all.extend(collider_fields());
    all.extend(shape);
    object(path_id, class_id, class, None, s(all))
}

fn box_collider(path_id: i64, owner: i64, center: [f32; 3], size: [f32; 3]) -> Object {
    let [x, y, z] = size;
    collider(
        path_id,
        owner,
        65,
        "BoxCollider",
        vec![
            ("m_Size", xyz(x, y, z)),
            ("m_Center", xyz(center[0], center[1], center[2])),
        ],
    )
}

fn mesh_collider(path_id: i64, owner: i64, mesh: i64) -> Object {
    collider(
        path_id,
        owner,
        64,
        "MeshCollider",
        vec![
            ("m_Convex", Value::Bool(true)),
            ("m_CookingOptions", i(30)),
            ("m_Mesh", r(mesh)),
        ],
    )
}

fn obstacle(path_id: i64, owner: i64) -> Object {
    object(
        path_id,
        208,
        "NavMeshObstacle",
        None,
        s(vec![
            ("m_GameObject", r(owner)),
            ("m_Enabled", u(1)),
            ("m_Shape", i(1)),
            ("m_Extents", xyz(0.5, 1.0, 0.25)),
            ("m_MoveThreshold", f(0.1)),
            ("m_Carve", Value::Bool(true)),
            ("m_CarveOnlyStationary", Value::Bool(false)),
            ("m_Center", xyz(0.0, 2.0, 0.0)),
            ("m_TimeToStationary", f(0.5)),
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

/// Three vertices at `(k, k + 1, k + 2)` with the normal `+y` and the texture coordinates `uvs`,
/// in one stream of 32 bytes each.
fn vertex_data(uvs: [[f32; 2]; 3]) -> Value {
    let mut channels = vec![
        channel(0, 0, 3),
        channel(12, 0, 3),
        channel(0, 0, 0),
        channel(0, 0, 0),
        channel(24, 0, 2),
    ];
    channels.extend((6..12).map(|_| channel(0, 0, 0)));
    let mut bytes = Vec::new();
    for (k, uv) in (0..3).zip(uvs) {
        let position = [k as f32, k as f32 + 1.0, k as f32 + 2.0];
        let floats = position.into_iter().chain([0.0, 1.0, 0.0]);
        bytes.extend(floats.flat_map(f32::to_le_bytes));
        bytes.extend(uv.into_iter().flat_map(f32::to_le_bytes));
    }
    s(vec![
        ("m_VertexCount", u(3)),
        ("m_Channels", list(channels)),
        ("m_DataSize", Value::Bytes(bytes)),
    ])
}

/// A mesh of one submesh with the three vertices `0, 2, 1`.
fn mesh(path_id: i64, name: &str, uvs: [[f32; 2]; 3]) -> Object {
    let indices: Vec<u8> = [0u16, 2, 1].iter().flat_map(|v| v.to_le_bytes()).collect();
    object(
        path_id,
        43,
        "Mesh",
        None,
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
            ("m_VertexData", vertex_data(uvs)),
            (
                "m_CompressedMesh",
                s(vec![("m_Vertices", s(vec![("m_NumItems", u(0))]))]),
            ),
        ]),
    )
}

/// A `MeshRenderer` on the game object `owner` that draws with the material `material`.
fn renderer(path_id: i64, owner: i64, material: i64) -> Object {
    object(
        path_id,
        23,
        "MeshRenderer",
        None,
        s(vec![
            ("m_GameObject", r(owner)),
            ("m_Enabled", u(1)),
            ("m_CastShadows", u(1)),
            ("m_ReceiveShadows", u(1)),
            ("m_Materials", list(vec![r(material)])),
        ]),
    )
}

fn mesh_filter(path_id: i64, owner: i64, mesh: i64) -> Object {
    object(
        path_id,
        33,
        "MeshFilter",
        None,
        s(vec![("m_GameObject", r(owner)), ("m_Mesh", r(mesh))]),
    )
}

/// A material named `name` whose shader is `shader`, with the texture property `_MainTex` bound to
/// the texture `texture`.
fn material(path_id: i64, name: &str, shader: i64, texture: i64) -> Object {
    let env = s(vec![
        ("m_Texture", r(texture)),
        ("m_Scale", s(vec![("x", f(1.0)), ("y", f(1.0))])),
        ("m_Offset", s(vec![("x", f(0.0)), ("y", f(0.0))])),
    ]);
    object(
        path_id,
        21,
        "Material",
        None,
        s(vec![
            ("m_Name", text(name)),
            ("m_Shader", r(shader)),
            ("m_ValidKeywords", list(Vec::new())),
            ("m_InvalidKeywords", list(Vec::new())),
            ("m_CustomRenderQueue", i(-1)),
            (
                "m_SavedProperties",
                s(vec![
                    ("m_TexEnvs", Value::Map(vec![(text("_MainTex"), env)])),
                    ("m_Ints", Value::Map(Vec::new())),
                    ("m_Floats", Value::Map(vec![(text("_Cull"), f(1.0))])),
                    ("m_Colors", Value::Map(Vec::new())),
                ]),
            ),
        ]),
    )
}

/// A shader `name` whose one subshader has one pass: its state names `_Cull` as its culling value
/// and carries the other values.
fn shader(path_id: i64, name: &str) -> Object {
    let fixed = |name: &str, val: f32| s(vec![("val", f(val)), ("name", text(name))]);
    let blend = s(vec![
        ("colMask", fixed("_ColorMask", 15.0)),
        ("srcBlend", fixed("_SrcBlend", 1.0)),
        ("destBlend", fixed("_DstBlend", 0.0)),
        ("srcBlendAlpha", fixed("<noninit>", 0.0)),
        ("destBlendAlpha", fixed("<noninit>", 0.0)),
        ("blendOp", fixed("<noninit>", 0.0)),
        ("blendOpAlpha", fixed("<noninit>", 0.0)),
    ]);
    let state = s(vec![
        ("m_Name", text("Main")),
        (
            "m_Tags",
            s(vec![(
                "tags",
                Value::Map(vec![(text("LightMode"), text("ForwardBase"))]),
            )]),
        ),
        ("culling", fixed("_Cull", 2.0)),
        ("zTest", fixed("_ZTest", 4.0)),
        ("zWrite", fixed("_ZWrite", 1.0)),
        ("rtBlend0", blend),
    ]);
    let property = |name: &str, kind: i64, default: f32, texture: &str| {
        s(vec![
            ("m_Name", text(name)),
            ("m_Type", i(kind)),
            ("m_DefValue[0]", f(default)),
            ("m_DefTexture", s(vec![("m_DefaultName", text(texture))])),
        ])
    };
    object(
        path_id,
        48,
        "Shader",
        None,
        s(vec![
            ("m_Name", text(name)),
            (
                "m_ParsedForm",
                s(vec![
                    (
                        "m_PropInfo",
                        s(vec![(
                            "m_Props",
                            list(vec![
                                property("_MainTex", 4, 0.0, "white"),
                                property("_Cull", 5, 0.0, ""),
                                property("_ZTest", 5, 8.0, ""),
                            ]),
                        )]),
                    ),
                    (
                        "m_SubShaders",
                        list(vec![s(vec![
                            ("m_Passes", list(vec![s(vec![("m_State", state)])])),
                            ("m_Tags", s(vec![("tags", Value::Map(Vec::new()))])),
                        ])]),
                    ),
                    ("m_Name", text(name)),
                ]),
            ),
        ]),
    )
}

/// A 2 x 2 RGBA32 texture. Stored rows run from the bottom, and the red value of a pixel is
/// `10 * row + column` with row 0 at the top.
fn texture(path_id: i64, name: &str) -> Object {
    let mut data = Vec::new();
    for row in (0..2u8).rev() {
        for column in 0..2u8 {
            data.extend([10 * row + column, 1, 2, 3]);
        }
    }
    object(
        path_id,
        28,
        "Texture2D",
        None,
        s(vec![
            ("m_Name", text(name)),
            ("m_Width", i(2)),
            ("m_Height", i(2)),
            ("m_TextureFormat", i(4)),
            ("m_MipCount", i(1)),
            (
                "m_TextureSettings",
                s(vec![
                    ("m_FilterMode", i(1)),
                    ("m_WrapU", i(0)),
                    ("m_WrapV", i(0)),
                ]),
            ),
            ("image data", Value::Bytes(data)),
            (
                "m_StreamData",
                s(vec![("offset", u(0)), ("size", u(0)), ("path", text(""))]),
            ),
        ]),
    )
}

/// The texture coordinates of the meshes: the second vertex has `v` beyond the image, so that a
/// flipped value differs from the stored one.
const UVS: [[f32; 2]; 3] = [[0.25, 0.75], [0.0, 1.0], [1.5, -0.25]];

/// The object ids of the pieces' scene.
const ROOT: i64 = 1;
const ROOT_TRANSFORM: i64 = 2;
const VIEW: i64 = 3;
const BODY: i64 = 10;
const BODY_TRANSFORM: i64 = 11;

/// A piece: the root carries the `FixtureView` and a body that draws a mesh with a material, the
/// body has a mesh renderer and a mesh filter, a box collider and a mesh collider that uses the
/// same mesh as the body, and the root has a navigation obstacle.
fn piece() -> File {
    File(vec![
        game_object(ROOT, "fixture", true, &[ROOT_TRANSFORM, VIEW, 30]),
        transform(ROOT_TRANSFORM, ROOT, [0.0, 0.0, 0.0], &[BODY_TRANSFORM]),
        behaviour(
            VIEW,
            ROOT,
            "FixtureView",
            vec![("_attachPoints", list(Vec::new())), ("_collider", r(0))],
        ),
        obstacle(30, ROOT),
        game_object(BODY, "body", true, &[BODY_TRANSFORM, 12, 13, 14, 15]),
        transform(BODY_TRANSFORM, BODY, [1.0, 0.0, 0.0], &[]),
        renderer(12, BODY, 21),
        mesh_filter(13, BODY, 20),
        box_collider(14, BODY, [0.0, 0.5, 0.0], [1.0, 2.0, 3.0]),
        mesh_collider(15, BODY, 20),
        mesh(20, "body_mesh", UVS),
        material(21, "body_mat", 22, 23),
        shader(22, "Mysekai/Fixture/Basic"),
        texture(23, "body_C"),
    ])
}

/// A GLB file read back: its JSON and its binary chunk.
struct Glb {
    json: Json,
    bin: Vec<u8>,
}

/// A GLB file read back.
fn read_glb(asset: &Asset) -> Glb {
    let member = asset
        .members
        .iter()
        .find(|member| member.role == "model")
        .expect("the asset has a model");
    let Content::File { bytes, extension } = &member.content else {
        panic!("the model is a file")
    };
    assert_eq!(extension, "glb");
    let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    assert_eq!(&bytes[..4], b"glTF");
    let json_length = word(12);
    let json = serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
    let at = 20 + json_length;
    let bin = if at == bytes.len() {
        Vec::new()
    } else {
        assert_eq!(&bytes[at + 4..at + 8], b"BIN\0");
        bytes[at + 8..at + 8 + word(at)].to_vec()
    };
    Glb { json, bin }
}

/// The values of the 32-bit float accessor `accessor`.
fn float_values(glb: &Glb, accessor: usize) -> Vec<f32> {
    let accessor = &glb.json["accessors"][accessor];
    assert_eq!(accessor["componentType"], json!(5126));
    let view = &glb.json["bufferViews"][accessor["bufferView"].as_u64().expect("a view") as usize];
    let start = view["byteOffset"].as_u64().unwrap_or(0) as usize;
    let end = start + view["byteLength"].as_u64().expect("a length") as usize;
    glb.bin[start..end]
        .chunks(4)
        .map(|c| f32::from_le_bytes(c.try_into().expect("four bytes")))
        .collect()
}

/// The model of a piece's asset.
fn model(file: &File) -> Json {
    read_glb(&file.convert().expect("the piece converts")).json
}

/// The extras of the node named `name`.
fn extras(glb: &Json, name: &str) -> Json {
    let nodes = glb["nodes"].as_array().expect("nodes");
    let node = nodes
        .iter()
        .find(|node| node["name"] == name)
        .unwrap_or_else(|| panic!("the node {name} exists"));
    node.get("extras").cloned().unwrap_or(Json::Null)
}

fn object_index(class_id: i32) -> ObjectIndex {
    ObjectIndex {
        path_id: 1,
        class_id,
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
    FixtureModel.is_root(&object_index(1), None, &[path.to_owned()])
}

#[test]
fn a_piece_is_the_prefab_named_after_its_directory() {
    assert!(is_root(
        "assets/sekai/assetbundle/resources/startapp/resources/ondemand/mysekai/fixture/piece_chair/piece_chair.prefab"
    ));
    // Paths are compared without regard to case.
    assert!(is_root(
        "assets/sekai/assetbundle/resources/startapp/resources/ondemand/MySekai/Fixture/Piece/PIECE.prefab"
    ));
    // The file name is not the directory's.
    assert!(!is_root(
        "a/resources/ondemand/mysekai/fixture/piece/other.prefab"
    ));
    // The directory is not the file's.
    assert!(!is_root(
        "a/resources/ondemand/mysekai/fixture/piece/other/piece.prefab"
    ));
    // No name.
    assert!(!is_root("a/resources/ondemand/mysekai/fixture/.prefab"));
    // Not a furniture prefab.
    assert!(!is_root(
        "a/resources/ondemand/mysekai/fixture/fbx/piece/piece.prefab"
    ));
    assert!(!is_root(
        "a/resources/ondemand/mysekai/camera/fieldcamera.prefab"
    ));
    assert!(!FixtureModel.is_root(
        &object_index(28),
        None,
        &[String::from(
            "a/resources/ondemand/mysekai/fixture/piece/piece.prefab"
        )]
    ));
}

#[test]
fn a_root_without_a_fixture_view_is_malformed() {
    let mut file = piece();
    file.object_mut(ROOT).fields = s(vec![
        (
            "m_Component",
            list(vec![s(vec![("component", r(ROOT_TRANSFORM))])]),
        ),
        ("m_Layer", i(0)),
        ("m_Name", text("fixture")),
        ("m_IsActive", Value::Bool(true)),
    ]);
    let error = file.convert().expect_err("the root has no FixtureView");
    assert_eq!(error.code, "malformed");
}

#[test]
fn a_fixture_view_below_the_root_is_malformed() {
    let mut file = piece();
    // The body now carries the behaviour, and the root none.
    file.object_mut(BODY).fields = s(vec![
        (
            "m_Component",
            list(vec![
                s(vec![("component", r(BODY_TRANSFORM))]),
                s(vec![("component", r(VIEW))]),
            ]),
        ),
        ("m_Layer", i(0)),
        ("m_Name", text("body")),
        ("m_IsActive", Value::Bool(true)),
    ]);
    file.object_mut(VIEW).fields = s(vec![("m_GameObject", r(BODY))]);
    let error = file
        .convert()
        .expect_err("the FixtureView is below the root");
    assert_eq!(error.code, "malformed");
}

#[test]
fn every_node_has_the_keys_of_its_source_and_its_hierarchy() {
    let glb = model(&piece());
    assert_eq!(
        extras(&glb, "fixture"),
        json!({
            "fixtureViewRoot": true,
            "sourceObject": { "file": "CAB-test", "gameObjectId": "1", "transformId": "2" },
            "sourceComponentIds": ["2", "3", "30"],
            "sourceChildOrder": ["11"],
            "sourceCollision": {
                "gameObjectId": "1",
                "transformId": "2",
                "activeSelf": true,
                "activeInHierarchy": true,
                "authoredLayer": 0,
                "fixtureNavLayer": true,
                "colliders": [],
                "geometryIds": [],
                "modifiers": [],
                "hasNavMeshAgent": false,
                "navMeshAgents": [],
                "navMeshObstacles": [{
                    "componentId": "30",
                    "enabled": true,
                    "shape": 1,
                    "center": [0.0, 2.0, 0.0],
                    "extents": [0.5, 1.0, 0.25],
                    "carve": true,
                    "onlyStationary": false,
                    "moveThreshold": 0.1_f32,
                    "stationaryTime": 0.5
                }],
            },
            "fixtureCollision": {
                "geometry": [{
                    "geometryId": "CAB-test:20",
                    "positions": [[0.0, 1.0, 2.0], [-1.0, 2.0, 3.0], [-2.0, 3.0, 4.0]],
                    "triangles": [[0, 1, 2]],
                }],
            },
        })
    );
    let body = extras(&glb, "body");
    assert_eq!(
        (
            body.get("sourceObject"),
            body.get("sourceComponentIds"),
            body.get("sourceChildOrder"),
            body.get("shadowCastingMode"),
            body.get("materialed"),
            body.get("fixtureViewRoot"),
            body.get("fixtureCollision")
        ),
        (
            Some(&json!({ "file": "CAB-test", "gameObjectId": "10", "transformId": "11" })),
            Some(&json!(["11", "12", "13", "14", "15"])),
            Some(&json!([])),
            Some(&json!(1)),
            Some(&json!(true)),
            None,
            None,
        )
    );
}

#[test]
fn a_box_collider_and_a_mesh_collider_share_one_geometry() {
    let glb = model(&piece());
    let collision = extras(&glb, "body")["sourceCollision"].clone();
    assert_eq!(collision["colliders"][0]["kind"], json!("BoxCollider"));
    assert_eq!(collision["colliders"][0]["componentId"], json!("14"));
    assert_eq!(collision["colliders"][0]["center"], json!([-0.0, 0.5, 0.0]));
    assert_eq!(collision["colliders"][0]["size"], json!([1.0, 2.0, 3.0]));
    assert_eq!(collision["colliders"][0]["radius"], Json::Null);
    assert_eq!(collision["colliders"][0]["geometryId"], Json::Null);
    assert_eq!(collision["colliders"][0]["excludeLayers"], json!(1023u32));
    assert_eq!(
        collision["colliders"][0]["layerOverridePriority"],
        json!(-1)
    );
    assert_eq!(
        collision["colliders"][1],
        json!({
            "componentId": "15",
            "kind": "MeshCollider",
            "enabled": true,
            "isTrigger": false,
            "includeLayers": 0,
            "excludeLayers": 1023,
            "layerOverridePriority": -1,
            "providesContacts": false,
            "mesh": { "fileId": 0, "pathId": "20" },
            "geometryId": "CAB-test:20",
            "convex": true,
            "cookingOptions": 30,
            "center": null,
            "size": null,
            "radius": null,
            "height": null,
            "direction": null,
        })
    );
    assert_eq!(collision["geometryIds"], json!(["CAB-test:20"]));
}

#[test]
fn an_inactive_game_object_records_whether_it_is_active_in_the_hierarchy() {
    let mut file = piece();
    let fields = &mut file.object_mut(BODY).fields;
    let Value::Struct(fields) = fields else {
        panic!("a game object is a structure")
    };
    fields
        .iter_mut()
        .find(|(name, _)| name == "m_IsActive")
        .expect("the game object stores m_IsActive")
        .1 = Value::Bool(false);
    let glb = model(&file);
    let collision = extras(&glb, "body")["sourceCollision"].clone();
    assert_eq!(collision["activeSelf"], json!(false));
    assert_eq!(collision["activeInHierarchy"], json!(false));
}

#[test]
fn a_material_carries_the_texture_defaults_and_passes_of_its_shader() {
    let glb = model(&piece());
    let unity = &glb["materials"][0]["extras"]["unity"];
    assert_eq!(unity["shader"], json!("Mysekai/Fixture/Basic"));
    assert_eq!(
        unity["shaderTextureDefaults"],
        json!({ "_MainTex": "white" })
    );
    let passes = unity["shaderPasses"].as_array().expect("the passes");
    assert_eq!(passes.len(), 1);
    assert_eq!(passes[0]["name"], json!("Main"));
    assert_eq!(passes[0]["lightMode"], json!("ForwardBase"));
    assert_eq!(
        passes[0]["renderState"]["culling"],
        json!({ "name": "_Cull", "default": 0.0, "val": 2.0 })
    );
    assert_eq!(
        passes[0]["renderState"]["blend"]["srcBlendAlpha"],
        json!({ "name": "<noninit>", "val": 0.0 })
    );
}

#[test]
fn a_material_without_a_shader_has_no_shader_keys() {
    let mut file = piece();
    let fields = &mut file.object_mut(21).fields;
    let Value::Struct(fields) = fields else {
        panic!("a material is a structure")
    };
    fields
        .iter_mut()
        .find(|(name, _)| name == "m_Shader")
        .expect("the material stores m_Shader")
        .1 = r(0);
    let glb = model(&file);
    let unity = &glb["materials"][0]["extras"]["unity"];
    assert_eq!(unity["shader"], Json::Null);
    assert!(unity.get("shaderTextureDefaults").is_none());
    assert!(unity.get("shaderPasses").is_none());
}

#[test]
fn the_model_keeps_its_texture_coordinates_and_names_its_images() {
    let asset = piece().convert().expect("the piece converts");
    let roles: Vec<&str> = asset.members.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, ["model", "texture/0", "attach-points"]);
    let Content::Image(image) = &asset.members[1].content else {
        panic!("the texture is an image")
    };
    assert_eq!((image.width(), image.height()), (2, 2));
    let glb = read_glb(&asset);
    assert_eq!(
        glb.json["images"],
        json!([{ "name": "body_C", "uri": "texture/0.webp" }])
    );
    assert_eq!(glb.json["textures"], json!([{ "sampler": 0, "source": 0 }]));
    assert!(glb.json.get("extensionsUsed").is_none());
    assert!(glb.json.get("extensionsRequired").is_none());
    let uv = glb.json["meshes"][0]["primitives"][0]["attributes"]["TEXCOORD_0"]
        .as_u64()
        .expect("the mesh has texture coordinates") as usize;
    assert_eq!(float_values(&glb, uv), [0.25, 0.75, 0.0, 1.0, 1.5, -0.25]);
}

/// The `attach-points` member of `asset`.
fn attach_points(asset: &Asset) -> Json {
    let member = asset
        .members
        .iter()
        .find(|member| member.role == "attach-points")
        .expect("the asset has attach points");
    let Content::File { bytes, extension } = &member.content else {
        panic!("the attach points are a file")
    };
    assert_eq!(extension, "json");
    serde_json::from_slice(bytes).unwrap()
}

const START: i64 = 40;
const START_TRANSFORM: i64 = 41;
const END: i64 = 42;
const END_TRANSFORM: i64 = 43;
const LONE_START: i64 = 44;
const LONE_START_TRANSFORM: i64 = 45;

fn point(start: i64, end: i64) -> Value {
    s(vec![
        ("StartLoc", r(start)),
        ("EndLoc", r(end)),
        ("IsUsing", Value::Bool(false)),
        ("CharacterId", i(0)),
    ])
}

/// The piece with two attach points under its root, the first with an end locator and the second
/// without, and with the body's mesh collider as the collider of its `FixtureView`.
fn piece_with_points() -> File {
    let mut file = piece();
    file.0.extend([
        game_object(START, "loc_start001", true, &[START_TRANSFORM]),
        transform(START_TRANSFORM, START, [1.0, 0.0, 2.0], &[]),
        game_object(END, "loc_end001", true, &[END_TRANSFORM]),
        transform(END_TRANSFORM, END, [0.5, 0.0, -0.25], &[]),
        game_object(LONE_START, "loc_start002_1", true, &[LONE_START_TRANSFORM]),
        transform(LONE_START_TRANSFORM, LONE_START, [0.0, 0.0, 0.0], &[]),
    ]);
    for locator in [START_TRANSFORM, END_TRANSFORM, LONE_START_TRANSFORM] {
        set(file.object_mut(locator), "m_Father", r(ROOT_TRANSFORM));
    }
    // A quarter turn about y, so that the reflection of the rotation shows.
    let half = std::f32::consts::FRAC_1_SQRT_2;
    set(
        file.object_mut(START_TRANSFORM),
        "m_LocalRotation",
        s(vec![
            ("x", f(0.0)),
            ("y", f(half)),
            ("z", f(0.0)),
            ("w", f(half)),
        ]),
    );
    set(
        file.object_mut(ROOT_TRANSFORM),
        "m_Children",
        list(vec![
            r(BODY_TRANSFORM),
            r(START_TRANSFORM),
            r(END_TRANSFORM),
            r(LONE_START_TRANSFORM),
        ]),
    );
    set(
        file.object_mut(VIEW),
        "_attachPoints",
        list(vec![point(START, END), point(LONE_START, 0)]),
    );
    set(file.object_mut(VIEW), "_collider", r(15));
    file
}

/// Replaces the field `name` of `object`.
fn set(object: &mut Object, name: &str, value: Value) {
    let Value::Struct(fields) = &mut object.fields else {
        panic!("the fields are a structure")
    };
    fields
        .iter_mut()
        .find(|(field, _)| field == name)
        .unwrap_or_else(|| panic!("the field {name} exists"))
        .1 = value;
}

#[test]
fn a_piece_without_attach_points_lists_none() {
    let asset = piece().convert().expect("the piece converts");
    assert_eq!(
        attach_points(&asset),
        json!({
            "root": {
                "sourceObject": { "file": "CAB-test", "gameObjectId": "1", "transformId": "2" },
                "scale": [1.0, 1.0, 1.0],
            },
            "collider": null,
            "points": [],
        })
    );
}

#[test]
fn the_attach_points_are_the_stored_locators_in_the_frame_of_the_file() {
    let half = std::f32::consts::FRAC_1_SQRT_2
        .to_string()
        .parse::<f64>()
        .unwrap();
    let asset = piece_with_points().convert().expect("the piece converts");
    let source = |game_object: i64, transform: i64| {
        json!({
            "file": "CAB-test",
            "gameObjectId": game_object.to_string(),
            "transformId": transform.to_string(),
        })
    };
    assert_eq!(
        attach_points(&asset),
        json!({
            "root": { "sourceObject": source(ROOT, ROOT_TRANSFORM), "scale": [1.0, 1.0, 1.0] },
            "collider": { "sourceObject": source(BODY, BODY_TRANSFORM), "componentId": "15" },
            "points": [
                {
                    "start": {
                        "name": "loc_start001",
                        "sourceObject": source(START, START_TRANSFORM),
                        "position": [-1.0, 0.0, 2.0],
                        "rotation": [0.0, -half, 0.0, half],
                    },
                    "end": {
                        "name": "loc_end001",
                        "sourceObject": source(END, END_TRANSFORM),
                        "position": [-0.5, 0.0, -0.25],
                        "rotation": [0.0, 0.0, 0.0, 1.0],
                    },
                },
                {
                    "start": {
                        "name": "loc_start002_1",
                        "sourceObject": source(LONE_START, LONE_START_TRANSFORM),
                        "position": [-0.0, 0.0, 0.0],
                        "rotation": [0.0, 0.0, 0.0, 1.0],
                    },
                    "end": null,
                },
            ],
        })
    );
}

#[test]
fn each_locator_has_the_pose_of_its_node() {
    let asset = piece_with_points().convert().expect("the piece converts");
    let glb = read_glb(&asset).json;
    let points = attach_points(&asset);
    for (name, locator) in [
        ("loc_start001", &points["points"][0]["start"]),
        ("loc_end001", &points["points"][0]["end"]),
        ("loc_start002_1", &points["points"][1]["start"]),
    ] {
        let node = glb["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["name"] == name)
            .expect("the locator is a node");
        assert_eq!(node["extras"]["sourceObject"], locator["sourceObject"]);
        assert_eq!(node["translation"], locator["position"]);
        assert_eq!(node["rotation"], locator["rotation"]);
    }
}

#[test]
fn a_locator_below_another_node_is_unsupported() {
    let mut file = piece_with_points();
    set(
        file.object_mut(ROOT_TRANSFORM),
        "m_Children",
        list(vec![
            r(BODY_TRANSFORM),
            r(END_TRANSFORM),
            r(LONE_START_TRANSFORM),
        ]),
    );
    set(
        file.object_mut(BODY_TRANSFORM),
        "m_Children",
        list(vec![r(START_TRANSFORM)]),
    );
    set(
        file.object_mut(START_TRANSFORM),
        "m_Father",
        r(BODY_TRANSFORM),
    );
    let error = file.convert().expect_err("the locator is below the body");
    assert_eq!(error.code, "unsupported");
}

#[test]
fn a_point_without_a_start_is_malformed() {
    let mut file = piece_with_points();
    set(
        file.object_mut(VIEW),
        "_attachPoints",
        list(vec![point(0, END)]),
    );
    let error = file.convert().expect_err("the point has no start");
    assert_eq!(error.code, "malformed");
}

#[test]
fn a_collider_that_is_not_a_mesh_collider_is_malformed() {
    let mut file = piece_with_points();
    set(file.object_mut(VIEW), "_collider", r(14));
    let error = file.convert().expect_err("the collider is a box");
    assert_eq!(error.code, "malformed");
}
