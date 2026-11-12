//! The in-memory run against a native streaming run, on the Unity-built fixtures.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::Arc;

use sekai_catalog::LocalStore;
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::stream::{self, Entry, InBundle, Inputs, Segment, Stream};
use sekai_pipeline::{BundleInput, MemoryDerivations, NoBundleIndexes, Options};
use sekai_unity::{NoBlockCache, NoClasses};
use sekai_wasm::{converters, row, run};
use sha2::{Digest, Sha256};

const FIXTURES: [&str; 2] = ["fixture_scene", "fixture_data"];

fn bytes(file: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sekai-unity/tests/fixtures/lz4")
        .join(file);
    std::fs::read(path).unwrap()
}

fn bundles() -> Vec<(String, Vec<u8>)> {
    FIXTURES
        .iter()
        .map(|file| ((*file).to_owned(), bytes(file)))
        .collect()
}

struct Fixtures;

impl Inputs for Fixtures {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        let bytes = bytes(name);
        Ok(BundleInput {
            name: name.to_owned(),
            sha256: Sha256::digest(&bytes).into(),
            source: Arc::new(Arc::<[u8]>::from(bytes)),
        })
    }
}

#[test]
fn the_in_memory_run_writes_what_a_native_run_writes() {
    let root = std::env::temp_dir().join(format!("sekai-wasm-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let store = LocalStore::open(&root).unwrap();
    let mut manifest: Vec<Entry> = FIXTURES
        .iter()
        .map(|name| Entry {
            name: (*name).to_owned(),
            size: 1,
            declared: String::new(),
            dependencies: Vec::new(),
        })
        .collect();
    stream::with_dependencies(&mut manifest, &Fixtures, &NoBlockCache, &NoBundleIndexes, 2)
        .unwrap();
    let converters = converters();
    let native = stream::evaluate(
        &manifest,
        &Fixtures,
        &Stream {
            converters: &converters,
            naming: &InBundle,
            store: &store,
            derivations: &MemoryDerivations::default(),
            indexes: &NoBundleIndexes,
            cache: &NoBlockCache,
            classes: &NoClasses,
            recorder: &NoRecorder,
            fusion: None,
            options: Options::threads(2),
            segment: Segment::default(),
            ledger: None,
            limit: None,
        },
    )
    .unwrap();
    assert!(native.errors.is_empty(), "{:?}", native.errors);
    let native = native.evaluation;
    assert!(!native.outputs.is_empty());

    let mut files = BTreeMap::new();
    let mut rows = String::new();
    let report = run(
        bundles(),
        &mut |key, bytes| {
            files.insert(key.clone(), bytes.to_vec());
            Ok(())
        },
        &mut |output| {
            rows.push_str(&row(output));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(report.assets, native.outputs.len());
    assert_eq!(report.unnamed, native.unnamed.len());
    assert!(report.bundle_errors.is_empty());
    let expected: String = native.outputs.iter().map(row).collect();
    assert_eq!(rows, expected);
    assert!(!files.is_empty());
    for (key, bytes) in &files {
        assert_eq!(*bytes, store.read(key).unwrap());
    }
    let json = store.keys("json").unwrap();
    assert_eq!(
        json.len(),
        files
            .keys()
            .filter(|k| k.to_string().ends_with("json"))
            .count()
    );
    drop(store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn the_in_memory_object_documents_are_those_of_a_native_run() {
    use sekai_pipeline::objects::{Source, document, index};
    use sekai_unity::document::Payloads;

    struct Collect(BTreeMap<String, Vec<u8>>);
    impl Payloads for Collect {
        fn store(&mut self, sha256: &[u8; 32], bytes: &[u8]) -> sekai_unity::Result<()> {
            let hex: String = sha256.iter().map(|b| format!("{b:02x}")).collect();
            let path = format!("_payloads/{}/{hex}.bin", &hex[..2]);
            self.0.insert(path, bytes.to_vec());
            Ok(())
        }
    }

    let source = Source {
        inputs: &Fixtures,
        classes: &NoClasses,
        cache: &NoBlockCache,
    };
    let names: Vec<String> = FIXTURES.iter().rev().map(|n| (*n).to_owned()).collect();
    let indexed = index(&source, &names, 2);
    assert!(indexed.failed.is_empty(), "{:?}", indexed.failed);
    let mut payloads = Collect(BTreeMap::new());
    let mut expected = BTreeMap::new();
    for (b, bundle) in indexed.version.bundles().iter().enumerate() {
        let bytes = document(&source, &indexed.version, b, &mut payloads).unwrap();
        expected.insert(format!("{}/_objects.json", bundle.name), bytes);
    }
    assert!(!payloads.0.is_empty());
    expected.extend(payloads.0);

    let mut bundles = bundles();
    bundles.push(("not_a_bundle".to_owned(), b"not a bundle".to_vec()));
    let mut files = BTreeMap::new();
    let failed = sekai_wasm::objects(bundles, &mut |path, bytes| {
        assert!(files.insert(path.to_owned(), bytes.to_vec()).is_none());
        Ok(())
    })
    .unwrap();
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].0, "not_a_bundle");
    assert_eq!(files, expected);
}

#[test]
fn converters_cover_every_default_form_and_absent_rule() {
    let forms: Vec<&str> = converters().iter().map(|c| c.name()).collect();
    assert_eq!(
        forms,
        [
            "texture",
            "sprite",
            "audio",
            "video",
            "text",
            "font",
            "live2d-model",
            "live2d-motions",
            "emoticon",
            "facial-table",
            "field-camera",
            "fixture-meta",
            "fixture-model",
            "fixture-reactions",
            "site-model",
            "scenario",
            "virtual-live-script",
            "action-set",
            "scenario-background",
            "costume-model",
            "costume-motion",
        ]
    );
}
