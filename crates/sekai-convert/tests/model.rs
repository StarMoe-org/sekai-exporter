//! Model files on objects built in memory: the choices a form makes in `ModelConfig`, the shaders
//! a material names and the triangles of a mesh that no renderer need draw.

#![cfg(feature = "model")]

use std::collections::BTreeMap;

use sekai_convert::model::{
    MaterialKeys, MeshTriangles, ModelConfig, NodeKeys, ParsedShader, Prefab, StateValue,
    mesh_triangles, model_file,
};
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

    fn file_name(&self, _: ObjectId) -> Result<&str, ConvertError> {
        Ok("CAB-test")
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

/// The map of the string keys `entries`.
fn string_map(entries: &[(&str, &str)]) -> Value {
    let entries = entries
        .iter()
        .map(|&(key, value)| (text(key), text(value)))
        .collect();
    map(entries)
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
        material_keys: None,
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
        let source = json!({
            "file": cx.file_name(object)?,
            "pathId": object.path_id.to_string(),
            "name": name,
        });
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
        &json!({
            "active": false,
            "sourceObject": { "file": "CAB-test", "pathId": "1", "name": "body" },
        })
    );
}

/// Material keys that add `usage`, and the key `clash` under the given name when it is given.
struct MaterialUsage {
    clash: Option<&'static str>,
}

impl MaterialKeys for MaterialUsage {
    fn keys(
        &self,
        material: ObjectId,
        cx: &dyn Context,
    ) -> Result<BTreeMap<String, Json>, ConvertError> {
        let material = cx.object(material)?;
        let name = material.fields.get("m_Name").and_then(Value::as_str);
        let mut keys = BTreeMap::from([
            ("usage".to_owned(), json!(name)),
            (
                "furniture".to_owned(),
                json!({ "tiers": ["base", "detail"] }),
            ),
        ]);
        if let Some(clash) = self.clash {
            keys.insert(clash.to_owned(), Json::Null);
        }
        Ok(keys)
    }
}

/// The material extras of the part's material, written with `keys` as its material keys.
fn part_material_keys(keys: &MaterialUsage) -> Json {
    let config = ModelConfig {
        material_keys: Some(keys),
        ..base_config()
    };
    let glb = Glb::read(&part_glb(&config));
    glb.json["materials"][0]["extras"]["unity"].clone()
}

#[test]
fn the_material_keys_of_the_form_follow_the_keys_of_the_material() {
    let unity = part_material_keys(&MaterialUsage { clash: None });
    assert_eq!(
        unity,
        json!({
            "shader": null,
            "renderQueue": -1,
            "keywords": { "valid": [], "invalid": [] },
            "textures": {
                "_MainTex": { "texture": 0, "scale": [1.0, 1.0], "offset": [0.0, 0.0] },
                "_DetailTex": { "texture": null, "scale": [1.0, 1.0], "offset": [0.0, 0.0] }
            },
            "floats": {},
            "ints": {},
            "colors": {},
            "furniture": { "tiers": ["base", "detail"] },
            "usage": "body_mat"
        })
    );
}

#[test]
fn a_material_key_may_not_take_the_name_of_a_key_of_the_material() {
    let stored = [
        "shader",
        "renderQueue",
        "keywords",
        "textures",
        "floats",
        "ints",
        "colors",
    ];
    for clash in stored {
        let config = ModelConfig {
            material_keys: Some(&MaterialUsage { clash: Some(clash) }),
            ..base_config()
        };
        let file = part();
        let prefab = Prefab::read(id(1), &file).expect("a prefab");
        let error = model_file(&prefab, &config, &file).expect_err("a key of the form is taken");
        assert_eq!(error.code, "malformed");
        assert!(error.message.contains(clash), "{error}");
    }
}

/// The structure `value` without its field `name`.
fn remove(value: &mut Value, name: &str) {
    let Value::Struct(fields) = value else {
        panic!("the value is not a structure")
    };
    fields.retain(|(field, _)| field != name);
}

