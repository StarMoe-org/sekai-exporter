//! Model files on objects built in memory: the choices a form makes in `ModelConfig`.

#![cfg(feature = "model")]

use std::collections::BTreeMap;

use sekai_convert::model::{ModelConfig, NodeKeys, Prefab, model_file};
use sekai_convert::{Chunk, Context, ConvertError, Object, ObjectId, PPtr, Value};
use serde_json::{Value as Json, json};

/// One file of one bundle, holding the given objects.
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
            .find(|o| o.id == id)
            .cloned()
            .ok_or_else(|| ConvertError::new("missing_object", "no such object"))
    }

    fn resolve(&self, _: ObjectId, reference: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        assert_eq!(reference.file_id, 0);
        Ok((reference.path_id != 0).then(|| id(reference.path_id)))
    }

    fn resource(&self, _: ObjectId, path: &str, _: u64, _: u64) -> Result<Chunk<'_>, ConvertError> {
        Err(ConvertError::new("missing_resource", path))
    }

    fn container(&self, _: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        Ok(Vec::new())
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

fn map(entries: Vec<(Value, Value)>) -> Value {
    Value::Map(entries)
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

fn xy(x: f32, y: f32) -> Value {
    s(vec![("x", f(x)), ("y", f(y))])
}

fn object(path_id: i64, class_id: i32, fields: Value) -> Object {
    Object {
        id: id(path_id),
        class_id,
        class: String::new(),
        script: None,
        fields,
    }
}

/// A game object with the objects `components`, the transform first among them, and active when
/// `active` is set.
fn game_object(path_id: i64, name: &str, components: &[i64], active: bool) -> Object {
    let entries = components
        .iter()
        .map(|&component| s(vec![("component", r(component))]))
        .collect();
    object(
        path_id,
        1,
        s(vec![
            ("m_Name", text(name)),
            ("m_IsActive", Value::Bool(active)),
            ("m_Component", list(entries)),
        ]),
    )
}

fn transform(path_id: i64, owner: i64, position: [f32; 3]) -> Object {
    let [x, y, z] = position;
    object(
        path_id,
        4,
        s(vec![
            ("m_GameObject", r(owner)),
            ("m_LocalPosition", xyz(x, y, z)),
            (
                "m_LocalRotation",
                s(vec![
                    ("x", f(0.0)),
                    ("y", f(0.0)),
                    ("z", f(0.0)),
                    ("w", f(1.0)),
                ]),
            ),
            ("m_LocalScale", xyz(1.0, 1.0, 1.0)),
            ("m_Children", list(Vec::new())),
            ("m_Father", r(0)),
        ]),
    )
}

fn mesh_renderer(path_id: i64, owner: i64, materials: &[i64]) -> Object {
    object(
        path_id,
        23,
        s(vec![
            ("m_GameObject", r(owner)),
            ("m_Enabled", u(1)),
            (
                "m_Materials",
                list(materials.iter().map(|&m| r(m)).collect()),
            ),
        ]),
    )
}

fn mesh_filter(path_id: i64, owner: i64, mesh: i64) -> Object {
    object(
        path_id,
        33,
        s(vec![("m_GameObject", r(owner)), ("m_Mesh", r(mesh))]),
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

/// Three vertices at `(k, k + 1, k + 2)` with the normal `+y`, the tangent `+z` and the texture
/// coordinates `uvs`, in one stream of 48 bytes each.
fn vertex_data(uvs: [[f32; 2]; 3]) -> Value {
    let mut channels = vec![
        channel(0, 0, 3),
        channel(12, 0, 3),
        channel(24, 0, 4),
        channel(0, 0, 0),
        channel(40, 0, 2),
    ];
    channels.extend((6..12).map(|_| channel(0, 0, 0)));
    let mut bytes = Vec::new();
    for (k, uv) in (0..3).zip(uvs) {
        let position = [k as f32, k as f32 + 1.0, k as f32 + 2.0];
        let floats = position
            .into_iter()
            .chain([0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0]);
        bytes.extend(floats.flat_map(f32::to_le_bytes));
        bytes.extend(uv.into_iter().flat_map(f32::to_le_bytes));
    }
    s(vec![
        ("m_VertexCount", u(3)),
        ("m_Channels", list(channels)),
        ("m_DataSize", Value::Bytes(bytes)),
    ])
}

/// A mesh of one submesh with the three vertices `0, 1, 2`.
fn mesh(path_id: i64, name: &str, uvs: [[f32; 2]; 3]) -> Object {
    let indices: Vec<u8> = [0u16, 1, 2].iter().flat_map(|v| v.to_le_bytes()).collect();
    object(
        path_id,
        43,
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

/// A material whose texture properties `textures` bind to textures, or leave without one for 0.
fn material(path_id: i64, name: &str, textures: &[(&str, i64)]) -> Object {
    let entries = textures
        .iter()
        .map(|&(name, texture)| {
            let env = s(vec![
                ("m_Texture", r(texture)),
                ("m_Scale", xy(1.0, 1.0)),
                ("m_Offset", xy(0.0, 0.0)),
            ]);
            (text(name), env)
        })
        .collect();
    object(
        path_id,
        21,
        s(vec![
            ("m_Name", text(name)),
            ("m_Shader", r(0)),
            ("m_ValidKeywords", list(Vec::new())),
            ("m_InvalidKeywords", list(Vec::new())),
            ("m_CustomRenderQueue", i(-1)),
            (
                "m_SavedProperties",
                s(vec![
                    ("m_TexEnvs", map(entries)),
                    ("m_Ints", map(Vec::new())),
                    ("m_Floats", map(Vec::new())),
                    ("m_Colors", map(Vec::new())),
                ]),
            ),
        ]),
    )
}

/// A 2 x 2 RGBA32 texture.
fn texture(path_id: i64, name: &str) -> Object {
    object(
        path_id,
        28,
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
            ("image data", Value::Bytes((0..16).collect())),
            (
                "m_StreamData",
                s(vec![("offset", u(0)), ("size", u(0)), ("path", text(""))]),
            ),
        ]),
    )
}

/// The texture coordinates of a mesh, with `v` beyond the image.
const UVS: [[f32; 2]; 3] = [[0.25, 0.75], [0.0, 1.0], [1.5, -0.25]];

/// An inactive game object `body` that draws the mesh, with one material that binds `_MainTex` to
/// a texture and leaves `_DetailTex` without one.
fn part() -> File {
    File(vec![
        game_object(1, "body", &[2, 9, 10], false),
        transform(2, 1, [0.0, 1.0, 0.0]),
        mesh(5, "body_mesh", UVS),
        material(6, "body_mat", &[("_MainTex", 7), ("_DetailTex", 0)]),
        texture(7, "body_C"),
        mesh_renderer(9, 3, &[6]),
        mesh_filter(10, 3, 5),
    ])
}

fn base_config() -> ModelConfig<'static> {
    ModelConfig {
        flip_uv: true,
        declare_webp: true,
        node_keys: None,
    }
}

/// The part's model file, as a GLB file.
fn part_glb(config: &ModelConfig<'_>) -> Vec<u8> {
    let file = part();
    let prefab = Prefab::read(id(1), &file).expect("a prefab");
    model_file(&prefab, config, &file)
        .expect("a model file")
        .glb
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

    /// The values of the accessor `index`, which reads 32-bit floats from the binary chunk:
    /// `width` for each of its elements.
    fn floats(&self, index: usize, width: usize) -> Vec<Vec<f32>> {
        let accessor = &self.json["accessors"][index];
        assert_eq!(accessor["componentType"], json!(5126));
        let view = &self.json["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
        let start = view["byteOffset"].as_u64().unwrap() as usize;
        let length = view["byteLength"].as_u64().unwrap() as usize;
        let values: Vec<f32> = self.bin[start..start + length]
            .chunks(4)
            .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        let elements: Vec<Vec<f32>> = values.chunks(width).map(<[f32]>::to_vec).collect();
        assert_eq!(elements.len() as u64, accessor["count"].as_u64().unwrap());
        elements
    }

    /// The accessor of the attribute `name` of the first primitive of the mesh `mesh`.
    fn attribute(&self, mesh: usize, name: &str) -> usize {
        self.json["meshes"][mesh]["primitives"][0]["attributes"][name]
            .as_u64()
            .unwrap() as usize
    }

    /// The node extras of `node`.
    fn extras(&self, node: usize) -> &Json {
        &self.json["nodes"][node]["extras"]
    }
}

/// The texture coordinates the first mesh is drawn with.
fn part_uvs(config: &ModelConfig<'_>) -> Vec<[f32; 2]> {
    let glb = Glb::read(&part_glb(config));
    let uv = glb.attribute(0, "TEXCOORD_0");
    glb.floats(uv, 2)
        .into_iter()
        .map(|pair| [pair[0], pair[1]])
        .collect()
}

/// The texture properties of the part's material.
fn texture_properties(config: &ModelConfig<'_>) -> Json {
    let glb = Glb::read(&part_glb(config));
    glb.json["materials"][0]["extras"]["unity"]["textures"].clone()
}

#[test]
fn texture_coordinates_are_flipped_by_default() {
    assert_eq!(
        part_uvs(&base_config()),
        [[0.25, 0.25], [0.0, 0.0], [1.5, 1.25]]
    );
}

#[test]
fn unflipped_texture_coordinates_are_stored_as_they_are() {
    let config = ModelConfig {
        flip_uv: false,
        ..base_config()
    };
    assert_eq!(part_uvs(&config), UVS);
}

#[test]
fn the_webp_extension_is_declared_by_default() {
    let glb = Glb::read(&part_glb(&base_config()));
    assert_eq!(glb.json["extensionsUsed"], json!(["EXT_texture_webp"]));
    assert_eq!(glb.json["extensionsRequired"], json!(["EXT_texture_webp"]));
    assert_eq!(
        glb.json["textures"],
        json!([{ "sampler": 0, "extensions": { "EXT_texture_webp": { "source": 0 } } }])
    );
    assert_eq!(
        glb.json["images"],
        json!([{ "name": "body_C", "uri": "texture/0.webp" }])
    );
}

#[test]
fn the_webp_extension_is_left_out_on_request() {
    let config = ModelConfig {
        declare_webp: false,
        ..base_config()
    };
    let glb = Glb::read(&part_glb(&config));
    assert!(glb.json.get("extensionsUsed").is_none());
    assert!(glb.json.get("extensionsRequired").is_none());
    assert_eq!(glb.json["textures"], json!([{ "sampler": 0, "source": 0 }]));
    assert_eq!(
        glb.json["images"],
        json!([{ "name": "body_C", "uri": "texture/0.webp" }])
    );
}

#[test]
fn a_texture_property_without_a_texture_is_written_with_a_null_texture() {
    assert_eq!(
        texture_properties(&base_config()),
        json!({
            "_MainTex": { "texture": 0, "scale": [1.0, 1.0], "offset": [0.0, 0.0] },
            "_DetailTex": { "texture": null, "scale": [1.0, 1.0], "offset": [0.0, 0.0] }
        })
    );
}

/// The node keys of a source that gives each node the path ID of its game object as
/// `sourceObject`.
struct SourceObjects;

impl NodeKeys for SourceObjects {
    fn keys(
        &self,
        object: ObjectId,
        cx: &dyn Context,
    ) -> Result<BTreeMap<String, Json>, ConvertError> {
        let game_object = cx.object(object)?;
        let name = game_object.fields.get("m_Name").and_then(Value::as_str);
        let source = json!({ "pathId": object.path_id.to_string(), "name": name });
        Ok(BTreeMap::from([("sourceObject".to_owned(), source)]))
    }
}

#[test]
fn node_extras_hold_only_the_state_that_differs_from_the_default() {
    let glb = Glb::read(&part_glb(&base_config()));
    assert_eq!(glb.extras(0), &json!({ "active": false }));
}

#[test]
fn every_node_has_the_keys_of_the_form_and_of_its_source() {
    let config = ModelConfig {
        node_keys: Some(&SourceObjects),
        ..base_config()
    };
    let glb = Glb::read(&part_glb(&config));
    assert_eq!(
        glb.extras(0),
        &json!({ "active": false, "sourceObject": { "name": "body", "pathId": "1" } })
    );
}
