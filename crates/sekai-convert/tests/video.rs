//! The `video` form on USM files built here.

#![cfg(feature = "video")]

use sekai_convert::video::Video;
use sekai_convert::{
    Chunk, Content, Context, ConvertError, Converter, Cost, CostClass, Object, ObjectId,
    ObjectIndex, PPtr, Value,
};

fn id(path_id: i64) -> ObjectId {
    ObjectId {
        bundle: 0,
        file: 0,
        path_id,
    }
}

/// One bundle holding `TextAsset` objects at the given container paths.
struct Bundle(Vec<(String, Vec<u8>)>);

impl Context for Bundle {
    fn object(&self, object: ObjectId) -> Result<Object, ConvertError> {
        let (_, bytes) = &self.0[object.path_id as usize];
        Ok(Object {
            id: object,
            class_id: 49,
            class: "TextAsset".into(),
            script: None,
            fields: Value::Struct(vec![("m_Script".into(), Value::Bytes(bytes.clone()))]),
        })
    }

    fn resolve(&self, _: ObjectId, _: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        Ok(None)
    }

    fn resource(&self, _: ObjectId, path: &str, _: u64, _: u64) -> Result<Chunk<'_>, ConvertError> {
        Err(ConvertError::new("missing_resource", path))
    }

    fn container(&self, _: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        Ok(self
            .0
            .iter()
            .enumerate()
            .map(|(i, (path, _))| (path.clone(), id(i as i64)))
            .collect())
    }
}

/// An `@UTF` table with one row of 32-bit columns.
fn utf(name: &str, columns: &[(&str, u32)]) -> Vec<u8> {
    let mut strings = b"<NULL>\0".to_vec();
    let text = |value: &str, strings: &mut Vec<u8>| {
        let offset = strings.len() as u32;
        strings.extend(value.as_bytes());
        strings.push(0);
        offset
    };
    let name_offset = text(name, &mut strings);
    let mut schema = Vec::new();
    let mut body = Vec::new();
    for (column, value) in columns {
        schema.push(0x54);
        schema.extend(text(column, &mut strings).to_be_bytes());
        body.extend(value.to_be_bytes());
    }
    let rows_offset = 0x18 + schema.len();
    let strings_offset = rows_offset + body.len();
    let data_offset = strings_offset + strings.len();

    let mut out = b"@UTF".to_vec();
    out.extend((data_offset as u32).to_be_bytes());
    out.extend(1u16.to_be_bytes());
    out.extend((rows_offset as u16).to_be_bytes());
    out.extend((strings_offset as u32).to_be_bytes());
    out.extend((data_offset as u32).to_be_bytes());
    out.extend(name_offset.to_be_bytes());
    out.extend((columns.len() as u16).to_be_bytes());
    out.extend((body.len() as u16).to_be_bytes());
    out.extend(1u32.to_be_bytes());
    out.extend(schema);
    out.extend(body);
    out.extend(strings);
    out
}

fn chunk(kind: &[u8; 4], data_type: u8, time: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = kind.to_vec();
    out.extend((payload.len() as u32 + 0x18).to_be_bytes());
    out.extend([0, 0x18, 0, 0, 0, 0, 0, data_type]);
    out.extend(time.to_be_bytes());
    out.extend(3000u32.to_be_bytes());
    out.extend([0; 8]);
    out.extend(payload);
    out
}

const FIRST: &[u8] = &[0, 0, 1, 0xB3, 1, 2];
const SECOND: &[u8] = &[0, 0, 1, 0, 3];

fn movie(codec: u32, first: &[u8]) -> Vec<u8> {
    let video = utf(
        "VIDEO_HDRINFO",
        &[
            ("mpeg_codec", codec),
            ("width", 640),
            ("height", 360),
            ("framerate_n", 30000),
            ("framerate_d", 1000),
        ],
    );
    let seek = utf("VIDEO_SEEKINFO", &[("ofs_frmid", 0)]);
    let directory = utf("CRIUSF_DIR_STREAM", &[("fmtver", 1)]);
    [
        chunk(b"CRID", 1, 0, &directory),
        chunk(b"@SFV", 1, 0, &video),
        chunk(b"@SFV", 3, 0, &seek),
        chunk(b"@SFV", 2, 0, b"#HEADER END"),
        chunk(b"@SFV", 0, 0, first),
        chunk(b"@SFV", 0, 100, SECOND),
    ]
    .concat()
}