/// The value of the field `name` of a structure, to replace.
fn get_mut<'v>(value: &'v mut Value, name: &str) -> &'v mut Value {
    let Value::Struct(fields) = value else {
        panic!("the value is not a structure")
    };
    fields
        .iter_mut()
        .find(|(field, _)| field == name)
        .map(|(_, value)| value)
        .unwrap_or_else(|| panic!("{name} is missing"))
}

/// A `SerializedTagMap` of the tags `tags`.
fn tag_map(tags: &[(&str, &str)]) -> Value {
    s(vec![("tags", string_map(tags))])
}

/// A render state value (`SerializedShaderFloatValue`): the property `property` it binds, or
/// `<noninit>` for a value that binds none, and the value compiled into the pass.
fn state_value(property: &str, compiled: f32) -> Value {
    s(vec![("val", f(compiled)), ("name", text(property))])
}

/// The blend state of render target 0 that binds `_ColorMask`, `_SrcBlend` and `_DstBlend`,
/// draws with its blend operators as `fixed`, and blends the alpha channel with `<noninit>`.
fn shader_blend(fixed: f32) -> Value {
    s(vec![
        ("colMask", state_value("_ColorMask", 15.0)),
        ("srcBlend", state_value("_SrcBlend", 1.0)),
        ("destBlend", state_value("_DstBlend", 10.0)),
        ("srcBlendAlpha", state_value("<noninit>", 0.0)),
        ("destBlendAlpha", state_value("<noninit>", 0.0)),
        ("blendOp", state_value("<noninit>", fixed)),
        ("blendOpAlpha", state_value("<noninit>", 0.0)),
    ])
}

/// The state of a pass named `name` with the tags `tags`: it draws with `_Cull`, `_ZTest` and
/// `_ZWrite` and the blend state of `shader_blend(0)`.
fn shader_state(name: &str, tags: &[(&str, &str)]) -> Value {
    s(vec![
        ("m_Name", text(name)),
        ("m_Tags", tag_map(tags)),
        ("culling", state_value("_Cull", 2.0)),
        ("zTest", state_value("_ZTest", 4.0)),
        ("zWrite", state_value("_ZWrite", 0.0)),
        ("rtBlend0", shader_blend(0.0)),
    ])
}

/// A pass as its subshader stores it: its state only.
fn shader_pass(state: Value) -> Value {
    s(vec![("m_State", state)])
}

/// A shader property named `name` of type `kind` whose number default is `default`, and whose
/// default texture, for a texture property, is `texture`.
fn shader_property(name: &str, kind: i64, default: f32, texture: &str) -> Value {
    s(vec![
        ("m_Name", text(name)),
        ("m_Type", i(kind)),
        ("m_DefValue[0]", f(default)),
        ("m_DefValue[1]", f(0.0)),
        ("m_DefValue[2]", f(0.0)),
        ("m_DefValue[3]", f(0.0)),
        (
            "m_DefTexture",
            s(vec![("m_DefaultName", text(texture)), ("m_TexDim", i(2))]),
        ),
    ])
}

/// The properties `Test/Material` declares: two textures with defaults, then each render state
/// property with a default of its own, distinct from the value its passes compile in.
fn shader_properties() -> Vec<Value> {
    vec![
        shader_property("_MainTex", 4, 0.0, "white"),
        shader_property("_Ramp", 4, 0.0, "grey"),
        shader_property("_Cull", 5, 0.0, ""),
        shader_property("_ZTest", 5, 8.0, ""),
        shader_property("_ZWrite", 5, 9.0, ""),
        shader_property("_ColorMask", 5, 7.0, ""),
        shader_property("_SrcBlend", 5, 5.0, ""),
        shader_property("_DstBlend", 5, 3.0, ""),
    ]
}

