//! The `scenario-prefab` form on objects built in memory.

#![cfg(feature = "episode")]

use sekai_convert::{
    Asset, Chunk, Content, Context, ConvertError, Converter, EngineObject, Object, ObjectId,
    ObjectIndex, PPtr, Value,
};
use sekai_moesekai::roots;
use sekai_moesekai::scenario_prefab::ScenarioPrefab;
use serde_json::{Value as Json, json};

/// The file of the engine's own resources, as references name it with file ID 1.
const ENGINE_FILE: &str = "Library/unity default resources";
const BUILD_DATA: &str = "Assembly-CSharp/Sekai.ScenarioBackgroundBuildData";

/// One bundle with one file, holding the given objects, and its resource file.
struct Bundle {
    objects: Vec<Object>,
    resource: Vec<u8>,
}

impl Context for Bundle {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        self.objects
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

    fn resource(
        &self,
        _: ObjectId,
        _: &str,
        offset: u64,
        size: u64,
    ) -> Result<Chunk<'_>, ConvertError> {
        let range = usize::try_from(offset).unwrap()..usize::try_from(offset + size).unwrap();
        let bytes = self.resource.get(range);
        bytes
            .map(Chunk::Borrowed)
            .ok_or_else(|| ConvertError::new("malformed", "outside the resource file"))
    }

    fn container(&self, _from: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        Ok(Vec::new())
    }

    fn file_name(&self, _from: ObjectId) -> Result<&str, ConvertError> {
        Ok("CAB-test")
    }

    fn engine_object(
        &self,
        _from: ObjectId,
        reference: PPtr,
    ) -> Result<Option<EngineObject>, ConvertError> {
        Ok((reference.file_id == 1).then(|| EngineObject {
            file: ENGINE_FILE.into(),
            path_id: reference.path_id,
        }))
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

fn engine(path_id: i64) -> Value {
    Value::PPtr(PPtr {
        file_id: 1,
        path_id,
    })
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
    let components = components
        .iter()
        .map(|&c| s(vec![("component", r(c))]))
        .collect();
    object(
        path_id,
        1,
        "GameObject",
        s(vec![
            ("m_Component", list(components)),
            ("m_Name", text(name)),
        ]),
    )
}

/// A 2×2 RGBA32 texture.
fn texture(path_id: i64) -> Object {
    object(
        path_id,
        28,
        "Texture2D",
        s(vec![
            ("m_Name", text("tex_kirakira")),
            ("m_Width", i(2)),
            ("m_Height", i(2)),
            ("m_TextureFormat", i(4)),
            ("m_MipCount", i(1)),
            ("image data", Value::Bytes((0..16).collect())),
            (
                "m_StreamData",
                s(vec![("offset", u(0)), ("size", u(0)), ("path", text(""))]),
            ),
        ]),
    )
}

/// A clip that drives nothing.
fn clip(path_id: i64) -> Object {
    let empty = || list(Vec::new());
    let data = s(vec![
        (
            "m_StreamedClip",
            s(vec![("data", empty()), ("curveCount", u(0))]),
        ),
        (
            "m_DenseClip",
            s(vec![("m_FrameCount", i(0)), ("m_CurveCount", u(0))]),
        ),
        ("m_ConstantClip", s(vec![("data", empty())])),
    ]);
    object(
        path_id,
        74,
        "AnimationClip",
        s(vec![
            ("m_Name", text("kirakira")),
            ("m_Legacy", Value::Bool(false)),
            ("m_Compressed", Value::Bool(false)),
            ("m_SampleRate", f(60.0)),
            (
                "m_MuscleClip",
                s(vec![
                    ("m_Clip", s(vec![("data", data)])),
                    ("m_StartTime", f(0.0)),
                    ("m_StopTime", f(1.0)),
                    ("m_LoopTime", Value::Bool(false)),
                ]),
            ),
            (
                "m_ClipBindingConstant",
                s(vec![
                    ("genericBindings", empty()),
                    ("pptrCurveMapping", empty()),
                ]),
            ),
        ]),
    )
}

/// An effect: a game object with a transform, a sprite renderer drawing the built-in quad with a
/// material, and an animator whose controller plays a clip. The shader refers back to the game
/// object, which the walk does not follow.
fn effect() -> Bundle {
    let objects = vec![
        game_object(1, "kirakira", &[2, 3, 4]),
        object(
            2,
            4,
            "Transform",
            s(vec![
                ("m_GameObject", r(1)),
                ("m_LocalPosition", s(vec![("x", f(1.5)), ("y", f(0.0))])),
                ("m_Children", list(Vec::new())),
                ("m_Father", r(0)),
            ]),
        ),
        object(
            3,
            212,
            "SpriteRenderer",
            s(vec![
                ("m_GameObject", r(1)),
                ("m_Materials", list(vec![r(5)])),
                ("m_Mesh", engine(10210)),
            ]),
        ),
        object(
            4,
            95,
            "Animator",
            s(vec![("m_GameObject", r(1)), ("m_Controller", r(8))]),
        ),
        object(
            5,
            21,
            "Material",
            s(vec![
                ("m_Name", text("mat_kirakira")),
                ("m_Shader", r(6)),
                (
                    "m_TexEnvs",
                    Value::Map(vec![(text("_MainTex"), s(vec![("m_Texture", r(7))]))]),
                ),
            ]),
        ),
        object(
            6,
            48,
            "Shader",
            s(vec![
                ("m_ParsedForm", s(vec![("m_Name", text("Sekai/Additive"))])),
                ("m_Dependencies", list(vec![r(1)])),
            ]),
        ),
        texture(7),
        object(
            8,
            91,
            "AnimatorController",
            s(vec![
                ("m_Name", text("kirakira")),
                ("m_AnimationClips", list(vec![r(9)])),
            ]),
        ),
        clip(9),
    ];
    Bundle {
        objects,
        resource: Vec::new(),
    }
}

fn convert(root: i64, bundle: &Bundle) -> Result<Asset, ConvertError> {
    Ok(ScenarioPrefab.convert(id(root), bundle)?.expect("an asset"))
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

fn document(asset: &Asset) -> Json {
    serde_json::from_slice(&file(asset, "prefab")).unwrap()
}

fn roles(asset: &Asset) -> Vec<&str> {
    asset.members.iter().map(|m| m.role.as_str()).collect()
}

#[test]
fn an_effect_lists_its_objects_in_first_reference_order() {
    let asset = convert(1, &effect()).unwrap();
    let entry = |class: &str, id: &str, extra: Json| {
        let mut entry = json!({ "file": "CAB-test", "id": id, "class": class });
        entry
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        entry
    };
    let expected = json!({
        "format": "scenario-prefab",
        "version": 1,
        "objects": [
            entry("GameObject", "1", json!({ "class_id": 1, "name": "kirakira", "fields": {
                "m_Component": [
                    { "component": { "$ref": 1 } },
                    { "component": { "$ref": 2 } },
                    { "component": { "$ref": 3 } },
                ],
                "m_Name": "kirakira",
            }})),
            entry("Transform", "2", json!({ "class_id": 4, "fields": {
                "m_GameObject": { "$ref": 0 },
                "m_LocalPosition": { "x": 1.5, "y": 0 },
                "m_Children": [],
                "m_Father": null,
            }})),
            entry("SpriteRenderer", "3", json!({ "class_id": 212, "fields": {
                "m_GameObject": { "$ref": 0 },
                "m_Materials": [{ "$ref": 4 }],
                "m_Mesh": { "$ref": { "external": ENGINE_FILE, "id": "10210" } },
            }})),
            entry("Animator", "4", json!({ "class_id": 95, "fields": {
                "m_GameObject": { "$ref": 0 },
                "m_Controller": { "$ref": 5 },
            }})),
            entry("Material", "5", json!({ "class_id": 21, "name": "mat_kirakira", "fields": {
                "m_Name": "mat_kirakira",
                "m_Shader": { "$ref": 6 },
                "m_TexEnvs": [["_MainTex", { "m_Texture": { "$ref": 7 } }]],
            }})),
            entry("AnimatorController", "8", json!({ "class_id": 91, "name": "kirakira",
                "fields": { "m_Name": "kirakira", "m_AnimationClips": [{ "$ref": 8 }] },
            })),
            entry("Shader", "6", json!({ "class_id": 48, "name": "Sekai/Additive" })),
            entry("Texture2D", "7", json!({ "class_id": 28, "name": "tex_kirakira",
                "member": "texture/7",
                "fields": {
                    "m_Name": "tex_kirakira",
                    "m_Width": 2,
                    "m_Height": 2,
                    "m_TextureFormat": 4,
                    "m_MipCount": 1,
                },
            })),
            entry("AnimationClip", "9", json!({ "class_id": 74, "name": "kirakira",
                "member": "clip/8",
                "fields": clip_fields(),
            })),
        ],
    });
    assert_eq!(document(&asset), expected);
    assert_eq!(roles(&asset), ["prefab", "texture/7", "clip/8"]);
}

/// The stored fields of [`clip`].
fn clip_fields() -> Json {
    json!({
        "m_Name": "kirakira",
        "m_Legacy": false,
        "m_Compressed": false,
        "m_SampleRate": 60,
        "m_MuscleClip": {
            "m_Clip": { "data": {
                "m_StreamedClip": { "data": [], "curveCount": 0 },
                "m_DenseClip": { "m_FrameCount": 0, "m_CurveCount": 0 },
                "m_ConstantClip": { "data": [] },
            }},
            "m_StartTime": 0,
            "m_StopTime": 1,
            "m_LoopTime": false,
        },
        "m_ClipBindingConstant": { "genericBindings": [], "pptrCurveMapping": [] },
    })
}

#[test]
fn a_clip_that_clip_documents_do_not_represent_keeps_its_fields_and_references() {
    let mut bundle = effect();
    let event = s(vec![
        ("time", f(0.5)),
        ("functionName", text("Spawn")),
        ("data", text("")),
        ("objectReferenceParameter", r(10)),
        ("floatParameter", f(0.0)),
        ("intParameter", i(0)),
        ("messageOptions", i(0)),
    ]);
    let clip = bundle.objects.iter_mut().find(|o| o.id == id(9)).unwrap();
    let Value::Struct(fields) = &mut clip.fields else {
        panic!("not a structure")
    };
    fields.push(("m_Events".into(), list(vec![event])));
    bundle.objects.push(game_object(10, "spawned", &[]));

    let asset = convert(1, &bundle).unwrap();
    let objects = document(&asset)["objects"].clone();
    assert_eq!(objects[8]["class"], "AnimationClip");
    assert_eq!(objects[8].get("member"), None);
    let events = &objects[8]["fields"]["m_Events"];
    assert_eq!(events[0]["objectReferenceParameter"], json!({ "$ref": 9 }));
    assert_eq!(objects[9]["name"], "spawned");
    assert_eq!(roles(&asset), ["prefab", "texture/7"]);
}

#[test]
fn the_same_prefab_gives_the_same_bytes() {
    let bundle = effect();
    let a = convert(1, &bundle).unwrap();
    let b = convert(1, &bundle).unwrap();
    assert_eq!(file(&a, "prefab"), file(&b, "prefab"));
}

#[test]
fn a_background_starts_at_its_build_data_and_lists_scripts_by_name() {
    let mut bundle = effect();
    bundle.objects.extend([
        Object {
            script: Some(BUILD_DATA.into()),
            ..object(
                20,
                114,
                "MonoBehaviour",
                s(vec![
                    ("m_GameObject", r(0)),
                    ("m_Script", r(21)),
                    ("m_Name", text("ScenarioBGData")),
                    ("BackgroundPrefab", r(1)),
                    ("ForegroundPrefab", r(0)),
                ]),
            )
        },
        object(
            21,
            115,
            "MonoScript",
            s(vec![
                ("m_Name", text("ScenarioBackgroundBuildData")),
                ("m_ClassName", text("ScenarioBackgroundBuildData")),
                ("m_Namespace", text("Sekai")),
                ("m_AssemblyName", text("Assembly-CSharp.dll")),
                ("m_Dependencies", list(vec![r(1)])),
            ]),
        ),
    ]);
    let objects = document(&convert(20, &bundle).unwrap())["objects"].clone();
    assert_eq!(objects[0]["script"], BUILD_DATA);
    assert_eq!(objects[0]["fields"]["m_Script"], json!({ "$ref": 1 }));
    assert_eq!(
        objects[0]["fields"]["BackgroundPrefab"],
        json!({ "$ref": 2 })
    );
    assert_eq!(objects[0]["fields"]["ForegroundPrefab"], Json::Null);
    assert_eq!(
        objects[1],
        json!({ "file": "CAB-test", "id": "21", "class": "MonoScript", "class_id": 115,
                "name": BUILD_DATA })
    );
    assert_eq!(objects[2]["class"], "GameObject");
    assert_eq!(objects.as_array().unwrap().len(), 11);
}

#[test]
fn values_are_written_as_json_readers_hold_them() {
    let large = vec![7u8; 5000];
    let bundle = Bundle {
        objects: vec![
            game_object(1, "values", &[2, 3, 4]),
            object(
                2,
                114,
                "MonoBehaviour",
                s(vec![
                    ("small", i(-(1 << 53))),
                    ("big", i(-(1 << 53) - 1)),
                    ("unsigned", u(u64::MAX)),
                    ("single", f(0.1)),
                    ("double", Value::Float(0.1)),
                    ("nan", Value::Float(f64::NAN)),
                    ("infinite", Value::Float(f64::NEG_INFINITY)),
                    ("bytes", Value::Bytes(vec![1, 2, 3])),
                    ("large", Value::Bytes(large.clone())),
                    ("again", Value::Bytes(large.clone())),
                    ("text", text("a\"b")),
                ]),
            ),
            object(
                3,
                43,
                "Mesh",
                s(vec![(
                    "m_StreamData",
                    s(vec![
                        ("offset", u(2)),
                        ("size", u(3)),
                        ("path", text("archive:/CAB-test/CAB-test.resS")),
                    ]),
                )]),
            ),
            object(
                4,
                83,
                "AudioClip",
                s(vec![(
                    "m_Resource",
                    s(vec![
                        ("m_Source", text("archive:/CAB-test/CAB-test.resource")),
                        ("m_Offset", u(0)),
                        ("m_Size", u(2)),
                    ]),
                )]),
            ),
        ],
        resource: vec![10, 11, 12, 13, 14],
    };
    let asset = convert(1, &bundle).unwrap();
    let doc = document(&asset);
    let fields = &doc["objects"][1]["fields"];
    let large_hash = file_sha256(&large);
    assert_eq!(fields["small"], json!(-(1i64 << 53)));
    assert_eq!(fields["big"], json!("-9007199254740993"));
    assert_eq!(fields["unsigned"], json!(u64::MAX.to_string()));
    assert_eq!(serde_json::to_string(&fields["single"]).unwrap(), "0.1");
    assert_eq!(fields["double"], json!(0.1));
    assert_eq!(fields["nan"], json!("NaN"));
    assert_eq!(fields["infinite"], json!("-Infinity"));
    assert_eq!(
        fields["bytes"],
        json!({ "$bytes": { "size": 3, "sha256": file_sha256(&[1, 2, 3]), "base64": "AQID" } })
    );
    let member = format!("bytes/{large_hash}");
    let large_payload =
        json!({ "$bytes": { "size": 5000, "sha256": large_hash, "member": member } });
    assert_eq!(fields["large"], large_payload);
    assert_eq!(fields["again"], large_payload);
    assert_eq!(fields["text"], "a\"b");
    let streamed = |n: usize, field: &str| doc["objects"][n]["fields"][field]["$bytes"].clone();
    assert_eq!(streamed(2, "m_StreamData")["base64"], "DA0O");
    assert_eq!(streamed(3, "m_Resource")["base64"], "Cgs=");
    assert_eq!(roles(&asset), ["prefab", member.as_str()]);
    assert_eq!(file(&asset, &member), large);
}

fn file_sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn a_reference_to_a_missing_object_is_an_error() {
    let mut bundle = effect();
    bundle.objects[0] = game_object(1, "kirakira", &[2, 99]);
    assert_eq!(convert(1, &bundle).unwrap_err().code, "missing_object");
}

#[test]
fn bytes_outside_the_resource_file_are_an_error() {
    let mut bundle = effect();
    bundle.objects.push(object(
        30,
        43,
        "Mesh",
        s(vec![(
            "m_StreamData",
            s(vec![
                ("offset", u(4)),
                ("size", u(8)),
                ("path", text("archive:/CAB-test/CAB-test.resS")),
            ]),
        )]),
    ));
    bundle.objects[0] = game_object(1, "kirakira", &[30]);
    assert_eq!(convert(1, &bundle).unwrap_err().code, "malformed");
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
fn roots_are_background_build_data_and_effect_prefabs() {
    let root = |class_id: i32, script: Option<&str>, path: &str| {
        roots::scenario_prefab(&index(class_id), script, &[path.to_owned()])
    };
    let base = "assets/sekai/assetbundle/resources/ondemand/scenario";
    let effect = format!("{base}/effect/kirakira/kirakira.prefab");
    assert!(root(1, None, &effect));
    assert!(root(1, None, &effect.to_uppercase()));
    assert!(!root(28, None, &format!("{base}/effect/kirakira/tex.png")));
    assert!(!root(
        1,
        None,
        "assets/sekai/live_pv/effect/kirakira.prefab"
    ));
    let background = format!("{base}/background/bg_s000387/scenariobgdata.asset");
    assert!(root(114, Some(BUILD_DATA), &background));
    assert!(!root(114, Some("Assembly-CSharp/Sekai.Other"), &background));
    let prefab = format!("{base}/background/bg_s000387/backgrounds000387.prefab");
    assert!(!root(1, None, &prefab));
}
