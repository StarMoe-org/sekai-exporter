//! The `costume-model` and `costume-motion` forms on objects built in memory.

#![cfg(feature = "costume")]

use sekai_convert::{
    Asset, Chunk, Content, Context, ConvertError, Converter, Object, ObjectId, ObjectIndex, PPtr,
    Value,
};
use sekai_moesekai::costume::{CostumeModel, CostumeMotion};
use sekai_moesekai::roots;
use serde_json::{Value as Json, json};

/// One bundle with one file, holding the given objects.
struct Bundle(Vec<Object>);

impl Context for Bundle {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        self.0
            .iter()
            .find(|o| o.id == id)
            .cloned()
            .ok_or_else(|| ConvertError::new("missing_object", "no such object"))
    }

    fn resolve(&self, _from: ObjectId, reference: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        if reference.path_id == 0 {
            return Ok(None);
        }
        self.object(id(reference.path_id)).map(|o| Some(o.id))
    }

    fn resource(&self, _: ObjectId, path: &str, _: u64, _: u64) -> Result<Chunk<'_>, ConvertError> {
        Err(ConvertError::new("missing_resource", path))
    }

    fn container(&self, _from: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        Ok(Vec::new())
    }
}

impl Bundle {
    /// Changes the field `name` of object `path_id`.
    fn set(&mut self, path_id: i64, name: &str, to: Value) {
        let object = self
            .0
            .iter_mut()
            .find(|o| o.id.path_id == path_id)
            .expect("the object exists");
        set(&mut object.fields, name, to);
    }
}

fn id(path_id: i64) -> ObjectId {
    ObjectId {
        bundle: 0,
        file: 0,
        path_id,
    }
}

fn s(fields: Vec<(&str, Value)>) -> Value {
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

fn r(path_id: i64) -> Value {
    Value::PPtr(PPtr {
        file_id: 0,
        path_id,
    })
}

fn xyz(x: f32, y: f32, z: f32) -> Value {
    s(vec![("x", f(x)), ("y", f(y)), ("z", f(z))])
}

fn set(value: &mut Value, name: &str, to: Value) {
    let Value::Struct(fields) = value else {
        panic!("not a structure")
    };
    fields
        .iter_mut()
        .find(|(n, _)| n == name)
        .expect("the field exists")
        .1 = to;
}

fn add(value: &mut Value, name: &str, to: Value) {
    let Value::Struct(fields) = value else {
        panic!("not a structure")
    };
    fields.push((name.into(), to));
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

fn game_object(path_id: i64, name: &str, components: &[i64]) -> Object {
    let components = components
        .iter()
        .map(|&c| s(vec![("component", r(c))]))
        .collect();
    object(
        path_id,
        1,
        "GameObject",
        None,
        s(vec![
            ("m_Component", list(components)),
            ("m_Name", text(name)),
            ("m_IsActive", Value::Bool(true)),
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
                    ("y", f(0.6)),
                    ("z", f(0.0)),
                    ("w", f(0.8)),
                ]),
            ),
            ("m_LocalPosition", xyz(x, y, z)),
            ("m_LocalScale", xyz(1.0, 1.0, 1.0)),
            ("m_Children", list(children.iter().map(|&c| r(c)).collect())),
            ("m_Father", r(0)),
        ]),
    )
}

