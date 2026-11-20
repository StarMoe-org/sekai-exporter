//! The `virtual-live-script` and `virtual-live-timeline` forms on objects built in memory.

#![cfg(feature = "virtual-live")]

use sekai_convert::{
    Chunk, Content, Context, ConvertError, Converter, Object, ObjectId, ObjectIndex, PPtr, Value,
};
use sekai_moesekai::virtual_live::{VirtualLiveScript, VirtualLiveTimeline};

const SCRIPT: &str = "Assembly-CSharp/Sekai.Core.VirtualLive.MasterOfCeremonyData";

const ROOT: ObjectId = ObjectId {
    bundle: 0,
    file: 0,
    path_id: 1,
};

struct Script;

impl Context for Script {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        Ok(Object {
            id,
            class_id: 114,
            class: "MonoBehaviour".into(),
            script: Some(SCRIPT.into()),
            fields: Value::Struct(vec![
                ("m_Name".into(), Value::String("mc_1".into())),
                ("Id".into(), Value::String("mc_1".into())),
                ("characterTalkEvents".into(), Value::Array(Vec::new())),
            ]),
        })
    }

    fn resolve(&self, _: ObjectId, _: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        Ok(None)
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
fn the_script_objects_are_the_roots() {
    let root = |class_id, script| VirtualLiveScript.is_root(&index(class_id), script, &[]);
    assert!(root(114, Some(SCRIPT)));
    assert!(!root(114, Some("Assembly-CSharp/Sekai.ScenarioSceneData")));
    assert!(!root(49, Some(SCRIPT)));
}

#[test]
fn the_script_is_its_fields_without_the_engine_ones() {
    let asset = VirtualLiveScript.convert(ROOT, &Script).unwrap().unwrap();
    let [member] = asset.members.as_slice() else {
        panic!("one member");
    };
    assert_eq!(member.role, "script");
    let Content::File { extension, bytes } = &member.content else {
        panic!("a file");
    };
    assert_eq!(extension, "json");
    assert_eq!(bytes, br#"{"Id":"mc_1","characterTalkEvents":[]}"#);
}

const TIMELINE_ASSET: &str = "Unity.Timeline/UnityEngine.Timeline.TimelineAsset";
const TIMELINE_PATH: &str =
    "assets/sekai/assetbundle/resources/ondemand/virtual_live/mc/timeline/mc_1/mc_1.playable";

/// One bundle with one file, holding the given objects.
struct Bundle(Vec<Object>);

impl Context for Bundle {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        let object = self.0.iter().find(|o| o.id == id).cloned();
        object.ok_or_else(|| ConvertError::new("missing_object", "no such object"))
    }

    fn resolve(&self, _: ObjectId, reference: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        if reference.path_id == 0 {
            return Ok(None);
        }
        self.object(id(reference.path_id)).map(|o| Some(o.id))
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

fn id(path_id: i64) -> ObjectId {
    ObjectId {
        bundle: 0,
        file: 0,
        path_id,
    }
}

fn r(path_id: i64) -> Value {
    Value::PPtr(PPtr {
        file_id: 0,
        path_id,
    })
}

fn text(v: &str) -> Value {
    Value::String(v.into())
}

/// A `MonoBehaviour` of `script`, which object 9 is, with its name and further fields.
fn behaviour(path_id: i64, script: &str, name: &str, fields: Vec<(&str, Value)>) -> Object {
    let engine = [
        ("m_GameObject", r(0)),
        ("m_Enabled", Value::UInt(1)),
        ("m_Script", r(9)),
        ("m_Name", text(name)),
    ];
    let fields = engine.into_iter().chain(fields);
    Object {
        id: id(path_id),
        class_id: 114,
        class: "MonoBehaviour".into(),
        script: Some(script.into()),
        fields: Value::Struct(fields.map(|(n, v)| (n.into(), v)).collect()),
    }
}

/// A timeline with a group of one talk track, whose one clip names the build data of the
/// performers, and the script they all refer to.
fn timeline() -> Bundle {
    let clip = Value::Struct(vec![
        ("m_Start".into(), Value::Float(14.5)),
        ("m_Asset".into(), r(4)),
        ("m_Duration".into(), Value::Float(2.799_999_952_316_284)),
        ("m_ParentTrack".into(), r(3)),
    ]);
    Bundle(vec![
        behaviour(
            1,
            TIMELINE_ASSET,
            "mc_1",
            vec![("m_Tracks", Value::Array(vec![r(2)]))],
        ),
        behaviour(
            2,
            "Unity.Timeline/UnityEngine.Timeline.GroupTrack",
            "Miku",
            vec![("m_Parent", r(1)), ("m_Children", Value::Array(vec![r(3)]))],
        ),
        behaviour(
            3,
            "Assembly-CSharp/Sekai.MCTimeline.MCTimelineCharacterTalkTrack",
            "Miku_Talk",
            vec![
                ("m_Parent", r(2)),
                ("m_Clips", Value::Array(vec![clip])),
                ("CharacterId", Value::Int(21)),
            ],
        ),
        behaviour(
            4,
            "Assembly-CSharp/Sekai.MCTimeline.CharacterTalkClip",
            "CharacterTalkClip",
            vec![
                ("CueName", text("voice_mc_1_01")),
                ("Serif", text("こんにちは！\n")),
                ("Character", r(0)),
                ("BuiledData", r(5)),
            ],
        ),
        behaviour(
            5,
            "Assembly-CSharp/Sekai.VirtualLive.VirtualLivePerformerBundleBuildData",
            "VirtualLivePerformerBundleBuildData",
            vec![("standingPositionList", Value::Array(Vec::new()))],
        ),
        Object {
            id: id(9),
            class_id: 115,
            class: "MonoScript".into(),
            script: None,
            fields: Value::Struct(vec![
                ("m_Name".into(), text("TimelineAsset")),
                ("m_ClassName".into(), text("TimelineAsset")),
                ("m_Namespace".into(), text("UnityEngine.Timeline")),
                ("m_AssemblyName".into(), text("Unity.Timeline")),
            ]),
        },
    ])
}

#[test]
fn the_timelines_of_virtual_lives_are_the_roots() {
    let root = |class_id, script, path: &str| {
        VirtualLiveTimeline.is_root(&index(class_id), script, &[path.to_owned()])
    };
    assert!(root(114, Some(TIMELINE_ASSET), TIMELINE_PATH));
    assert!(root(
        114,
        Some(TIMELINE_ASSET),
        &TIMELINE_PATH.to_uppercase()
    ));
    let other = "assets/sekai/assetbundle/resources/ondemand/live_pv/timeline/0001/0001.playable";
    assert!(!root(114, Some(TIMELINE_ASSET), other));
    assert!(!root(114, Some(SCRIPT), TIMELINE_PATH));
    assert!(!root(1, Some(TIMELINE_ASSET), TIMELINE_PATH));
}

#[test]
fn a_timeline_lists_every_object_it_reaches_with_references_as_positions() {
    let asset = VirtualLiveTimeline
        .convert(id(1), &timeline())
        .unwrap()
        .unwrap();
    let [member] = asset.members.as_slice() else {
        panic!("one member");
    };
    assert_eq!(member.role, "timeline");
    let Content::File { extension, bytes } = &member.content else {
        panic!("a file");
    };
    assert_eq!(extension, "json");
    let behaviour = |path_id: i64, script: &str, name: &str, fields: &str| {
        format!(
            concat!(
                r#"{{"file":"CAB-test","id":"{}","class":"MonoBehaviour","class_id":114,"#,
                r#""script":"{}","name":"{}","fields":{{"m_GameObject":null,"m_Enabled":1,"#,
                r#""m_Script":{{"$ref":1}},"m_Name":"{}",{}}}}}"#,
            ),
            path_id, script, name, name, fields
        )
    };
    let objects = [
        behaviour(1, TIMELINE_ASSET, "mc_1", r#""m_Tracks":[{"$ref":2}]"#),
        concat!(
            r#"{"file":"CAB-test","id":"9","class":"MonoScript","class_id":115,"#,
            r#""name":"Unity.Timeline/UnityEngine.Timeline.TimelineAsset"}"#,
        )
        .to_owned(),
        behaviour(
            2,
            "Unity.Timeline/UnityEngine.Timeline.GroupTrack",
            "Miku",
            r#""m_Parent":{"$ref":0},"m_Children":[{"$ref":3}]"#,
        ),
        behaviour(
            3,
            "Assembly-CSharp/Sekai.MCTimeline.MCTimelineCharacterTalkTrack",
            "Miku_Talk",
            concat!(
                r#""m_Parent":{"$ref":2},"m_Clips":[{"m_Start":14.5,"m_Asset":{"$ref":4},"#,
                r#""m_Duration":2.8,"m_ParentTrack":{"$ref":3}}],"CharacterId":21"#,
            ),
        ),
        behaviour(
            4,
            "Assembly-CSharp/Sekai.MCTimeline.CharacterTalkClip",
            "CharacterTalkClip",
            concat!(
                r#""CueName":"voice_mc_1_01","Serif":"こんにちは！\n","Character":null,"#,
                r#""BuiledData":{"$ref":5}"#,
            ),
        ),
        behaviour(
            5,
            "Assembly-CSharp/Sekai.VirtualLive.VirtualLivePerformerBundleBuildData",
            "VirtualLivePerformerBundleBuildData",
            r#""standingPositionList":[]"#,
        ),
    ];
    let expected = format!(
        r#"{{"format":"virtual-live-timeline","version":1,"objects":[{}]}}"#,
        objects.join(",")
    );
    assert_eq!(std::str::from_utf8(bytes).unwrap(), expected);
}

#[test]
fn a_timeline_that_reaches_a_missing_object_is_an_error() {
    let Bundle(mut objects) = timeline();
    objects.retain(|object| object.id != id(5));
    let error = VirtualLiveTimeline
        .convert(id(1), &Bundle(objects))
        .unwrap_err();
    assert_eq!(error.code, "missing_object");
}
