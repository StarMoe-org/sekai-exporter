//! The `live2d-model` and `live2d-motions` forms, and clip documents, on objects built in memory.

#![cfg(feature = "live2d")]

use sekai_convert::live2d::{Model, Motions};
use sekai_convert::{
    Chunk, Content, Context, ConvertError, Converter, Member, Object, ObjectId, PPtr, Value,
};
use sekai_format::clip::CurveData;
use sekai_format::live2d::{MotionDocument, PARAMETER_ATTRIBUTE};

/// One bundle with one file, holding the given objects and container.
struct Bundle {
    objects: Vec<Object>,
    container: Vec<(String, ObjectId)>,
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
        let target = id(reference.path_id);
        if self.objects.iter().any(|o| o.id == target) {
            Ok(Some(target))
        } else {
            Err(ConvertError::new("missing_object", "no such object"))
        }
    }

    fn resource(&self, _: ObjectId, path: &str, _: u64, _: u64) -> Result<Chunk<'_>, ConvertError> {
        Err(ConvertError::new("missing_resource", path))
    }

    fn container(&self, _from: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        Ok(self.container.clone())
    }
}

impl Bundle {
    fn replace(&mut self, path_id: i64, edit: impl FnOnce(&mut Object)) {
        let object = self
            .objects
            .iter_mut()
            .find(|o| o.id.path_id == path_id)
            .expect("the object exists");
        edit(object);
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

fn u(v: u32) -> Value {
    Value::UInt(v.into())
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

/// Sets the field `name` of a structure.
fn set(value: &mut Value, name: &str, to: Value) {
    let Value::Struct(fields) = value else {
        panic!("not a structure")
    };
    let slot = fields
        .iter_mut()
        .find(|(n, _)| n == name)
        .expect("the field exists");
    slot.1 = to;
}

/// The field `name` of a structure, to change.
fn get<'a>(value: &'a mut Value, name: &str) -> &'a mut Value {
    let Value::Struct(fields) = value else {
        panic!("not a structure")
    };
    &mut fields
        .iter_mut()
        .find(|(n, _)| n == name)
        .expect("the field exists")
        .1
}

fn object(path_id: i64, class_id: i32, script: Option<&str>, fields: Value) -> Object {
    Object {
        id: id(path_id),
        class_id,
        class: match class_id {
            28 => "Texture2D",
            49 => "TextAsset",
            74 => "AnimationClip",
            114 => "MonoBehaviour",
            115 => "MonoScript",
            _ => "",
        }
        .into(),
        script: script.map(str::to_owned),
        fields,
    }
}

const SCRIPT: i64 = 90;
const MODEL_DATA: &str = "Assembly-CSharp/Sekai.Live2D.Live2DBuildModelData";
const MOTION_DATA: &str = "Assembly-CSharp/Sekai.Live2D.Live2DBuildMotionData";
const MOTION_META: &str = "Assembly-CSharp/Sekai.Live2D.Live2DBuildMotionMetaData";
const DIR: &str = "assets/sekai/assetbundle/resources/startapp/live2d/model/v2/main/m";

fn script() -> Object {
    object(
        SCRIPT,
        115,
        None,
        s(vec![
            ("m_ClassName", text("CubismParameter")),
            ("m_Namespace", text("Live2D.Cubism.Core")),
            ("m_AssemblyName", text("Live2D.Cubism.dll")),
        ]),
    )
}

/// A frame of a streamed clip: its time and its keys `(curve, [a, b, c, d])`.
type Frame<'a> = (f32, &'a [(i32, [f32; 4])]);

