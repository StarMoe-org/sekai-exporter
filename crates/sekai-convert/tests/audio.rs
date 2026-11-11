//! The `audio` form on cue sheets built here, holding HCA streams made for these tests.

#![cfg(feature = "audio")]

use sekai_codec::cri::{HcaKey, decode_hca};
use sekai_convert::audio::Audio;
use sekai_convert::{
    Asset, Chunk, Content, Context, ConvertError, Converter, Cost, CostClass, Object, ObjectId,
    ObjectIndex, PPtr, Pcm, Sound, Value,
};
use sekai_format::audio::CueTable;

const KEY: u64 = 0x0123456789ABCDE;
const SUBKEY: u16 = 0x4A1C;

fn hca(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sekai-codec/tests/fixtures/hca")
        .join(name);
    std::fs::read(path).expect("the fixture exists")
}

fn plain(name: &str, key: Option<HcaKey>) -> Pcm {
    decode_hca(&hca(name), key).expect("the fixture decodes")
}

/// One bundle holding one `TextAsset` with the given bytes.
struct Bundle(Vec<u8>);

const ROOT: ObjectId = ObjectId {
    bundle: 0,
    file: 0,
    path_id: 1,
};

impl Context for Bundle {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        assert_eq!(id, ROOT);
        Ok(Object {
            id,
            class_id: 49,
            class: "TextAsset".into(),
            script: None,
            fields: Value::Struct(vec![
                ("m_Name".into(), Value::String("se.acb".into())),
                ("m_Script".into(), Value::Bytes(self.0.clone())),
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
        Ok(vec![("assets/se.acb.bytes".into(), ROOT)])
    }

    fn file_name(&self, _: ObjectId) -> Result<&str, ConvertError> {
        Ok("CAB-test")
    }
}

enum Cell {
    U8(u8),
    U16(u16),
    U32(u32),
    Text(&'static str),
    Data(Vec<u8>),
}

/// An `@UTF` table whose columns all live in the rows.
fn utf(name: &str, columns: &[&str], rows: &[Vec<Cell>]) -> Vec<u8> {
    let mut strings = b"<NULL>\0".to_vec();
    let text = |value: &str, strings: &mut Vec<u8>| {
        let offset = strings.len() as u32;
        strings.extend(value.as_bytes());
        strings.push(0);
        offset
    };
    let name_offset = text(name, &mut strings);
    let mut schema = Vec::new();
    for (index, column) in columns.iter().enumerate() {
        let kind = match &rows[0][index] {
            Cell::U8(_) => 0u8,
            Cell::U16(_) => 2,
            Cell::U32(_) => 4,
            Cell::Text(_) => 10,
            Cell::Data(_) => 11,
        };
        schema.push(0x50 | kind);
        schema.extend(text(column, &mut strings).to_be_bytes());
    }
    let mut body = Vec::new();
    let mut blobs: Vec<u8> = Vec::new();
    for row in rows {
        for cell in row {
            match cell {
                Cell::U8(value) => body.push(*value),
                Cell::U16(value) => body.extend(value.to_be_bytes()),
                Cell::U32(value) => body.extend(value.to_be_bytes()),
                Cell::Text(value) => body.extend(text(value, &mut strings).to_be_bytes()),
                Cell::Data(value) => {
                    body.extend((blobs.len() as u32).to_be_bytes());
                    body.extend((value.len() as u32).to_be_bytes());
                    blobs.extend(value);
                }
            }
        }
    }
    let rows_offset = 0x18 + schema.len();
    let strings_offset = rows_offset + body.len();
    let data_offset = strings_offset + strings.len();
    let size = data_offset + blobs.len();
    let mut out = b"@UTF".to_vec();
    out.extend((size as u32).to_be_bytes());
    out.extend(1u16.to_be_bytes());
    out.extend((rows_offset as u16).to_be_bytes());
    out.extend((strings_offset as u32).to_be_bytes());
    out.extend((data_offset as u32).to_be_bytes());
    out.extend(name_offset.to_be_bytes());
    out.extend((columns.len() as u16).to_be_bytes());
    out.extend(((body.len() / rows.len()) as u16).to_be_bytes());
    out.extend((rows.len() as u32).to_be_bytes());
    out.extend(schema);
    out.extend(body);
    out.extend(strings);
    out.extend(blobs);
    out
}

/// An AFS2 wave bank.
fn afs2(subkey: u16, streams: &[(u16, Vec<u8>)]) -> Vec<u8> {
    const ALIGNMENT: usize = 0x20;
    let mut out = b"AFS2\x01\x04\x02\x00".to_vec();
    out.extend((streams.len() as u32).to_le_bytes());
    out.extend((ALIGNMENT as u16).to_le_bytes());
    out.extend(subkey.to_le_bytes());
    for (id, _) in streams {
        out.extend(id.to_le_bytes());
    }
    let mut body = Vec::new();
    let first = out.len() + (streams.len() + 1) * 4;
    let mut offset = first;
    for (_, data) in streams {
        out.extend((offset as u32).to_le_bytes());
        let start = offset.next_multiple_of(ALIGNMENT);
        body.resize(start - first, 0);
        body.extend(data);
        offset = first + body.len();
    }
    out.extend((offset as u32).to_le_bytes());
    out.extend(body);
    out
}

/// A cue sheet with three waveforms in its memory bank (wave IDs 3, 4 and 5) and the cues
/// `cues`: (reference type, reference index, name). Synth 0 plays waveforms 1, 0 and 1.
fn sheet(cues: &[(u8, u16, &'static str)], streaming: u8) -> Vec<u8> {
    use Cell::{Data, Text, U8, U16, U32};
    let cue_rows: Vec<_> = cues
        .iter()
        .enumerate()
        .map(|(i, &(kind, index, _))| vec![U8(kind), U16(index), U32(10 * i as u32), U32(1000)])
        .collect();
    let name_rows: Vec<_> = cues
        .iter()
        .enumerate()
        .map(|(i, &(_, _, name))| vec![Text(name), U16(i as u16)])
        .collect();
    let wave = |id, channels| {
        vec![
            U8(streaming),
            U16(id),
            U8(2),
            U8(0),
            U8(channels),
            U16(44100),
            U32(44100),
        ]
    };
    utf(
        "Header",
        &[
            "CueTable",
            "CueNameTable",
            "WaveformTable",
            "SynthTable",
            "AwbFile",
        ],
        &[vec![
            Data(utf(
                "Cue",
                &["ReferenceType", "ReferenceIndex", "CueId", "Length"],
                &cue_rows,
            )),
            Data(utf("CueName", &["CueName", "CueIndex"], &name_rows)),
            Data(utf(
                "Waveform",
                &[
                    "Streaming",
                    "MemoryAwbId",
                    "EncodeType",
                    "LoopFlag",
                    "NumChannels",
                    "SamplingRate",
                    "NumSamples",
                ],
                &[wave(3, 1), wave(4, 2), wave(5, 2)],
            )),
            Data(utf(
                "Synth",
                &["ReferenceItems"],
                &[vec![Data(vec![0, 1, 0, 1, 0, 1, 0, 0, 0, 1, 0, 1])]],
            )),
            Data(afs2(
                SUBKEY,
                &[
                    (3, hca("sine_mono.hca")),
                    (4, hca("sine_stereo.hca")),
                    (5, hca("sine_stereo_subkey.hca")),
                ],
            )),
        ]],
    )
}

fn convert(audio: Audio, data: Vec<u8>) -> Result<Option<Asset>, ConvertError> {
    audio.convert(ROOT, &Bundle(data))
}

/// The samples a sound decodes to.
fn decoded(sound: &Sound) -> Pcm {
    let mut samples = Vec::new();
    sound
        .decode_each(|block| samples.extend_from_slice(block))
        .unwrap();
    Pcm::new(sound.sample_rate(), sound.channels(), samples).unwrap()
}

/// Each member but the cue table, with the samples it decodes to.
fn members(asset: &Asset) -> Vec<(&str, Pcm)> {
    asset
        .members
        .iter()
        .filter(|m| m.role != "cues")
        .map(|m| match &m.content {
            Content::Audio(sound) => (m.role.as_str(), decoded(sound)),
            other => panic!("not audio: {other:?}"),
        })
        .collect()
}

fn cue_table(asset: &Asset) -> CueTable {
    let last = asset.members.last().expect("members");
    assert_eq!(last.role, "cues");
    match &last.content {
        Content::File { extension, bytes } => {
            assert_eq!(extension, "json");
            CueTable::from_bytes(bytes).unwrap()
        }
        other => panic!("not a file: {other:?}"),
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
fn cue_sheets_are_the_roots() {
    let audio = Audio::new(None);
    let paths = |p: &str| vec![p.to_owned()];
    assert!(audio.is_root(&index(49), None, &paths("assets/sound/se.acb.bytes")));
    assert!(audio.is_root(&index(49), None, &paths("assets/sound/SE.ACB.bytes")));
    assert!(!audio.is_root(&index(49), None, &paths("assets/sound/se.json")));
    assert!(!audio.is_root(&index(49), None, &[]));
    assert!(!audio.is_root(&index(83), None, &paths("assets/sound/se.acb.bytes")));
}

#[test]
fn each_cue_is_a_member_and_each_variation_has_a_number() {
    let data = sheet(&[(1, 0, "mono"), (2, 0, "variations"), (1, 1, "stereo")], 0);
    let asset = convert(Audio::new(None), data).unwrap().unwrap();
    let mono = plain("sine_mono.hca", None);
    let stereo = plain("sine_stereo.hca", None);
    assert_eq!(
        members(&asset),
        [
            ("cue/mono", mono.clone()),
            ("cue/variations/1", stereo.clone()),
            ("cue/variations/2", mono),
            ("cue/stereo", stereo),
        ]
    );
}

#[test]
fn the_cue_table_describes_each_member() {
    let data = sheet(&[(1, 0, "mono"), (2, 0, "variations")], 0);
    let asset = convert(Audio::new(None), data).unwrap().unwrap();
    let table = cue_table(&asset);
    let mono = plain("sine_mono.hca", None);
    let stereo = plain("sine_stereo.hca", None);
    let described: Vec<_> = table
        .cues
        .iter()
        .flat_map(|cue| cue.waveforms.iter().map(move |w| (cue.name.as_str(), w)))
        .map(|(name, w)| {
            (
                name,
                w.role.as_str(),
                w.frames,
                w.channels,
                w.loop_frames,
                w.volume,
            )
        })
        .collect();
    assert_eq!(
        described,
        [
            ("mono", "cue/mono", mono.frames() as u64, 1, None, 1.0),
            (
                "variations",
                "cue/variations/1",
                stereo.frames() as u64,
                2,
                None,
                1.0
            ),
            (
                "variations",
                "cue/variations/2",
                mono.frames() as u64,
                1,
                None,
                1.0
            ),
        ]
    );
}

#[test]
fn an_encrypted_stream_is_decrypted_with_the_key_and_the_bank_subkey() {
    let data = sheet(&[(1, 2, "locked")], 0);
    let asset = convert(Audio::new(Some(KEY)), data.clone())
        .unwrap()
        .unwrap();
    let expected = plain(
        "sine_stereo_subkey.hca",
        Some(HcaKey {
            key: KEY,
            subkey: SUBKEY,
        }),
    );
    assert_eq!(members(&asset), [("cue/locked", expected.clone())]);
    assert_eq!(expected, plain("sine_stereo.hca", None));

    let e = convert(Audio::new(None), data).unwrap_err();
    assert_eq!(e.code, "missing_key");
}

#[test]
fn a_streamed_waveform_is_unsupported() {
    let e = convert(Audio::new(None), sheet(&[(1, 0, "far")], 1)).unwrap_err();
    assert_eq!(e.code, "unsupported");
}

#[test]
fn a_damaged_sheet_is_an_error() {
    let mut data = sheet(&[(1, 0, "mono")], 0);
    data.truncate(data.len() / 2);
    assert_eq!(
        convert(Audio::new(None), data).unwrap_err().code,
        "truncated"
    );
    let e = convert(Audio::new(None), b"not a sheet".to_vec()).unwrap_err();
    assert!(matches!(e.code, "malformed" | "truncated"), "{e}");
}

#[test]
fn the_cost_is_five_times_the_sheet() {
    let mut object = index(49);
    object.size = 1000;
    let data = sheet(&[(1, 0, "mono")], 0);
    assert_eq!(
        Audio::new(None).cost(ROOT, &object, &Bundle(data)),
        Cost {
            class: CostClass::Heavy,
            memory: 5 * 1000,
        }
    );
}
