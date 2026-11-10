//! The `text` and `font` forms on objects built in memory.

#![cfg(feature = "text")]

use sekai_convert::text::{Font, Text};
use sekai_convert::{
    Chunk, Content, Context, ConvertError, Converter, Cost, CostClass, Object, ObjectId,
    ObjectIndex, PPtr, Value,
};

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

    fn resolve(&self, _from: ObjectId, _reference: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        Ok(None)
    }

    fn resource(&self, _: ObjectId, path: &str, _: u64, _: u64) -> Result<Chunk<'_>, ConvertError> {
        Err(ConvertError::new("missing_resource", path))
    }

    fn container(&self, _from: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        Ok(self.container.clone())
    }

    fn file_name(&self, _from: ObjectId) -> Result<&str, ConvertError> {
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

fn object(path_id: i64, class_id: i32, class: &str, fields: Vec<(&str, Value)>) -> Object {
    Object {
        id: id(path_id),
        class_id,
        class: class.into(),
        script: None,
        fields: Value::Struct(fields.into_iter().map(|(n, v)| (n.into(), v)).collect()),
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

fn file(content: &Content) -> (&str, &[u8]) {
    match content {
        Content::File { extension, bytes } => (extension, bytes),
        other => panic!("not a file: {other:?}"),
    }
}

const PREFIX: &str = "assets/sekai/assetbundle/resources/ondemand/";

#[test]
fn text_assets_are_roots_unless_they_hold_cri_media() {
    let text = index(49);
    let paths = |p: &str| vec![format!("{PREFIX}{p}")];
    assert!(Text.is_root(&text, None, &paths("a/b.atlas.txt")));
    assert!(Text.is_root(&text, None, &[]));
    assert!(!Text.is_root(&text, None, &paths("sound/se.acb.bytes")));
    assert!(!Text.is_root(&text, None, &paths("sound/se.ACB.bytes")));
    assert!(!Text.is_root(&text, None, &paths("movie/m-001.usm.bytes")));
    assert!(!Text.is_root(&index(28), None, &paths("a.json")));
}

#[test]
fn a_text_asset_keeps_its_bytes_and_the_extension_of_its_path() {
    let bytes = vec![0xde, 0x4e, 0xc1, 0x62, 0xff, 0x00];
    let bundle = Bundle {
        objects: vec![
            object(
                1,
                49,
                "TextAsset",
                vec![
                    ("m_Name", Value::String("skeleton.skel".into())),
                    ("m_Script", Value::Bytes(bytes.clone())),
                ],
            ),
            object(
                2,
                49,
                "TextAsset",
                vec![
                    ("m_Name", Value::String("data".into())),
                    ("m_Script", Value::String("{\"a\": 1}".into())),
                ],
            ),
        ],
        container: vec![
            (format!("{PREFIX}x/skeleton.skel.bytes"), id(1)),
            (format!("{PREFIX}x/data.json"), id(2)),
        ],
    };
    let asset = Text.convert(id(1), &bundle).unwrap().unwrap();
    assert_eq!(asset.members.len(), 1);
    assert_eq!(asset.members[0].role, "file");
    assert_eq!(file(&asset.members[0].content), ("bytes", &bytes[..]));
    let asset = Text.convert(id(2), &bundle).unwrap().unwrap();
    assert_eq!(
        file(&asset.members[0].content),
        ("json", &b"{\"a\": 1}"[..])
    );
}

#[test]
fn a_text_asset_outside_the_container_is_bytes() {
    let bundle = Bundle {
        objects: vec![object(
            1,
            49,
            "TextAsset",
            vec![("m_Script", Value::String("x".into()))],
        )],
        container: Vec::new(),
    };
    let asset = Text.convert(id(1), &bundle).unwrap().unwrap();
    assert_eq!(file(&asset.members[0].content), ("bytes", &b"x"[..]));
}

#[test]
fn a_text_asset_without_contents_is_malformed() {
    let bundle = Bundle {
        objects: vec![object(1, 49, "TextAsset", vec![])],
        container: Vec::new(),
    };
    assert_eq!(Text.convert(id(1), &bundle).unwrap_err().code, "malformed");
}

fn font(data: &[u8]) -> Bundle {
    Bundle {
        objects: vec![object(
            1,
            128,
            "Font",
            vec![
                ("m_Name", Value::String("f".into())),
                ("m_FontData", Value::Bytes(data.to_vec())),
            ],
        )],
        container: Vec::new(),
    }
}

#[test]
fn fonts_are_written_by_their_signature() {
    assert!(Font.is_root(&index(128), None, &[]));
    let cases: [(&[u8], &str); 3] = [
        (b"OTTO\0\x0a\0\x80", "otf"),
        (b"\0\x01\0\0\0\x0a\0\x80", "ttf"),
        (b"true\0\x0a\0\x80", "ttf"),
    ];
    for (data, extension) in cases {
        let asset = Font.convert(id(1), &font(data)).unwrap().unwrap();
        assert_eq!(asset.members[0].role, "font");
        assert_eq!(file(&asset.members[0].content), (extension, data));
    }
}

#[test]
fn a_font_without_data_is_no_asset() {
    assert!(Font.convert(id(1), &font(&[])).unwrap().is_none());
}

#[test]
fn other_font_data_is_unsupported() {
    let e = Font.convert(id(1), &font(b"wOFF\0\0")).unwrap_err();
    assert_eq!(e.code, "unsupported");
}

#[test]
fn text_has_the_default_cost() {
    let bundle = Bundle {
        objects: Vec::new(),
        container: Vec::new(),
    };
    let mut object = index(49);
    object.size = 250;
    assert_eq!(
        Text.cost(id(1), &object, &bundle),
        Cost {
            class: CostClass::Light,
            memory: 250 * Cost::PAYLOAD_FACTOR,
        }
    );
    assert_eq!(Cost::light(u64::MAX).memory, u64::MAX);
}