/// The words of a streamed clip: `frames`, then the closing frame at +∞.
fn words(frames: &[Frame<'_>]) -> Value {
    let mut out = Vec::new();
    for (time, keys) in frames {
        out.push(time.to_bits());
        out.push(keys.len() as u32);
        for (curve, coefficients) in *keys {
            out.push(*curve as u32);
            out.extend(coefficients.map(f32::to_bits));
        }
    }
    out.extend([f32::INFINITY.to_bits(), 0]);
    list(out.into_iter().map(u).collect())
}

fn binding(path: u32, type_id: i64) -> Value {
    s(vec![
        ("path", u(path)),
        ("attribute", u(PARAMETER_ATTRIBUTE)),
        ("script", r(SCRIPT)),
        ("typeID", i(type_id)),
        ("customType", i(0)),
        ("isPPtrCurve", i(0)),
        ("isIntCurve", i(0)),
        ("isSerializeReferenceCurve", i(0)),
    ])
}

/// A clip with two streamed curves, one dense curve of three frames and one constant curve.
fn clip(path_id: i64, name: &str) -> Object {
    let streamed = words(&[
        (
            -f32::MAX,
            &[(0, [0.0, 0.0, 0.0, 1.0]), (1, [0.0, 0.0, 2.0, -1.0])],
        ),
        (0.5, &[(0, [1.0, -2.0, 0.25, 1.0])]),
    ]);
    let empty = || list(Vec::new());
    object(
        path_id,
        74,
        None,
        s(vec![
            ("m_Name", text(name)),
            ("m_Legacy", Value::Bool(false)),
            ("m_Compressed", Value::Bool(false)),
            ("m_RotationCurves", empty()),
            ("m_CompressedRotationCurves", empty()),
            ("m_EulerCurves", empty()),
            ("m_PositionCurves", empty()),
            ("m_ScaleCurves", empty()),
            ("m_FloatCurves", empty()),
            ("m_PPtrCurves", empty()),
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
                                    s(vec![
                                        ("data", streamed),
                                        ("curveCount", u(2)),
                                        ("discreteCurveCount", u(0)),
                                    ]),
                                ),
                                (
                                    "m_DenseClip",
                                    s(vec![
                                        ("m_FrameCount", i(3)),
                                        ("m_CurveCount", u(1)),
                                        ("m_SampleRate", f(30.0)),
                                        ("m_BeginTime", f(0.25)),
                                        ("m_SampleArray", list(vec![f(1.0), f(2.0), f(4.0)])),
                                    ]),
                                ),
                                ("m_ConstantClip", s(vec![("data", list(vec![f(0.1)]))])),
                            ]),
                        )]),
                    ),
                    ("m_StartTime", f(0.0)),
                    ("m_StopTime", f(1.5)),
                    ("m_CycleOffset", f(0.0)),
                    ("m_LoopTime", Value::Bool(true)),
                ]),
            ),
            (
                "m_ClipBindingConstant",
                s(vec![
                    (
                        "genericBindings",
                        list(vec![
                            binding(11, 114),
                            binding(12, 114),
                            binding(13, 114),
                            binding(14, 114),
                        ]),
                    ),
                    ("pptrCurveMapping", empty()),
                ]),
            ),
            ("m_HasGenericRootTransform", Value::Bool(false)),
            ("m_HasMotionFloatCurves", Value::Bool(false)),
            (
                "m_Events",
                list(vec![s(vec![
                    ("time", f(0.0)),
                    ("functionName", text("OnLive2DInvokeUserData")),
                    ("data", text("eyeblink,0.2")),
                    ("objectReferenceParameter", r(0)),
                    ("floatParameter", f(0.0)),
                    ("intParameter", i(2)),
                    ("messageOptions", i(0)),
                ])]),
            ),
        ]),
    )
}

fn meta(path_id: i64, fade_in: f32, fade_out: f32) -> Object {
    object(
        path_id,
        114,
        Some(MOTION_META),
        s(vec![
            ("FadeInTime", f(fade_in)),
            ("FadeOutTime", f(fade_out)),
        ]),
    )
}

fn entry(clip: i64, meta: i64, name: &str) -> Value {
    s(vec![
        ("Clip", r(clip)),
        ("Meta", r(meta)),
        ("ClipAssetName", text(name)),
    ])
}