fn behaviour(
    path_id: i64,
    owner: i64,
    script: &str,
    enabled: u64,
    fields: Vec<(&str, Value)>,
) -> Object {
    let mut all = vec![
        ("m_GameObject", r(owner)),
        ("m_Enabled", u(enabled)),
        ("m_Script", r(0)),
        ("m_Name", text("")),
    ];
    all.extend(fields);
    object(path_id, 114, "MonoBehaviour", Some(script), s(all))
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

/// A stored vertex: position, normal and tangent; byte colour; texture coordinates and two bone
/// weights; two bone slots.
type StoredVertex = ([f32; 10], [u8; 4], [f32; 4], [u32; 2]);

/// Three vertices in one stream of 68 bytes each, weighing on the bone slots of `bones`.
fn vertex_data(bones: [[u32; 2]; 3]) -> Value {
    let mut channels = vec![
        channel(0, 0, 3),
        channel(12, 0, 3),
        channel(24, 0, 4),
        channel(40, 2, 4),
        channel(44, 0, 2),
    ];
    channels.extend((5..12).map(|_| channel(0, 0, 0)));
    channels.push(channel(52, 0, 2));
    channels.push(channel(60, 10, 2));
    let [first, second, third] = bones;
    let vertices: [StoredVertex; 3] = [
        (
            [1.0, 2.0, 3.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0],
            [10, 20, 30, 40],
            [0.25, 0.75, 1.0, 0.0],
            first,
        ),
        (
            [4.0, 5.0, 6.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, -1.0],
            [1, 2, 3, 4],
            [0.0, 0.0, 0.5, 0.5],
            second,
        ),
        (
            [7.0, 8.0, 9.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0],
            [5, 6, 7, 8],
            [1.0, 1.0, 1.0, 0.0],
            third,
        ),
    ];
    let mut bytes = Vec::new();
    for (floats, color, rest, indices) in vertices {
        bytes.extend(floats.iter().flat_map(|v| v.to_le_bytes()));
        bytes.extend(color);
        bytes.extend(rest.iter().flat_map(|v| v.to_le_bytes()));
        bytes.extend(indices.iter().flat_map(|v| v.to_le_bytes()));
    }
    s(vec![
        ("m_VertexCount", u(3)),
        ("m_Channels", list(channels)),
        ("m_DataSize", Value::Bytes(bytes)),
    ])
}

/// A bind pose that moves by `x` along X.
fn bind_pose(x: f32) -> Value {
    let mut entries = Vec::new();
    for row in 0..4 {
        for column in 0..4 {
            let value = match (row, column) {
                (0, 3) => x,
                (r, c) if r == c => 1.0,
                _ => 0.0,
            };
            entries.push((format!("e{row}{column}"), f(value)));
        }
    }
    Value::Struct(entries)
}

fn mesh() -> Object {
    let indices: Vec<u8> = [0u16, 1, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
    object(
        30,
        43,
        "Mesh",
        None,
        s(vec![
            ("m_Name", text("body_mesh")),
            (
                "m_SubMeshes",
                list(vec![s(vec![
                    ("firstByte", u(0)),
                    ("indexCount", u(3)),
                    ("topology", i(0)),
                    ("baseVertex", u(0)),
                ])]),
            ),
            (
                "m_Shapes",
                s(vec![
                    (
                        "vertices",
                        list(vec![s(vec![
                            ("vertex", xyz(1.0, 0.0, 0.0)),
                            ("normal", xyz(0.0, 1.0, 0.0)),
                            ("tangent", xyz(5.0, 5.0, 5.0)),
                            ("index", u(2)),
                        ])]),
                    ),
                    (
                        "shapes",
                        list(vec![s(vec![
                            ("firstVertex", u(0)),
                            ("vertexCount", u(1)),
                            ("hasNormals", Value::Bool(true)),
                            ("hasTangents", Value::Bool(false)),
                        ])]),
                    ),
                    (
                        "channels",
                        list(vec![s(vec![
                            ("name", text("eye_close")),
                            ("nameHash", u(0)),
                            ("frameIndex", i(0)),
                            ("frameCount", i(1)),
                        ])]),
                    ),
                    ("fullWeights", list(vec![f(100.0)])),
                ]),
            ),
            ("m_BindPose", list(vec![bind_pose(2.0), bind_pose(0.0)])),
            (
                "m_VariableBoneCountWeights",
                s(vec![("m_Data", list(Vec::new()))]),
            ),
            ("m_IndexFormat", i(0)),
            ("m_IndexBuffer", Value::Bytes(indices)),
            ("m_VertexData", vertex_data([[0, 1], [0, 1], [1, 0]])),
            (
                "m_CompressedMesh",
                s(vec![("m_Vertices", s(vec![("m_NumItems", u(0))]))]),
            ),
        ]),
    )
}

fn material() -> Object {
    let env = |texture, scale_x| {
        s(vec![
            ("m_Texture", r(texture)),
            ("m_Scale", s(vec![("x", f(scale_x)), ("y", f(1.0))])),
            ("m_Offset", s(vec![("x", f(0.0)), ("y", f(0.0))])),
        ])
    };
    object(
        31,
        21,
        "Material",
        None,
        s(vec![
            ("m_Name", text("body_mat")),
            ("m_Shader", r(32)),
            ("m_ValidKeywords", list(vec![text("_OUTLINE")])),
            ("m_InvalidKeywords", list(Vec::new())),
            ("m_CustomRenderQueue", i(-1)),
            (
                "m_SavedProperties",
                s(vec![
                    (
                        "m_TexEnvs",
                        Value::Map(vec![
                            (text("_MainTex"), env(33, 2.0)),
                            (text("_ShadowTex"), env(0, 1.0)),
                            (text("_MainTex"), env(0, 1.0)),
                        ]),
                    ),
                    ("m_Ints", Value::Map(Vec::new())),
                    (
                        "m_Floats",
                        Value::Map(vec![
                            (text("_Outline"), f(0.5)),
                            (text("_Bad"), f(f32::NAN)),
                        ]),
                    ),
                    (
                        "m_Colors",
                        Value::Map(vec![(
                            text("_Color"),
                            s(vec![
                                ("r", f(1.0)),
                                ("g", f(0.5)),
                                ("b", f(0.0)),
                                ("a", f(1.0)),
                            ]),
                        )]),
                    ),
                ]),
            ),
        ]),
    )
}

fn texture() -> Object {
    object(
        33,
        28,
        "Texture2D",
        None,
        s(vec![
            ("m_Name", text("body_C")),
            ("m_Width", i(2)),
            ("m_Height", i(2)),
            ("m_TextureFormat", i(4)),
            ("m_MipCount", i(1)),
            (
                "m_TextureSettings",
                s(vec![
                    ("m_FilterMode", i(1)),
                    ("m_WrapU", i(1)),
                    ("m_WrapV", i(0)),
                ]),
            ),
            ("image data", Value::Bytes((0..16).collect())),
            (
                "m_StreamData",
                s(vec![("offset", u(0)), ("size", u(0)), ("path", text(""))]),
            ),
        ]),
    )
}

/// A body: `body` › `Position` › (`Hip`, `Mesh`), with a skinned mesh bound to `Hip` and the
/// root, a spring manager, a spring bone, an aim constraint and a script the form does not list.
fn part() -> Bundle {
    Bundle(vec![
        game_object(1, "body", &[2, 20, 21]),
        transform(2, 1, [0.0; 3], &[4]),
        game_object(3, "Position", &[4, 22]),
        transform(4, 3, [1.0, 2.0, 3.0], &[6, 8]),
        game_object(5, "Hip", &[6, 23]),
        transform(6, 5, [0.0, 1.0, 0.0], &[]),
        game_object(7, "Mesh", &[8, 9]),
        transform(8, 7, [0.0; 3], &[]),
        object(
            9,
            137,
            "SkinnedMeshRenderer",
            None,
            s(vec![
                ("m_GameObject", r(7)),
                ("m_Enabled", Value::Bool(false)),
                ("m_Materials", list(vec![r(31), r(31)])),
                ("m_Mesh", r(30)),
                ("m_Bones", list(vec![r(6), r(2)])),
                ("m_BlendShapeWeights", list(vec![f(50.0)])),
                ("m_RootBone", r(6)),
            ]),
        ),
        behaviour(
            20,
            1,
            "ThirdParty.SpringBone/UTJ.SpringManager",
            1,
            vec![
                ("springBones", list(vec![r(23)])),
                ("gravity", xyz(0.0, -9.8, 0.0)),
            ],
        ),
        behaviour(
            21,
            1,
            "Unity.RenderPipelines.Universal.Runtime/Sekai.Rendering.SekaiCharacterHair",
            1,
            vec![("headTransform", r(31))],
        ),
        object(
            22,
            2011,
            "AimConstraint",
            None,
            s(vec![
                ("m_GameObject", r(3)),
                ("m_Enabled", u(0)),
                ("m_WorldUpObject", r(0)),
                (
                    "m_Sources",
                    list(vec![s(vec![("sourceTransform", r(1)), ("weight", f(1.0))])]),
                ),
            ]),
        ),
        behaviour(
            23,
            5,
            "Assembly-CSharp/Sekai.SekaiSpringBone",
            1,
            vec![
                ("pivotNode", r(6)),
                ("radius", f(0.02)),
                ("renderer", r(9)),
                ("yAngleLimits", s(vec![("active", u(1)), ("min", f(-20.0))])),
            ],
        ),
        mesh(),
        material(),
        object(
            32,
            48,
            "Shader",
            None,
            s(vec![(
                "m_ParsedForm",
                s(vec![("m_Name", text("Sekai/Character/Toon"))]),
            )]),
        ),
        texture(),
    ])
}

/// The part with its mesh changed by `change`.
fn part_with_mesh(change: impl FnOnce(&mut Value)) -> Bundle {
    let mut bundle = part();
    let mesh = bundle
        .0
        .iter_mut()
        .find(|o| o.id.path_id == 30)
        .expect("the part has a mesh");
    change(&mut mesh.fields);
    bundle
}

/// The CRC-32 of the paths `Position/Hip`, `` and `Nowhere`.
const HIP: u64 = 3_796_555_716;
const ROOT: u64 = 0;
const NOWHERE: u64 = 2_874_607_758;

fn bone_hashes(hashes: &[u64]) -> Value {
    list(hashes.iter().map(|&h| u(h)).collect())
}

fn convert(converter: &dyn Converter, root: i64, bundle: &Bundle) -> Result<Asset, ConvertError> {
    Ok(converter.convert(id(root), bundle)?.expect("an asset"))
}

fn code(result: Result<Asset, ConvertError>) -> &'static str {
    result.expect_err("an error").code
}

fn file(asset: &Asset, role: &str) -> Vec<u8> {
    let member = asset
        .members
        .iter()
        .find(|m| m.role == role)
        .expect("the member exists");
    let Content::File { bytes, .. } = &member.content else {
        panic!("not a file")
    };
    bytes.clone()
}

/// A GLB file read back: its JSON and its binary chunk.
struct Glb {
    json: Json,
    bin: Vec<u8>,
}

impl Glb {
    fn read(bytes: &[u8]) -> Self {
        let word = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(word(4), 2);
        assert_eq!(word(8), bytes.len());
        let json_length = word(12);
        assert_eq!(&bytes[16..20], b"JSON");
        assert_eq!(json_length % 4, 0);
        let json = serde_json::from_slice(&bytes[20..20 + json_length]).unwrap();
        let at = 20 + json_length;
        let bin = if at == bytes.len() {
            Vec::new()
        } else {
            assert_eq!(&bytes[at + 4..at + 8], b"BIN\0");
            bytes[at + 8..at + 8 + word(at)].to_vec()
        };
        Self { json, bin }
    }

    /// The values of an accessor, as numbers, with sparse elements in place.
    fn values(&self, accessor: usize) -> Vec<f64> {
        let a = &self.json["accessors"][accessor];
        let width = match a["type"].as_str().unwrap() {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            _ => 16,
        };
        let count = a["count"].as_u64().unwrap() as usize;
        let component = a["componentType"].as_u64().unwrap();
        let mut out = match a.get("bufferView") {
            Some(view) => self.view(view.as_u64().unwrap() as usize, component),
            None => vec![0.0; count * width],
        };
        if let Some(sparse) = a.get("sparse") {
            let indices = self.view(
                sparse["indices"]["bufferView"].as_u64().unwrap() as usize,
                sparse["indices"]["componentType"].as_u64().unwrap(),
            );
            let values = self.view(
                sparse["values"]["bufferView"].as_u64().unwrap() as usize,
                component,
            );
            for (k, index) in indices.iter().enumerate() {
                let index = *index as usize;
                out[index * width..][..width].copy_from_slice(&values[k * width..][..width]);
            }
        }
        assert_eq!(out.len(), count * width);
        out
    }

    fn view(&self, view: usize, component: u64) -> Vec<f64> {
        let v = &self.json["bufferViews"][view];
        let start = v["byteOffset"].as_u64().unwrap() as usize;
        let bytes = &self.bin[start..start + v["byteLength"].as_u64().unwrap() as usize];
        match component {
            5121 => bytes.iter().map(|&b| f64::from(b)).collect(),
            5123 => bytes
                .chunks(2)
                .map(|c| f64::from(u16::from_le_bytes([c[0], c[1]])))
                .collect(),
            5125 => bytes
                .chunks(4)
                .map(|c| f64::from(u32::from_le_bytes(c.try_into().unwrap())))
                .collect(),
            5126 => bytes
                .chunks(4)
                .map(|c| f64::from(f32::from_le_bytes(c.try_into().unwrap())))
                .collect(),
            other => panic!("component type {other}"),
        }
    }

    fn attribute(&self, mesh: usize, name: &str) -> Vec<f64> {
        let accessor = &self.json["meshes"][mesh]["primitives"][0]["attributes"][name];
        self.values(accessor.as_u64().unwrap() as usize)
    }
}

#[test]
fn a_part_has_its_model_then_its_components_then_its_textures() {
    let asset = convert(&CostumeModel, 1, &part()).unwrap();
    let roles: Vec<&str> = asset.members.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, ["model", "components", "texture/0"]);
    let Content::Image(image) = &asset.members[2].content else {
        panic!("the texture is an image")
    };
    assert_eq!((image.width(), image.height()), (2, 2));
}

