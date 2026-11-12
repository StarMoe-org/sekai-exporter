//! The path index format: chunks decode to what they were encoded from, and malformed ones are
//! rejected.

use sekai_format::paths::{Chunk, ChunkReference, PathFile, PathIndex};

fn file(path: &str, extension: &str, size: u64) -> PathFile {
    PathFile {
        path: path.to_owned(),
        sha256: [0xab; 32],
        extension: extension.to_owned(),
        size,
        media_type: format!("type/{extension}"),
    }
}

fn sample() -> Vec<PathFile> {
    vec![
        file("music/long/0001/0001.mp3", "mp3", 3_000_000),
        file("music/long/0001/0001_2.mp3", "mp3", 0),
        file("music/long/0002/0002.mp3", "mp3", u64::MAX),
        file("sprite/面板/星.png", "png", 127),
        file("sprite/面板/星.webp", "webp", 128),
    ]
}

#[test]
fn a_chunk_decodes_to_its_files() {
    let files = sample();
    let chunk = Chunk::decode(&Chunk::encode(&files)).unwrap();
    let decoded: Vec<PathFile> = chunk.files().map(PathFile::from).collect();
    assert_eq!(decoded, files);
    let found = chunk.get("sprite/面板/星.webp").map(PathFile::from);
    assert_eq!(found.as_ref(), Some(&files[4]));
    assert_eq!(chunk.get("sprite/面板/星"), None);
    assert_eq!(Chunk::decode(&Chunk::encode(&[])).unwrap().files().len(), 0);
}

#[test]
fn a_decoded_chunk_allocates_its_paths_and_a_row_for_each_file() {
    let files = sample();
    let chunk = Chunk::decode(&Chunk::encode(&files)).unwrap();
    let paths: usize = files.iter().map(|f| f.path.len()).sum();
    let allocated = chunk.allocated();
    assert!(allocated >= paths + files.len() * 32, "{allocated} bytes");
    assert!(allocated < paths + files.len() * 128, "{allocated} bytes");
}

#[test]
fn shared_prefixes_are_stored_once() {
    let files = sample();
    let encoded = Chunk::encode(&files);
    let paths: usize = files.iter().map(|f| f.path.len()).sum();
    assert!(
        encoded.len() < paths + files.len() * 40,
        "{} bytes",
        encoded.len()
    );
}

#[test]
fn malformed_chunks_are_rejected() {
    let good = Chunk::encode(&sample());
    let mut cases: Vec<(&str, Vec<u8>)> = vec![
        ("magic", [b"SKPX".as_slice(), &good[4..]].concat()),
        ("version", [&good[..4], &[2], &good[5..]].concat()),
        ("truncated", good[..good.len() - 1].to_vec()),
        ("trailing", [good.as_slice(), &[0]].concat()),
    ];
    let mut unordered = sample();
    unordered.swap(0, 1);
    cases.push(("order", Chunk::encode(&unordered)));
    let mut repeated = sample();
    repeated[1] = repeated[0].clone();
    cases.push(("repeat", Chunk::encode(&repeated)));
    // The last byte is the media type of the last file, and the chunk has three.
    let mut untyped = good.clone();
    *untyped.last_mut().unwrap() = 3;
    cases.push(("string index", untyped));
    for (name, bytes) in cases {
        assert!(Chunk::decode(&bytes).is_err(), "{name}");
    }
}

#[test]
fn a_path_falls_in_the_last_chunk_that_starts_at_or_before_it() {
    let chunk = |first: &str| ChunkReference {
        first: first.to_owned(),
        sha256: String::new(),
        files: 1,
    };
    let index = PathIndex {
        format: "path-index".to_owned(),
        version: 1,
        view: "canonical".to_owned(),
        published_version: "1".to_owned(),
        revision: String::new(),
        files: 3,
        chunks: vec![chunk("b"), chunk("d"), chunk("f")],
    };
    let first = |path: &str| {
        index
            .chunk_of(path)
            .map(|at| index.chunks[at].first.as_str())
    };
    assert_eq!(first("a"), None);
    assert_eq!(first("b"), Some("b"));
    assert_eq!(first("c/x"), Some("b"));
    assert_eq!(first("d"), Some("d"));
    assert_eq!(first("zzz"), Some("f"));

    let bytes = serde_json::to_vec(&index).unwrap();
    assert_eq!(PathIndex::from_bytes(&bytes).unwrap(), index);
    let mut unordered = index;
    unordered.chunks.swap(0, 1);
    assert!(PathIndex::from_bytes(&serde_json::to_vec(&unordered).unwrap()).is_err());
}