fn file_object(path_id: i64, bytes: &[u8]) -> Object {
    object(
        path_id,
        49,
        None,
        s(vec![("m_Script", Value::Bytes(bytes.to_vec()))]),
    )
}

/// A 2 x 1 RGBA32 texture.
fn texture(path_id: i64) -> Object {
    object(
        path_id,
        28,
        None,
        s(vec![
            ("m_Width", i(2)),
            ("m_Height", i(1)),
            ("m_TextureFormat", i(4)),
            ("image data", Value::Bytes(vec![1, 2, 3, 4, 5, 6, 7, 8])),
            (
                "m_StreamData",
                s(vec![("offset", i(0)), ("size", i(0)), ("path", text(""))]),
            ),
        ]),
    )
}

/// A model bundle: build data, a `.moc3` file, physics, two textures and one motion of its own.
fn model_bundle() -> Bundle {
    let data = s(vec![
        ("m_Name", text("BuildModelData")),
        ("Moc3FileName", text("M_t01.moc3.bytes")),
        (
            "TextureNames",
            list(vec![
                text("M_t01.2048/texture_00.png"),
                text("M_t01.2048/texture_01.png"),
            ]),
        ),
        ("PhysicsFileName", text("M_t01.physics3.json")),
        ("UserDataFileName", text("")),
        ("AdditionalMotionData", list(vec![entry(20, 21, "w-a01")])),
        ("CategoryRules", list(Vec::new())),
    ]);
    let objects = vec![
        object(1, 114, Some(MODEL_DATA), data),
        file_object(2, b"MOC3\x05rest"),
        file_object(3, b"{\"Version\":3}"),
        texture(4),
        texture(5),
        file_object(6, b"{\"Version\":3,\"FileReferences\":{}}"),
        clip(20, "w-a01"),
        meta(21, 0.5, 0.25),
        script(),
    ];
    let container = [
        ("buildmodeldata.asset", 1),
        ("m_t01.moc3.bytes", 2),
        ("m_t01.physics3.json", 3),
        ("m_t01.2048/texture_00.png", 4),
        ("m_t01.2048/texture_01.png", 5),
        ("m_t01.model3.json", 6),
        ("motion/w-a01.anim", 20),
        ("motion/w-a01.asset", 21),
    ]
    .into_iter()
    .map(|(path, n)| (format!("{DIR}/{path}"), id(n)))
    .collect();
    Bundle { objects, container }
}

/// A motion bundle: two body motions and one facial, the second body motion without metadata.
fn motion_bundle() -> Bundle {
    let data = s(vec![
        ("m_Name", text("BuildMotionData")),
        (
            "Motions",
            list(vec![entry(20, 21, "w-b"), entry(22, 0, "w-a")]),
        ),
        ("Facials", list(vec![entry(24, 25, "face_smile_01")])),
        ("Arms", list(Vec::new())),
        ("Poses", list(Vec::new())),
    ]);
    let objects = vec![
        object(1, 114, Some(MOTION_DATA), data),
        clip(20, "w-b"),
        meta(21, 0.5, 0.5),
        clip(22, "w-a"),
        clip(24, "face_smile_01"),
        meta(25, 0.1, 0.2),
        script(),
    ];
    Bundle {
        objects,
        container: vec![(format!("{DIR}/buildmotiondata.asset"), id(1))],
    }
}

fn convert(converter: &dyn Converter, bundle: &Bundle) -> Result<Vec<Member>, ConvertError> {
    Ok(converter
        .convert(id(1), bundle)?
        .expect("the build data holds an asset")
        .members)
}

fn roles(members: &[Member]) -> Vec<&str> {
    members.iter().map(|m| m.role.as_str()).collect()
}

fn bytes(member: &Member) -> (&str, &[u8]) {
    match &member.content {
        Content::File { extension, bytes } => (extension, bytes),
        other => panic!("not a file: {other:?}"),
    }
}