#[test]
fn nodes_follow_the_hierarchy_in_gltf_coordinates() {
    let glb = Glb::read(&file(&convert(&CostumeModel, 1, &part()).unwrap(), "model"));
    let nodes = &glb.json["nodes"];
    let names: Vec<&str> = nodes
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["body", "Position", "Hip", "Mesh"]);
    assert_eq!(glb.json["scenes"], json!([{ "nodes": [0] }]));
    assert_eq!(nodes[0]["children"], json!([1]));
    assert_eq!(nodes[1]["children"], json!([2, 3]));
    assert_eq!(nodes[1]["translation"], json!([-1.0, 2.0, 3.0]));
    assert_eq!(nodes[1]["rotation"], json!([0.0, -0.6, -0.0, 0.8]));
    assert_eq!(nodes[3]["extras"], json!({ "enabled": false }));
    assert_eq!(nodes[3]["weights"], json!([0.5]));
    assert_eq!(nodes[3]["mesh"], json!(0));
    assert_eq!(nodes[3]["skin"], json!(0));
    assert_eq!(glb.json["skins"][0]["joints"], json!([2, 0]));
    assert_eq!(glb.json["skins"][0]["extras"], json!({ "rootBone": 2 }));
}

#[test]
fn vertices_are_mirrored_and_triangles_reversed() {
    let glb = Glb::read(&file(&convert(&CostumeModel, 1, &part()).unwrap(), "model"));
    assert_eq!(
        glb.attribute(0, "POSITION"),
        [-1.0, 2.0, 3.0, -4.0, 5.0, 6.0, -7.0, 8.0, 9.0]
    );
    let position = &glb.json["accessors"]
        [glb.json["meshes"][0]["primitives"][0]["attributes"]["POSITION"]
            .as_u64()
            .unwrap() as usize];
    assert_eq!(position["min"], json!([-7.0, 2.0, 3.0]));
    assert_eq!(position["max"], json!([-1.0, 8.0, 9.0]));
    assert_eq!(
        glb.attribute(0, "TANGENT"),
        [
            -1.0, 0.0, 0.0, -1.0, -0.0, 1.0, 0.0, 1.0, -0.0, 0.0, 1.0, -1.0
        ]
    );
    assert_eq!(
        glb.attribute(0, "TEXCOORD_0"),
        [0.25, 0.25, 0.0, 1.0, 1.0, 0.0]
    );
    assert_eq!(
        glb.attribute(0, "COLOR_0"),
        [
            10.0, 20.0, 30.0, 40.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0
        ]
    );
    // a joint whose weight is 0 is written as 0
    assert_eq!(
        glb.attribute(0, "JOINTS_0"),
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]
    );
    assert_eq!(
        glb.attribute(0, "WEIGHTS_0"),
        [1.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]
    );
    let primitive = &glb.json["meshes"][0]["primitives"][0];
    assert_eq!(
        glb.values(primitive["indices"].as_u64().unwrap() as usize),
        [0.0, 2.0, 1.0]
    );
    assert_eq!(
        glb.json["accessors"][primitive["indices"].as_u64().unwrap() as usize]["componentType"],
        json!(5123)
    );
    // the bind pose moved by 2 along X moves by -2 in glTF
    let matrices = glb.values(
        glb.json["skins"][0]["inverseBindMatrices"]
            .as_u64()
            .unwrap() as usize,
    );
    assert_eq!(matrices[12], -2.0);
    assert_eq!(matrices[28], 0.0);
}

