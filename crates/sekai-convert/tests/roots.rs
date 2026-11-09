//! Root rules and the converter that stands for one that is not compiled in.

use sekai_convert::roots::{self, Absent};
use sekai_convert::{Chunk, Context, ConvertError, Object, ObjectId, ObjectIndex, PPtr};

struct NoContext;

impl Context for NoContext {
    fn object(&self, _id: ObjectId) -> Result<Object, ConvertError> {
        unreachable!("an absent converter reads nothing")
    }

    fn resolve(&self, _from: ObjectId, _reference: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        unreachable!("an absent converter reads nothing")
    }

    fn resource(
        &self,
        _from: ObjectId,
        _path: &str,
        _offset: u64,
        _size: u64,
    ) -> Result<Chunk<'_>, ConvertError> {
        unreachable!("an absent converter reads nothing")
    }

    fn container(&self, _from: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        unreachable!("an absent converter reads nothing")
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

const TEXT_ASSET: i32 = 49;

fn paths(path: &str) -> Vec<String> {
    vec![path.to_owned()]
}

#[test]
fn an_absent_converter_selects_the_roots_of_its_form_and_names_itself() {
    let converter = Absent::boxed("texture", roots::texture);
    assert_eq!(converter.name(), "texture");
    assert_eq!(converter.version(), 0);
    assert!(converter.is_root(&index(28), None, &[]));
    assert!(!converter.is_root(&index(TEXT_ASSET), None, &[]));

    let id = ObjectId {
        bundle: 0,
        file: 0,
        path_id: 1,
    };
    let error = converter.convert(id, &NoContext).unwrap_err();
    assert_eq!(error.code, "not_compiled");
    assert!(error.message.contains("texture"));
}

#[test]
fn text_assets_go_to_exactly_one_form() {
    let object = index(TEXT_ASSET);
    let claims = |path: &str| {
        let paths = paths(path);
        [
            roots::audio(&object, None, &paths),
            roots::video(&object, None, &paths),
            roots::text(&object, None, &paths),
        ]
    };
    assert_eq!(claims("a/b.acb.bytes"), [true, false, false]);
    assert_eq!(claims("a/b.usm.bytes"), [false, true, false]);
    assert_eq!(claims("a/b-001.usm.bytes"), [false, true, false]);
    assert_eq!(claims("a/b.txt"), [false, false, true]);
}

#[test]
fn a_later_segment_of_a_video_is_not_a_root() {
    let object = index(TEXT_ASSET);
    assert!(!roots::video(&object, None, &paths("a/b-002.usm.bytes")));
}