fn motion(member: &Member) -> MotionDocument {
    let (extension, bytes) = bytes(member);
    assert_eq!(extension, "json");
    MotionDocument::from_bytes(bytes).expect("a motion document")
}

fn code(result: Result<Vec<Member>, ConvertError>) -> &'static str {
    result.expect_err("the conversion fails").code
}

#[test]
fn a_model_holds_its_moc_physics_textures_and_motions_in_order() {
    let bundle = model_bundle();
    let members = convert(&Model, &bundle).unwrap();
    assert_eq!(
        roles(&members),
        ["moc", "physics", "texture/0", "texture/1", "motion/w-a01"]
    );
    assert_eq!(bytes(&members[0]), ("moc3", &b"MOC3\x05rest"[..]));
    assert_eq!(bytes(&members[1]), ("json", &b"{\"Version\":3}"[..]));
    let Content::Image(surface) = &members[2].content else {
        panic!("a texture is an image")
    };
    assert_eq!((surface.width(), surface.height()), (2, 1));
    let document = motion(&members[4]);
    assert_eq!(
        (document.fade_in, document.fade_out),
        (Some(0.5), Some(0.25))
    );
    assert_eq!(document.clip.name, "w-a01");
}

#[test]
fn a_model_declares_the_files_it_finds_by_name() {
    let bundle = model_bundle();
    assert_eq!(Model.inputs(id(1), &bundle), [id(2), id(3), id(4), id(5)]);
}

#[test]
fn a_model_without_physics_has_no_physics_member() {
    let mut bundle = model_bundle();
    bundle.replace(1, |o| set(&mut o.fields, "PhysicsFileName", text("")));
    let members = convert(&Model, &bundle).unwrap();
    assert_eq!(
        roles(&members),
        ["moc", "texture/0", "texture/1", "motion/w-a01"]
    );
}

#[test]
fn files_are_found_without_regard_to_case_and_must_exist_with_their_class() {
    let mut bundle = model_bundle();
    bundle.replace(1, |o| {
        set(&mut o.fields, "Moc3FileName", text("missing.moc3.bytes"))
    });
    assert_eq!(code(convert(&Model, &bundle)), "missing_object");

    let mut bundle = model_bundle();
    bundle.replace(1, |o| {
        set(
            &mut o.fields,
            "Moc3FileName",
            text("M_t01.2048/texture_00.png"),
        )
    });
    assert_eq!(code(convert(&Model, &bundle)), "malformed");

    let mut bundle = model_bundle();
    bundle.replace(2, |o| {
        set(&mut o.fields, "m_Script", Value::Bytes(b"MOC2".to_vec()))
    });
    assert_eq!(code(convert(&Model, &bundle)), "malformed");
}

#[test]
fn model_data_the_form_does_not_define_is_unsupported() {
    let mut bundle = model_bundle();
    bundle.replace(1, |o| {
        set(&mut o.fields, "UserDataFileName", text("M.userdata3.json"))
    });
    assert_eq!(code(convert(&Model, &bundle)), "unsupported");

    let mut bundle = model_bundle();
    bundle.replace(1, |o| {
        set(&mut o.fields, "CategoryRules", list(vec![text("x")]))
    });
    assert_eq!(code(convert(&Model, &bundle)), "unsupported");
}

#[test]
fn a_motion_set_lists_body_motions_then_facials_in_the_game_order() {
    let bundle = motion_bundle();
    let members = convert(&Motions, &bundle).unwrap();
    assert_eq!(
        roles(&members),
        ["motion/w-b", "motion/w-a", "facial/face_smile_01"]
    );
    assert_eq!(motion(&members[0]).fade_in, Some(0.5));
    let without = motion(&members[1]);
    assert_eq!((without.fade_in, without.fade_out), (None, None));
    assert_eq!(motion(&members[2]).fade_out, Some(0.2));
    let (_, json) = bytes(&members[1]);
    assert!(!std::str::from_utf8(json).unwrap().contains("fade"));
}