/// The joints of the first skin and the joint of each vertex's bones.
fn skin_joints(bundle: &Bundle) -> (Json, Vec<f64>) {
    let glb = Glb::read(&file(&convert(&CostumeModel, 1, bundle).unwrap(), "model"));
    (
        glb.json["skins"][0]["joints"].clone(),
        glb.attribute(0, "JOINTS_0"),
    )
}

#[test]
fn bones_bind_by_their_path_hash_before_the_stored_bones() {
    // the stored bones are in the other order; the hashes name Hip, then the root
    let mut bundle =
        part_with_mesh(|mesh| add(mesh, "m_BoneNameHashes", bone_hashes(&[HIP, ROOT])));
    bundle.set(9, "m_Bones", list(vec![r(2), r(6)]));
    let (joints, _) = skin_joints(&bundle);
    assert_eq!(joints, json!([2, 0]));
    // a hash that names no node falls back to the stored bone of its slot
    let bundle =
        part_with_mesh(|mesh| add(mesh, "m_BoneNameHashes", bone_hashes(&[NOWHERE, ROOT])));
    let (joints, _) = skin_joints(&bundle);
    assert_eq!(joints, json!([2, 0]));
    // a stored bone that a hash already binds is passed over, and the slot left without a
    // node, which no vertex weighs on, has no joint
    let mut bundle = part_with_mesh(|mesh| {
        add(mesh, "m_BoneNameHashes", bone_hashes(&[HIP, NOWHERE, ROOT]));
        set(
            mesh,
            "m_BindPose",
            list(vec![bind_pose(2.0), bind_pose(1.0), bind_pose(0.0)]),
        );
        set(mesh, "m_VertexData", vertex_data([[0, 2], [0, 2], [2, 0]]));
    });
    bundle.set(9, "m_Bones", list(vec![r(6), r(6), r(2)]));
    let (joints, vertex_joints) = skin_joints(&bundle);
    assert_eq!(joints, json!([2, 0]));
    assert_eq!(
        vertex_joints,
        [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0]
    );
    let glb = Glb::read(&file(&convert(&CostumeModel, 1, &bundle).unwrap(), "model"));
    let matrices = glb.values(
        glb.json["skins"][0]["inverseBindMatrices"]
            .as_u64()
            .unwrap() as usize,
    );
    assert_eq!(
        (matrices.len(), matrices[12], matrices[28]),
        (32, -2.0, 0.0)
    );
}

