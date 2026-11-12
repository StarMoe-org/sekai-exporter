//! Path indexes: every path resolves to its file, unchanged ranges keep their chunks, and every
//! version keeps its pointer.

use std::collections::BTreeMap;
use std::path::PathBuf;

use sekai_catalog::store::parse_hex;
use sekai_catalog::{Key, LocalStore, Objects};
use sekai_format::paths::{Chunk, PathIndex};
use sekai_format::publication::Pointer;
use sekai_publish::{File, Layout, Plan, publish};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sekai-paths-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn store(&self) -> LocalStore {
        LocalStore::open(self.0.join("store")).unwrap()
    }

    fn published(&self) -> PathBuf {
        self.0.join("published")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A plan of a view whose paths all hold one small object.
fn view(store: &LocalStore, version: &str, paths: &[String]) -> Plan {
    let key = store.put(b"x", "txt").unwrap();
    let mut files: Vec<File> = paths
        .iter()
        .map(|path| File {
            path: path.clone(),
            key: key.clone(),
            size: 1,
            media_type: "text/plain".to_owned(),
            asset: path.clone(),
            role: "text".to_owned(),
        })
        .collect();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Plan {
        version: version.to_owned(),
        files,
        assets: Vec::new(),
        listings: BTreeMap::new(),
        failures: Vec::new(),
        revision: format!("revision of {version}"),
    }
}

fn paths(count: usize) -> Vec<String> {
    (0..count)
        .map(|i| format!("group_{:02}/item_{i:05}.txt", i % 13))
        .collect()
}

fn object(sha256: &str, extension: &str) -> Key {
    Key {
        sha256: parse_hex(sha256).unwrap(),
        extension: extension.to_owned(),
    }
}

fn index(store: &LocalStore, pointer: &Pointer) -> PathIndex {
    let root = object(pointer.paths.as_deref().unwrap(), "json");
    PathIndex::from_bytes(&store.read(&root).unwrap()).unwrap()
}

fn chunk(store: &LocalStore, sha256: &str) -> Chunk {
    Chunk::decode(&store.read(&object(sha256, "paths")).unwrap()).unwrap()
}

#[test]
fn every_path_resolves_to_its_file_and_no_other_path_does() {
    let s = Scratch::new("resolve");
    let store = s.store();
    let plan = view(&store, "v1", &paths(9000));
    let pointer = publish(
        &Objects::local(&store),
        &s.published(),
        "jp/storage",
        &plan,
        Layout::Paths,
        2,
    )
    .unwrap();
    assert_eq!(pointer.root, None);
    let index = index(&store, &pointer);
    assert_eq!(index.view, "storage");
    assert_eq!(index.files, 9000);
    assert!(index.chunks.len() > 1, "9000 paths fill several chunks");
    let total: u64 = index.chunks.iter().map(|c| c.files).sum();
    assert_eq!(total, 9000);

    let mut chunks: BTreeMap<&str, Chunk> = BTreeMap::new();
    for file in &plan.files {
        let reference = &index.chunks[index.chunk_of(&file.path).unwrap()];
        let found = chunks
            .entry(&reference.sha256)
            .or_insert_with(|| chunk(&store, &reference.sha256));
        let found = found.get(&file.path).unwrap();
        assert_eq!((found.sha256, found.size), (file.key.sha256, file.size));
        assert_eq!((found.extension, found.media_type), ("txt", "text/plain"));
    }
    for absent in [
        "",
        "group_00",
        "group_00/item_00000.tx",
        "group_05/item_99999.txt",
        "zz",
    ] {
        let found = index
            .chunk_of(absent)
            .map(|at| chunk(&store, &index.chunks[at].sha256));
        assert!(
            found.is_none_or(|chunk| chunk.get(absent).is_none()),
            "{absent}"
        );
    }
}

#[test]
fn a_change_to_one_path_keeps_the_other_chunks() {
    let s = Scratch::new("stable");
    let store = s.store();
    let mut names = paths(20000);
    let first = publish(
        &Objects::local(&store),
        &s.published(),
        "jp/storage",
        &view(&store, "v1", &names),
        Layout::Paths,
        2,
    )
    .unwrap();
    names.push("group_03/item_10000a.txt".to_owned());
    let second = publish(
        &Objects::local(&store),
        &s.published(),
        "jp/storage",
        &view(&store, "v2", &names),
        Layout::Paths,
        2,
    )
    .unwrap();

    let chunks = |pointer: &Pointer| -> Vec<String> {
        index(&store, pointer)
            .chunks
            .into_iter()
            .map(|c| c.sha256)
            .collect()
    };
    let (before, after) = (chunks(&first), chunks(&second));
    let kept = after.iter().filter(|sha| before.contains(sha)).count();
    assert!(before.len() > 3, "{} chunks", before.len());
    assert_eq!(
        kept,
        before.len() - 1,
        "only the chunk that gained the path changes"
    );
}

#[test]
fn every_version_keeps_its_pointer() {
    let s = Scratch::new("versions");
    let store = s.store();
    let root = s.published();
    let one = publish(
        &Objects::local(&store),
        &root,
        "jp/canonical",
        &view(&store, "1.0.0", &paths(10)),
        Layout::Index,
        2,
    )
    .unwrap();
    let two = publish(
        &Objects::local(&store),
        &root,
        "jp/canonical",
        &view(&store, "1.1.0", &paths(20)),
        Layout::Index,
        2,
    )
    .unwrap();
    let read = |path: &str| -> Pointer {
        serde_json::from_slice(&std::fs::read(root.join(path)).unwrap()).unwrap()
    };
    assert_eq!(read("jp/canonical/versions/1.0.0.json"), one);
    assert_eq!(read("jp/canonical/versions/1.1.0.json"), two);
    assert_eq!(read("jp/canonical/current.json"), two);
    assert!(one.root.is_some() && one.paths.is_some());
}

#[test]
fn a_version_that_cannot_name_a_file_is_refused() {
    let s = Scratch::new("names");
    let store = s.store();
    for version in ["", "..", "a/b", "v 1"] {
        let error = publish(
            &Objects::local(&store),
            &s.published(),
            "jp/canonical",
            &view(&store, version, &paths(3)),
            Layout::Paths,
            2,
        )
        .unwrap_err();
        assert_eq!(error.stage, "pointer", "{version:?}");
    }
}
