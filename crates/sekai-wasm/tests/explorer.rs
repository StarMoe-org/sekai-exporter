//! The explorer session against the one-shot runs, on the Unity-built fixtures.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use sekai_format::AssetDocument;
use sekai_unity::ByteSource;
use sekai_unity::document::Payloads;
use sekai_wasm::explorer::Explorer;
use sekai_wasm::stored::Restored;

const FIXTURES: [&str; 2] = ["fixture_data", "fixture_scene"];

fn bytes(file: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sekai-unity/tests/fixtures/lz4")
        .join(file);
    std::fs::read(path).unwrap()
}

/// `plain` as the game stores it: a marker, then the first five of every eight of the first 128
/// bytes inverted.
fn obfuscated(plain: &[u8]) -> Vec<u8> {
    let mut stored = vec![0x10, 0, 0, 0];
    let inverted = plain.iter().enumerate();
    stored.extend(inverted.map(|(i, &b)| if i < 128 && i % 8 < 5 { !b } else { b }));
    stored
}

fn source(bytes: Vec<u8>) -> Arc<dyn ByteSource> {
    Arc::new(Arc::<[u8]>::from(bytes))
}

/// The fixtures, the first stored obfuscated.
fn explorer() -> Explorer {
    let stored = |(i, name): (usize, &&str)| {
        let plain = bytes(name);
        let stored = if i == 0 { obfuscated(&plain) } else { plain };
        ((*name).to_owned(), source(stored))
    };
    Explorer::open(FIXTURES.iter().enumerate().map(stored).collect()).unwrap()
}

#[derive(Default)]
struct Collect(BTreeMap<[u8; 32], Vec<u8>>);

impl Payloads for Collect {
    fn store(&mut self, sha256: &[u8; 32], bytes: &[u8]) -> sekai_unity::Result<()> {
        self.0.insert(*sha256, bytes.to_vec());
        Ok(())
    }
}

#[test]
fn a_restored_bundle_reads_as_its_unityfs_bytes() {
    let plain = bytes(FIXTURES[0]);
    let restored = Restored::new(source(obfuscated(&plain))).unwrap();
    assert_eq!(restored.len(), plain.len() as u64);
    for (offset, len) in [(0, 8), (5, 200), (120, 16), (300, 50)] {
        let range = restored.read(offset, len).unwrap();
        let offset = offset as usize;
        assert_eq!(
            *range,
            plain[offset..offset + len],
            "{len} bytes at {offset}"
        );
    }
    assert!(Restored::new(source(vec![9; 200])).is_err());
}

#[test]
fn object_documents_are_those_of_the_one_shot_run() {
    let explorer = explorer();
    assert_eq!(explorer.bundles().collect::<Vec<_>>(), FIXTURES);
    assert!(explorer.failed().is_empty());
    let mut expected = BTreeMap::new();
    let plain = FIXTURES.map(|name| (name.to_owned(), bytes(name)));
    sekai_wasm::objects(plain.to_vec(), &mut |path, bytes| {
        expected.insert(path.to_owned(), bytes.to_vec());
        Ok(())
    })
    .unwrap();
    for bundle in FIXTURES {
        let document = explorer.document(bundle, &mut Collect::default()).unwrap();
        assert_eq!(document, expected[&format!("{bundle}/_objects.json")]);
    }
    assert!(
        explorer
            .document("no such bundle", &mut Collect::default())
            .is_err()
    );
}

#[test]
fn assets_are_planned_converted_and_closed_as_in_the_one_shot_run() {
    let explorer = explorer();
    let mut files = BTreeMap::new();
    let mut outputs = Vec::new();
    let plain = FIXTURES.map(|name| (name.to_owned(), bytes(name)));
    sekai_wasm::run(
        plain.to_vec(),
        &mut |key, bytes| {
            files.insert(key.clone(), bytes.to_vec());
            Ok(())
        },
        &mut |output| {
            outputs.push(output.clone());
            Ok(())
        },
    )
    .unwrap();
    assert!(!outputs.is_empty());

    let planned: Vec<_> = FIXTURES
        .iter()
        .flat_map(|bundle| explorer.assets(bundle).unwrap())
        .collect();
    let mut names: Vec<&str> = planned.iter().map(|asset| asset.name.as_str()).collect();
    names.sort_unstable();
    let expected: Vec<&str> = outputs.iter().map(|output| output.name.as_str()).collect();
    assert_eq!(names, expected);

    for output in &outputs {
        let document = explorer.convert(&output.name).unwrap().unwrap();
        assert_eq!(document, files[&output.document]);
        let document = AssetDocument::from_bytes(&document).unwrap();
        for member in &document.members {
            let key = format!("{}.{}", member.sha256, member.extension)
                .parse()
                .unwrap();
            assert_eq!(explorer.file(&key).unwrap(), files[&key]);
        }
        let closure = explorer.closure(&output.name).unwrap().unwrap();
        assert_eq!(closure.first(), document.sources.first());
        assert!(document.sources.iter().all(|read| closure.contains(read)));
    }
    assert!(explorer.convert("no/such/asset").unwrap().is_none());
    assert!(explorer.closure("no/such/asset").unwrap().is_none());
}