#[test]
fn a_material_beyond_the_submeshes_draws_the_last_one_again() {
    let glb = Glb::read(&file(&convert(&CostumeModel, 1, &part()).unwrap(), "model"));
    let primitives = glb.json["meshes"][0]["primitives"].as_array().unwrap();
    assert_eq!(primitives.len(), 2);
    assert_eq!(primitives[0]["indices"], primitives[1]["indices"]);
    assert_eq!(primitives[1]["material"], json!(0));
}

#[test]
fn blend_shapes_are_sparse_morph_targets_named_by_channel() {
    let glb = Glb::read(&file(&convert(&CostumeModel, 1, &part()).unwrap(), "model"));
    let mesh = &glb.json["meshes"][0];
    assert_eq!(
        mesh["extras"],
        json!({ "targetNames": ["eye_close"], "fullWeights": [100.0] })
    );
    let target = &mesh["primitives"][0]["targets"][0];
    let at = |name: &str| glb.values(target[name].as_u64().unwrap() as usize);
    assert_eq!(
        at("POSITION"),
        [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0]
    );
    assert_eq!(at("NORMAL"), [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -0.0, 1.0, 0.0]);
    // the frame stores no tangents
    assert_eq!(at("TANGENT"), [0.0; 9]);
    let position = &glb.json["accessors"][target["POSITION"].as_u64().unwrap() as usize];
    assert_eq!(position["min"], json!([-1.0, 0.0, 0.0]));
    assert_eq!(position["max"], json!([0.0, 0.0, 0.0]));
}

