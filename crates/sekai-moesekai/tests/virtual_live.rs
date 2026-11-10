//! The `virtual-live-script` form on an object built in memory.

#![cfg(feature = "virtual-live")]

use sekai_convert::{
    Chunk, Content, Context, ConvertError, Converter, Object, ObjectId, ObjectIndex, PPtr, Value,
};
use sekai_moesekai::virtual_live::VirtualLiveScript;

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