#[test]
fn motion_sets_reject_arms_poses_and_repeated_names() {
    let mut bundle = motion_bundle();
    bundle.replace(1, |o| {
        set(&mut o.fields, "Poses", list(vec![entry(20, 21, "p")]))
    });
    assert_eq!(code(convert(&Motions, &bundle)), "unsupported");

    let mut bundle = motion_bundle();
    bundle.replace(1, |o| {
        set(
            &mut o.fields,
            "Motions",
            list(vec![entry(20, 21, "w-a"), entry(22, 0, "w-a")]),
        )
    });
    assert_eq!(code(convert(&Motions, &bundle)), "malformed");
}

#[test]
fn an_entry_with_a_null_clip_gives_no_member() {
    let mut bundle = motion_bundle();
    bundle.replace(1, |o| {
        set(&mut o.fields, "Facials", list(vec![entry(0, 0, "f")]))
    });
    let members = convert(&Motions, &bundle).unwrap();
    assert!(!members.iter().any(|m| m.role == "facial/f"));
}

#[test]
fn a_clip_document_keeps_the_stored_curves_bindings_and_events() {
    let bundle = motion_bundle();
    let members = convert(&Motions, &bundle).unwrap();
    let clip = motion(&members[0]).clip;
    assert_eq!(
        (
            clip.sample_rate,
            clip.start_time,
            clip.stop_time,
            clip.looping
        ),
        (60.0, 0.0, 1.5, true)
    );
    let curves = &clip.curves;
    assert_eq!(
        curves.iter().map(|c| c.path).collect::<Vec<_>>(),
        [11, 12, 13, 14]
    );
    assert!(
        curves
            .iter()
            .all(|c| c.attribute == PARAMETER_ATTRIBUTE && c.class_id == 114)
    );
    assert_eq!(
        curves[0].script.as_deref(),
        Some("Live2D.Cubism/Live2D.Cubism.Core.CubismParameter")
    );
    assert_eq!(
        curves[0].data,
        CurveData::Streamed {
            segments: vec![[-f32::MAX, 0.0, 0.0, 0.0, 1.0], [0.5, 1.0, -2.0, 0.25, 1.0]]
        }
    );
    assert_eq!(
        curves[1].data,
        CurveData::Streamed {
            segments: vec![[-f32::MAX, 0.0, 0.0, 2.0, -1.0]]
        }
    );
    assert_eq!(
        curves[2].data,
        CurveData::Dense {
            begin_time: 0.25,
            sample_rate: 30.0,
            samples: vec![1.0, 2.0, 4.0]
        }
    );
    assert_eq!(curves[3].data, CurveData::Constant { value: 0.1 });
    assert_eq!(clip.events.len(), 1);
    assert_eq!(
        (
            clip.events[0].function.as_str(),
            clip.events[0].data.as_str(),
            clip.events[0].int
        ),
        ("OnLive2DInvokeUserData", "eyeblink,0.2", 2)
    );
}

#[test]
fn a_motion_document_has_pinned_bytes() {
    let mut bundle = motion_bundle();
    bundle.replace(22, |o| {
        let muscle = get(&mut o.fields, "m_MuscleClip");
        let data = get(get(muscle, "m_Clip"), "data");
        set(
            get(data, "m_StreamedClip"),
            "data",
            words(&[(-f32::MAX, &[(0, [0.0, 0.0, 0.5, 1.0])])]),
        );
        set(get(data, "m_StreamedClip"), "curveCount", u(1));
        set(get(data, "m_DenseClip"), "m_CurveCount", u(0));
        set(get(data, "m_DenseClip"), "m_SampleArray", list(Vec::new()));
        set(
            get(&mut o.fields, "m_ClipBindingConstant"),
            "genericBindings",
            list(vec![binding(11, 114), binding(14, 114)]),
        );
        set(&mut o.fields, "m_Events", list(Vec::new()));
    });
    let members = convert(&Motions, &bundle).unwrap();
    let (_, json) = bytes(&members[1]);
    assert_eq!(
        std::str::from_utf8(json).unwrap(),
        r#"{"format":"live2d-motion","version":1,"clip":{"format":"clip","version":2,"name":"w-a","sample_rate":60.0,"start_time":0.0,"stop_time":1.5,"loop":true,"curves":[{"path":11,"attribute":3702945584,"class_id":114,"script":"Live2D.Cubism/Live2D.Cubism.Core.CubismParameter","custom_type":0,"component":0,"kind":"streamed","segments":[[-3.4028235e+38,0.0,0.0,0.5,1.0]]},{"path":14,"attribute":3702945584,"class_id":114,"script":"Live2D.Cubism/Live2D.Cubism.Core.CubismParameter","custom_type":0,"component":0,"kind":"constant","value":0.1}],"events":[]}}"#
    );
}