#[test]
fn materials_keep_their_stored_properties_and_images_are_members() {
    let glb = Glb::read(&file(&convert(&CostumeModel, 1, &part()).unwrap(), "model"));
    let material = &glb.json["materials"][0];
    assert_eq!(material["name"], json!("body_mat"));
    assert_eq!(
        material["pbrMetallicRoughness"],
        json!({
            "baseColorTexture": {
                "index": 0,
                "extensions": { "KHR_texture_transform": { "offset": [0.0, 0.0], "scale": [2.0, 1.0] } }
            },
            "metallicFactor": 0.0,
            "roughnessFactor": 1.0
        })
    );
    assert_eq!(
        material["extras"]["unity"],
        json!({
            "shader": "Sekai/Character/Toon",
            "renderQueue": -1,
            "keywords": { "valid": ["_OUTLINE"], "invalid": [] },
            "textures": {
                "_MainTex": { "texture": 0, "scale": [2.0, 1.0], "offset": [0.0, 0.0] },
                "_ShadowTex": { "texture": null, "scale": [1.0, 1.0], "offset": [0.0, 0.0] }
            },
            "floats": { "_Outline": 0.5, "_Bad": "NaN" },
            "ints": {},
            "colors": { "_Color": [1.0, 0.5, 0.0, 1.0] }
        })
    );
    assert_eq!(
        glb.json["textures"],
        json!([{ "sampler": 0, "extensions": { "EXT_texture_webp": { "source": 0 } } }])
    );
    assert_eq!(
        glb.json["samplers"],
        json!([{ "magFilter": 9729, "minFilter": 9729, "wrapS": 33071, "wrapT": 10497 }])
    );
    assert_eq!(
        glb.json["images"],
        json!([{ "name": "body_C", "uri": "texture/0.webp" }])
    );
    assert_eq!(glb.json["extensionsRequired"], json!(["EXT_texture_webp"]));
    assert_eq!(
        glb.json["extensionsUsed"],
        json!(["EXT_texture_webp", "KHR_texture_transform"])
    );
}

#[test]
fn components_refer_to_nodes_and_to_each_other() {
    let asset = convert(&CostumeModel, 1, &part()).unwrap();
    let doc: Json = serde_json::from_slice(&file(&asset, "components")).unwrap();
    assert_eq!(
        doc,
        json!({
            "format": "components",
            "version": 1,
            "components": [
                {
                    "class": "MonoBehaviour",
                    "script": "ThirdParty.SpringBone/UTJ.SpringManager",
                    "node": "",
                    "enabled": true,
                    "fields": {
                        "springBones": [{ "$ref": { "component": 2 } }],
                        "gravity": { "x": 0.0, "y": -9.8, "z": 0.0 }
                    }
                },
                {
                    "class": "AimConstraint",
                    "script": null,
                    "node": "Position",
                    "enabled": false,
                    "fields": {
                        "m_WorldUpObject": null,
                        "m_Sources": [{ "sourceTransform": { "$ref": { "node": "" } }, "weight": 1.0 }]
                    }
                },
                {
                    "class": "MonoBehaviour",
                    "script": "Assembly-CSharp/Sekai.SekaiSpringBone",
                    "node": "Position/Hip",
                    "enabled": true,
                    "fields": {
                        "pivotNode": { "$ref": { "node": "Position/Hip" } },
                        "radius": 0.02,
                        "renderer": { "$ref": { "node": "Position/Mesh", "class": "SkinnedMeshRenderer" } },
                        "yAngleLimits": { "active": 1, "min": -20.0 }
                    }
                }
            ]
        })
    );
}