/// Three subshaders: one of two passes, one whose LightMode only its subshader's tags give, and
/// one without passes.
fn test_sub_shaders() -> Vec<Value> {
    vec![
        s(vec![
            (
                "m_Passes",
                list(vec![
                    shader_pass(shader_state(
                        "Main",
                        &[("LightMode", "ForwardBase"), ("Queue", "Geometry")],
                    )),
                    shader_pass(shader_state("Overlay", &[])),
                ]),
            ),
            ("m_Tags", tag_map(&[("RenderType", "Opaque")])),
            ("m_LOD", i(200)),
        ]),
        s(vec![
            (
                "m_Passes",
                list(vec![shader_pass(shader_state("Shadow", &[]))]),
            ),
            (
                "m_Tags",
                tag_map(&[("LightMode", "ShadowCaster"), ("RenderType", "Transparent")]),
            ),
            ("m_LOD", i(100)),
        ]),
        s(vec![
            ("m_Passes", list(Vec::new())),
            ("m_Tags", tag_map(&[])),
            ("m_LOD", i(0)),
        ]),
    ]
}

/// A shader object named `Test/Material` with the properties `properties` and the subshaders
/// `sub_shaders`.
fn shader_object(properties: Vec<Value>, sub_shaders: Vec<Value>) -> Object {
    object(
        1,
        48,
        s(vec![
            ("m_Name", text("Test/Material")),
            (
                "m_ParsedForm",
                s(vec![
                    ("m_PropInfo", s(vec![("m_Props", list(properties))])),
                    ("m_SubShaders", list(sub_shaders)),
                    ("m_Name", text("Test/Material")),
                    ("m_FallbackName", text("")),
                ]),
            ),
        ]),
    )
}

/// The shader `Test/Material`.
fn test_shader() -> Object {
    shader_object(shader_properties(), test_sub_shaders())
}

fn parse(shader: Object) -> Result<ParsedShader, ConvertError> {
    ParsedShader::read(id(1), &File(vec![shader]))
}

#[test]
fn a_shader_is_read_with_its_name_texture_defaults_and_subshaders() {
    let parsed = parse(test_shader()).expect("a shader");
    assert_eq!(parsed.name, "Test/Material");
    assert_eq!(
        parsed.texture_defaults_json(),
        json!({ "_MainTex": "white", "_Ramp": "grey" })
    );
    let tag = |name: &str, value: &str| (name.to_owned(), value.to_owned());
    let opaque = [tag("RenderType", "Opaque")];
    let transparent = [
        tag("LightMode", "ShadowCaster"),
        tag("RenderType", "Transparent"),
    ];
    let forward = [tag("LightMode", "ForwardBase"), tag("Queue", "Geometry")];
    let sub_shaders: Vec<_> = parsed
        .sub_shaders
        .iter()
        .map(|sub_shader| {
            let passes: Vec<_> = sub_shader
                .passes
                .iter()
                .map(|pass| (pass.name.as_str(), pass.tags.as_slice()))
                .collect();
            (sub_shader.tags.as_slice(), passes)
        })
        .collect();
    assert_eq!(
        sub_shaders,
        [
            (
                &opaque[..],
                vec![("Main", &forward[..]), ("Overlay", &[][..])]
            ),
            (&transparent[..], vec![("Shadow", &[][..])]),
            (&[][..], Vec::new()),
        ]
    );
}

#[test]
fn a_pass_reads_the_values_its_state_binds_and_the_values_it_carries() {
    let parsed = parse(test_shader()).expect("a shader");
    let main = &parsed.sub_shaders[0].passes[0];
    assert_eq!(
        main.culling,
        StateValue::Property {
            name: "_Cull".to_owned(),
            default: Some(0.0),
            compiled: 2.0,
        }
    );
    assert_eq!(
        main.z_test,
        StateValue::Property {
            name: "_ZTest".to_owned(),
            default: Some(8.0),
            compiled: 4.0,
        }
    );
    assert_eq!(
        main.z_write,
        StateValue::Property {
            name: "_ZWrite".to_owned(),
            default: Some(9.0),
            compiled: 0.0,
        }
    );
    assert_eq!(
        main.blend.dest_blend,
        StateValue::Property {
            name: "_DstBlend".to_owned(),
            default: Some(3.0),
            compiled: 10.0,
        }
    );
    assert_eq!(main.blend.src_blend_alpha, StateValue::Fixed(0.0));
    assert_eq!(main.blend.blend_op, StateValue::Fixed(0.0));
}