/// Converts the motion set after changing its first clip.
fn with_first_clip(edit: impl FnOnce(&mut Value)) -> Result<Vec<Member>, ConvertError> {
    let mut bundle = motion_bundle();
    bundle.replace(20, |o| edit(&mut o.fields));
    convert(&Motions, &bundle)
}

fn streamed_data(fields: &mut Value) -> &mut Value {
    let data = get(get(get(fields, "m_MuscleClip"), "m_Clip"), "data");
    get(data, "m_StreamedClip")
}

#[test]
fn clips_the_document_does_not_define_are_unsupported() {
    assert_eq!(
        code(with_first_clip(|c| set(
            c,
            "m_Compressed",
            Value::Bool(true)
        ))),
        "unsupported"
    );
    assert_eq!(
        code(with_first_clip(|c| set(
            c,
            "m_FloatCurves",
            list(vec![i(1)])
        ))),
        "unsupported"
    );
    assert_eq!(
        code(with_first_clip(|c| set(
            c,
            "m_HasGenericRootTransform",
            Value::Bool(true)
        ))),
        "unsupported"
    );
    assert_eq!(
        code(with_first_clip(|c| set(
            get(c, "m_MuscleClip"),
            "m_CycleOffset",
            f(0.5)
        ))),
        "unsupported"
    );
    assert_eq!(
        code(with_first_clip(|c| {
            set(
                get(c, "m_ClipBindingConstant"),
                "genericBindings",
                list(vec![
                    binding(11, 4),
                    binding(12, 114),
                    binding(13, 114),
                    binding(14, 114),
                ]),
            )
        })),
        "unsupported"
    );
    assert_eq!(
        code(with_first_clip(|c| {
            let events = get(c, "m_Events");
            let Value::Array(items) = events else {
                panic!()
            };
            set(&mut items[0], "objectReferenceParameter", r(SCRIPT));
        })),
        "unsupported"
    );
}

#[test]
fn damaged_clip_data_is_malformed() {
    // one binding fewer than curves
    assert_eq!(
        code(with_first_clip(|c| {
            set(
                get(c, "m_ClipBindingConstant"),
                "genericBindings",
                list(vec![binding(11, 114), binding(12, 114), binding(13, 114)]),
            )
        })),
        "malformed"
    );
    // frame times that do not increase
    assert_eq!(
        code(with_first_clip(|c| {
            set(
                streamed_data(c),
                "data",
                words(&[(0.5, &[(0, [0.0; 4])]), (0.5, &[(1, [0.0; 4])])]),
            )
        })),
        "malformed"
    );
    // a key for a curve that does not exist
    assert_eq!(
        code(with_first_clip(|c| {
            set(streamed_data(c), "data", words(&[(0.0, &[(2, [0.0; 4])])]))
        })),
        "malformed"
    );
    // a coefficient that is not finite
    assert_eq!(
        code(with_first_clip(|c| {
            set(
                streamed_data(c),
                "data",
                words(&[(0.0, &[(0, [f32::NAN, 0.0, 0.0, 0.0])])]),
            )
        })),
        "malformed"
    );
}