fn movie_of(asset: &sekai_convert::Asset) -> &sekai_convert::Movie {
    let [member] = asset.members.as_slice() else {
        panic!("one member is expected");
    };
    assert_eq!(member.role, "video");
    let Content::Video(movie) = &member.content else {
        panic!("the member is not a video");
    };
    movie
}

#[test]
fn a_file_becomes_a_movie() {
    let bundle = Bundle(vec![("a/m.usm.bytes".into(), movie(1, FIRST))]);
    let asset = Video.convert(id(0), &bundle).unwrap().unwrap();
    let movie = movie_of(&asset);
    assert_eq!(movie.stream, [FIRST, SECOND].concat());
    assert_eq!((movie.width, movie.height), (640, 360));
    assert_eq!((movie.framerate_n, movie.framerate_d), (30000, 1000));
    assert_eq!(movie.audio, None);
}

#[test]
fn segments_are_joined_in_number_order() {
    let data = movie(1, FIRST);
    let (head, tail) = data.split_at(data.len() / 2);
    let bundle = Bundle(vec![
        ("a/m-002.usm.bytes".into(), tail.to_vec()),
        ("a/m-001.usm.bytes".into(), head.to_vec()),
        ("a/other-001.usm.bytes".into(), vec![1, 2, 3]),
    ]);
    let asset = Video.convert(id(1), &bundle).unwrap().unwrap();
    assert_eq!(movie_of(&asset).stream, [FIRST, SECOND].concat());
}

#[test]
fn a_missing_segment_is_an_error() {
    let data = movie(1, FIRST);
    let bundle = Bundle(vec![
        ("a/m-001.usm.bytes".into(), data),
        ("a/m-003.usm.bytes".into(), Vec::new()),
    ]);
    let error = Video.convert(id(0), &bundle).unwrap_err();
    assert_eq!(error.code, "missing_segment");
}

#[test]
fn only_the_first_segment_is_a_root() {
    let object = ObjectIndex {
        path_id: 1,
        class_id: 49,
        class: "TextAsset".into(),
        name: None,
        script: None,
        script_name: None,
        references: Vec::new(),
        size: 0,
        sha256: [0; 32],
        error: None,
    };
    let root = |path: &str| Video.is_root(&object, None, &[path.to_owned()]);
    assert!(root("a/m.usm.bytes"));
    assert!(root("a/m-001.usm.bytes"));
    assert!(!root("a/m-002.usm.bytes"));
    assert!(!root("a/m.acb.bytes"));
}

#[test]
fn other_streams_are_named_errors() {
    let vp9 = Bundle(vec![("a/m.usm.bytes".into(), movie(9, FIRST))]);
    assert_eq!(Video.convert(id(0), &vp9).unwrap_err().code, "unsupported");
    let scrambled = Bundle(vec![("a/m.usm.bytes".into(), movie(1, &[9, 9, 9, 9]))]);
    assert_eq!(
        Video.convert(id(0), &scrambled).unwrap_err().code,
        "unsupported"
    );
}

#[test]
fn the_cost_of_a_movie_counts_its_segments() {
    let object = ObjectIndex {
        path_id: 1,
        class_id: 49,
        class: "TextAsset".into(),
        name: None,
        script: None,
        script_name: None,
        references: Vec::new(),
        size: 1000,
        sha256: [0; 32],
        error: None,
    };
    let bundle = Bundle(vec![
        ("a/m-002.usm.bytes".into(), Vec::new()),
        ("a/m-001.usm.bytes".into(), Vec::new()),
        ("a/m-003.usm.bytes".into(), Vec::new()),
        ("a/other-001.usm.bytes".into(), Vec::new()),
        ("a/single.usm.bytes".into(), Vec::new()),
    ]);
    let wide = |memory| Cost {
        class: CostClass::Wide,
        memory,
    };
    let encoder = 256 << 20;
    assert_eq!(
        Video.cost(id(1), &object, &bundle),
        wide(4 * 3 * 1000 + encoder)
    );
    assert_eq!(
        Video.cost(id(4), &object, &bundle),
        wide(4 * 1000 + encoder)
    );
}