#[test]
fn a_part_without_listed_components_has_no_component_document() {
    let mut bundle = part();
    bundle.set(1, "m_Component", list(vec![s(vec![("component", r(2))])]));
    bundle.set(3, "m_Component", list(vec![s(vec![("component", r(4))])]));
    bundle.set(5, "m_Component", list(vec![s(vec![("component", r(6))])]));
    let asset = convert(&CostumeModel, 1, &bundle).unwrap();
    let roles: Vec<&str> = asset.members.iter().map(|m| m.role.as_str()).collect();
    assert_eq!(roles, ["model", "texture/0"]);
}

#[test]
fn parts_the_documents_do_not_define_are_unsupported() {
    // a reference out of the prefab from a listed component
    let mut bundle = part();
    bundle.set(23, "pivotNode", r(31));
    assert_eq!(code(convert(&CostumeModel, 1, &bundle)), "unsupported");
    // a bone that is not a node of the model
    let mut bundle = part();
    bundle.set(9, "m_Bones", list(vec![r(6), r(31)]));
    assert_eq!(code(convert(&CostumeModel, 1, &bundle)), "unsupported");
    // a vertex weighs on a bone slot that binds no node by hash or by `m_Bones`
    let mut bundle =
        part_with_mesh(|mesh| add(mesh, "m_BoneNameHashes", bone_hashes(&[NOWHERE, ROOT])));
    bundle.set(9, "m_Bones", list(vec![r(31), r(2)]));
    assert_eq!(code(convert(&CostumeModel, 1, &bundle)), "unsupported");
    // a blend shape channel with two frames
    let bundle = part_with_mesh(|mesh| {
        let Value::Struct(fields) = mesh else {
            unreachable!()
        };
        let shapes = &mut fields.iter_mut().find(|(n, _)| n == "m_Shapes").unwrap().1;
        set(
            shapes,
            "channels",
            list(vec![s(vec![
                ("name", text("eye_close")),
                ("nameHash", u(0)),
                ("frameIndex", i(0)),
                ("frameCount", i(2)),
            ])]),
        );
    });
    assert_eq!(code(convert(&CostumeModel, 1, &bundle)), "unsupported");
}

#[test]
fn damaged_parts_are_malformed() {
    // one bind pose for two bones, and no bone name hashes
    let bundle = part_with_mesh(|mesh| set(mesh, "m_BindPose", list(vec![bind_pose(0.0)])));
    assert_eq!(code(convert(&CostumeModel, 1, &bundle)), "malformed");
    // a vertex weighs on a bone slot beyond the bind poses
    let mut bundle = part_with_mesh(|mesh| set(mesh, "m_BindPose", list(vec![bind_pose(0.0)])));
    bundle.set(9, "m_Bones", list(vec![r(6)]));
    assert_eq!(code(convert(&CostumeModel, 1, &bundle)), "malformed");
    // a game object below itself
    let mut bundle = part();
    bundle.set(6, "m_Children", list(vec![r(4)]));
    assert_eq!(code(convert(&CostumeModel, 1, &bundle)), "malformed");
}

#[test]
fn the_same_part_gives_the_same_bytes() {
    let a = convert(&CostumeModel, 1, &part()).unwrap();
    let b = convert(&CostumeModel, 1, &part()).unwrap();
    assert_eq!(file(&a, "model"), file(&b, "model"));
    assert_eq!(file(&a, "components"), file(&b, "components"));
}

fn binding(path: u64, attribute: u64, type_id: i64, custom_type: i64) -> Value {
    s(vec![
        ("path", u(path)),
        ("attribute", u(attribute)),
        ("script", r(0)),
        ("typeID", i(type_id)),
        ("customType", i(custom_type)),
        ("isPPtrCurve", u(0)),
        ("isIntCurve", u(0)),
        ("isSerializeReferenceCurve", u(0)),
    ])
}