#[test]
fn a_light_mode_tag_of_a_subshader_is_not_the_light_mode_of_its_passes() {
    let parsed = parse(test_shader()).expect("a shader");
    let light_modes: Vec<_> = parsed
        .passes_json()
        .iter()
        .map(|row| row["lightMode"].clone())
        .collect();
    assert_eq!(
        light_modes,
        [json!("ForwardBase"), json!(null), json!(null)]
    );
}

#[test]
fn passes_json_names_each_pass_with_its_shader_defaults_and_fixed_values() {
    let parsed = parse(test_shader()).expect("a shader");
    let rows = parsed.passes_json();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows[0],
        json!({
            "name": "Main",
            "lightMode": "ForwardBase",
            "subShaderIndex": 0,
            "passIndex": 0,
            "subShaderTags": { "RenderType": "Opaque" },
            "tags": { "LightMode": "ForwardBase", "Queue": "Geometry" },
            "renderState": {
                "culling": { "name": "_Cull", "default": 0.0, "val": 2.0 },
                "zTest": { "name": "_ZTest", "default": 8.0, "val": 4.0 },
                "zWrite": { "name": "_ZWrite", "default": 9.0, "val": 0.0 },
                "blend": {
                    "colMask": { "name": "_ColorMask", "default": 7.0, "val": 15.0 },
                    "srcBlend": { "name": "_SrcBlend", "default": 5.0, "val": 1.0 },
                    "destBlend": { "name": "_DstBlend", "default": 3.0, "val": 10.0 },
                    "srcBlendAlpha": { "name": "<noninit>", "val": 0.0 },
                    "destBlendAlpha": { "name": "<noninit>", "val": 0.0 },
                    "blendOp": { "name": "<noninit>", "val": 0.0 },
                    "blendOpAlpha": { "name": "<noninit>", "val": 0.0 }
                }
            }
        })
    );
    assert_eq!(rows[1]["subShaderIndex"], json!(0));
    assert_eq!(rows[1]["passIndex"], json!(1));
    assert_eq!(rows[1]["tags"], json!({}));
    assert_eq!(rows[2]["name"], json!("Shadow"));
    assert_eq!(rows[2]["subShaderIndex"], json!(1));
    assert_eq!(
        rows[2]["subShaderTags"],
        json!({ "LightMode": "ShadowCaster", "RenderType": "Transparent" })
    );
}

/// A shader named `Test/Material` with a single subshader through the pass `pass`.
fn shader_with_pass(pass: Value) -> Object {
    let sub_shaders = vec![s(vec![
        ("m_Passes", list(vec![pass])),
        ("m_Tags", tag_map(&[])),
        ("m_LOD", i(0)),
    ])];
    shader_object(shader_properties(), sub_shaders)
}

/// The error of reading the shader `shader`.
fn malformed(shader: Object) -> ConvertError {
    parse(shader).expect_err("the shader is malformed")
}

#[test]
fn a_state_that_binds_a_property_the_shader_does_not_declare_has_no_default() {
    let mut state = shader_state("Main", &[]);
    *get_mut(&mut state, "culling") = state_value("_CullFront", 1.0);
    let parsed = parse(shader_with_pass(shader_pass(state))).expect("a shader");
    assert_eq!(
        parsed.passes_json()[0]["renderState"]["culling"],
        json!({ "name": "_CullFront", "val": 1.0 })
    );
}

#[test]
fn a_pass_without_a_z_test_is_malformed() {
    let mut state = shader_state("Main", &[]);
    remove(&mut state, "zTest");
    assert_eq!(
        malformed(shader_with_pass(shader_pass(state))).code,
        "malformed"
    );
}

