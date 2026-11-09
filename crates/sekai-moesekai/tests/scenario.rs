//! The `scenario` form on objects built in memory.

#![cfg(feature = "scenario")]

use sekai_convert::{
    Chunk, Content, Context, ConvertError, Converter, Object, ObjectId, ObjectIndex, PPtr, Value,
};
use sekai_moesekai::scenario::Scenario;

const SCRIPT: &str = "Assembly-CSharp/Sekai.ScenarioSceneData";

const ROOT: ObjectId = ObjectId {
    bundle: 0,
    file: 0,
    path_id: 1,
};

struct One(Value);

impl Context for One {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        Ok(Object {
            id,
            class_id: 114,
            class: "MonoBehaviour".into(),
            script: Some(SCRIPT.into()),
            fields: self.0.clone(),
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
}

fn s(fields: Vec<(&str, Value)>) -> Value {
    Value::Struct(fields.into_iter().map(|(n, v)| (n.into(), v)).collect())
}

fn text(v: &str) -> Value {
    Value::String(v.into())
}

fn engine_fields() -> Vec<(&'static str, Value)> {
    let null = Value::PPtr(PPtr {
        file_id: 0,
        path_id: 0,
    });
    vec![
        ("m_GameObject", null.clone()),
        ("m_Enabled", Value::UInt(1)),
        (
            "m_Script",
            Value::PPtr(PPtr {
                file_id: 0,
                path_id: 9,
            }),
        ),
        ("m_Name", text("001_01")),
    ]
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
fn scene_data_of_the_scenario_script_are_the_roots() {
    assert!(Scenario.is_root(&index(114), Some(SCRIPT), &[]));
    assert!(!Scenario.is_root(&index(114), Some("Assembly-CSharp/Sekai.Other"), &[]));
    assert!(!Scenario.is_root(&index(114), None, &[]));
    assert!(!Scenario.is_root(&index(49), Some(SCRIPT), &[]));
}

#[test]
fn the_document_keeps_the_stored_fields_in_order() {
    let mut fields = engine_fields();
    fields.extend([
        ("ScenarioId", text("001_01")),
        (
            "Talk",
            Value::Array(vec![s(vec![
                ("Body", text("「いい？」\n\"quoted\" \\ \u{1}")),
                ("Volume", Value::Float(f64::from(0.1f32))),
                ("Delay", Value::Float(0.25)),
                ("Ratio", Value::Float(0.1)),
                ("Id", Value::Int(-3)),
                ("Flag", Value::Bool(true)),
            ])]),
        ),
        ("Empty", Value::Array(Vec::new())),
        ("Big", Value::UInt(u64::MAX)),
    ]);
    let asset = Scenario.convert(ROOT, &One(s(fields))).unwrap().unwrap();
    assert_eq!(asset.members.len(), 1);
    assert_eq!(asset.members[0].role, "scenario");
    let Content::File { extension, bytes } = &asset.members[0].content else {
        panic!("not a file");
    };
    assert_eq!(extension, "json");
    assert_eq!(
        std::str::from_utf8(bytes).unwrap(),
        "{\"ScenarioId\":\"001_01\",\"Talk\":[{\"Body\":\"「いい？」\\n\\\"quoted\\\" \\\\ \\u0001\",\
         \"Volume\":0.1,\"Delay\":0.25,\"Ratio\":0.1,\"Id\":-3,\"Flag\":true}],\"Empty\":[],\
         \"Big\":18446744073709551615}"
    );
}

#[test]
fn references_bytes_and_non_finite_numbers_are_errors() {
    let reference = Value::PPtr(PPtr {
        file_id: 0,
        path_id: 4,
    });
    for (value, code) in [
        (reference, "unsupported"),
        (Value::Bytes(vec![0xFF]), "unsupported"),
        (Value::Float(f64::NAN), "malformed"),
    ] {
        let mut fields = engine_fields();
        fields.push(("Field", value));
        let e = Scenario.convert(ROOT, &One(s(fields))).unwrap_err();
        assert_eq!(e.code, code);
    }
}