/// A clip that drives the position and rotation of one node and one blend shape, with constant
/// curves `0, 1, …`.
fn clip(bindings: Vec<Value>, constants: usize) -> Bundle {
    let empty = || list(Vec::new());
    Bundle(vec![object(
        40,
        74,
        "AnimationClip",
        None,
        s(vec![
            ("m_Name", text("motion_loop")),
            ("m_Legacy", Value::Bool(false)),
            ("m_Compressed", Value::Bool(false)),
            ("m_SampleRate", f(60.0)),
            (
                "m_MuscleClip",
                s(vec![
                    (
                        "m_Clip",
                        s(vec![(
                            "data",
                            s(vec![
                                (
                                    "m_StreamedClip",
                                    s(vec![("data", empty()), ("curveCount", u(0))]),
                                ),
                                (
                                    "m_DenseClip",
                                    s(vec![("m_FrameCount", i(0)), ("m_CurveCount", u(0))]),
                                ),
                                (
                                    "m_ConstantClip",
                                    s(vec![(
                                        "data",
                                        list((0..constants).map(|k| f(k as f32)).collect()),
                                    )]),
                                ),
                            ]),
                        )]),
                    ),
                    ("m_StartTime", f(0.0)),
                    ("m_StopTime", f(2.0)),
                    ("m_LoopTime", Value::Bool(true)),
                ]),
            ),
            (
                "m_ClipBindingConstant",
                s(vec![
                    ("genericBindings", list(bindings)),
                    ("pptrCurveMapping", empty()),
                ]),
            ),
        ]),
    )])
}

fn transform_and_shape() -> Vec<Value> {
    vec![
        binding(100, 1, 4, 0),
        binding(100, 2, 4, 0),
        binding(200, 3_000_000_000, 137, 20),
    ]
}

#[test]
fn a_motion_takes_as_many_curves_as_each_property_has_components() {
    let asset = convert(&CostumeMotion, 40, &clip(transform_and_shape(), 8)).unwrap();
    assert_eq!(asset.members.len(), 1);
    assert_eq!(asset.members[0].role, "motion");
    let doc: Json = serde_json::from_slice(&file(&asset, "motion")).unwrap();
    assert_eq!(doc["version"], json!(2));
    let curves = doc["curves"].as_array().unwrap();
    let summary: Vec<(u64, u64, u64, f64)> = curves
        .iter()
        .map(|c| {
            (
                c["path"].as_u64().unwrap(),
                c["attribute"].as_u64().unwrap(),
                c["component"].as_u64().unwrap(),
                c["value"].as_f64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            (100, 1, 0, 0.0),
            (100, 1, 1, 1.0),
            (100, 1, 2, 2.0),
            (100, 2, 0, 3.0),
            (100, 2, 1, 4.0),
            (100, 2, 2, 5.0),
            (100, 2, 3, 6.0),
            (200, 3_000_000_000, 0, 7.0),
        ]
    );
    assert_eq!(curves[7]["class_id"], json!(137));
    assert_eq!(curves[7]["custom_type"], json!(20));
}

#[test]
fn motions_the_documents_do_not_define_are_unsupported_or_malformed() {
    // one curve fewer than the bindings take
    assert_eq!(
        code(convert(&CostumeMotion, 40, &clip(transform_and_shape(), 7))),
        "malformed"
    );
    // a Transform property outside the four the document defines
    assert_eq!(
        code(convert(
            &CostumeMotion,
            40,
            &clip(vec![binding(100, 5, 4, 0)], 3)
        )),
        "unsupported"
    );
}

fn index(class_id: i32) -> ObjectIndex {
    ObjectIndex {
        path_id: 1,
        class_id,
        class: String::new(),
        name: None,
        script: None,
        script_name: None,
        references: Vec::new(),
        size: 0,
        sha256: [0; 32],
        error: None,
    }
}

#[test]
fn roots_are_the_part_prefabs_and_the_costume_screen_motions() {
    let part = |path: &str| roots::costume_model(&index(1), None, &[path.to_owned()]);
    let base = "assets/sekai/assetbundle/resources/ondemand/live_pv/model";
    assert!(part(&format!("{base}/character/body/21/0001/body.prefab")));
    assert!(part(&format!(
        "{base}/characterv2/face/01/0001/Face.prefab"
    )));
    assert!(part(&format!(
        "{base}/character/head_optional/1/optional.prefab"
    )));
    assert!(!part(&format!(
        "{base}/character/body/21/0001/sit_body.prefab"
    )));
    assert!(!part(&format!(
        "{base}/character/body/21/0001/mdl_body.fbx"
    )));
    assert!(!roots::costume_model(
        &index(4),
        None,
        &[format!("{base}/character/body/21/0001/body.prefab")]
    ));
    let motion = |path: &str| roots::costume_motion(&index(74), None, &[path.to_owned()]);
    let motions = "assets/sekai/assetbundle/resources/ondemand/live_pv/outgame/character/motion";
    assert!(motion(&format!(
        "{motions}/costume_setting/01_00/motion_loop.anim"
    )));
    assert!(!motion(&format!("{motions}/live/01_00/motion_loop.anim")));
}