#[test]
fn a_pass_without_the_blend_state_of_render_target_0_is_malformed() {
    let mut state = shader_state("Main", &[]);
    remove(&mut state, "rtBlend0");
    assert_eq!(
        malformed(shader_with_pass(shader_pass(state))).code,
        "malformed"
    );
}

#[test]
fn an_object_that_is_not_a_shader_is_malformed() {
    let material = object(1, 21, s(vec![("m_Name", text("Test/Material"))]));
    assert_eq!(malformed(material).code, "malformed");
}

/// One submesh of the index buffer: `count` 16-bit indices from `first`, drawn from the vertex
/// `base_vertex` on.
fn submesh(first: u64, count: u64, base_vertex: u64) -> Value {
    s(vec![
        ("firstByte", u(first)),
        ("indexCount", u(count)),
        ("topology", i(0)),
        ("baseVertex", u(base_vertex)),
    ])
}

/// The three-vertex mesh of the part with its submeshes `submeshes` and its 16-bit `indices`.
fn mesh_with_submeshes(submeshes: Vec<Value>, indices: &[u16]) -> Object {
    let mut mesh = mesh(5, "collider", UVS);
    *get_mut(&mut mesh.fields, "m_SubMeshes") = list(submeshes);
    *get_mut(&mut mesh.fields, "m_IndexBuffer") =
        Value::Bytes(indices.iter().flat_map(|i| i.to_le_bytes()).collect());
    mesh
}

/// The triangles of the mesh `mesh`.
fn triangles_of(mesh: Object) -> Result<MeshTriangles, ConvertError> {
    mesh_triangles(mesh.id, &File(vec![mesh]))
}

#[test]
fn mesh_triangles_reflect_the_positions_and_reverse_the_triangles() {
    let mesh = mesh_with_submeshes(
        vec![submesh(0, 3, 0), submesh(6, 3, 0)],
        &[0, 1, 2, 2, 1, 0],
    );
    assert_eq!(
        triangles_of(mesh).expect("a collider mesh"),
        MeshTriangles {
            positions: vec![[0.0, 1.0, 2.0], [-1.0, 2.0, 3.0], [-2.0, 3.0, 4.0]],
            triangles: vec![[0, 2, 1], [2, 0, 1]],
        }
    );
}

#[test]
fn mesh_triangles_apply_the_base_vertex_of_each_submesh() {
    let mesh = mesh_with_submeshes(
        vec![submesh(0, 3, 0), submesh(6, 3, 1)],
        &[0, 2, 1, 1, 0, 0],
    );
    let triangles = triangles_of(mesh).expect("a collider mesh");
    assert_eq!(triangles.triangles, [[0, 1, 2], [2, 1, 1]]);
}

#[test]
fn a_mesh_without_triangles_is_malformed() {
    let mesh = mesh_with_submeshes(vec![submesh(0, 0, 0)], &[]);
    assert_eq!(
        triangles_of(mesh).expect_err("no triangles").code,
        "malformed"
    );
}

#[test]
fn a_triangle_index_outside_the_positions_is_malformed() {
    let mesh = mesh_with_submeshes(
        vec![submesh(0, 3, 0), submesh(6, 3, 1)],
        &[0, 1, 2, 0, 1, 2],
    );
    assert_eq!(
        triangles_of(mesh).expect_err("the second submesh starts at vertex 3"),
        ConvertError::new("malformed", "an index lies outside the vertices"),
    );
}

#[test]
fn a_position_that_is_not_finite_is_malformed() {
    let mut mesh = mesh(5, "collider", UVS);
    let vertex_data = get_mut(&mut mesh.fields, "m_VertexData");
    let Value::Bytes(bytes) = get_mut(vertex_data, "m_DataSize") else {
        panic!("vertex data holds bytes")
    };
    bytes[0..4].copy_from_slice(&f32::NAN.to_le_bytes());
    assert_eq!(
        triangles_of(mesh).expect_err("a vertex is NaN").code,
        "malformed"
    );
}
