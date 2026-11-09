//! The in-memory run against the eager evaluator, on the Unity-built fixtures.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use sekai_catalog::LocalStore;
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::{BundleInput, MemoryDerivations, Version, evaluate};
use sekai_unity::NoBlockCache;
use sekai_wasm::{converters, row, run};
use sha2::{Digest, Sha256};

fn bundles() -> Vec<(String, Vec<u8>)> {
    ["fixture_scene", "fixture_data"]
        .iter()
        .map(|file| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../sekai-unity/tests/fixtures/lz4")
                .join(file);
            ((*file).to_owned(), std::fs::read(path).unwrap())
        })
        .collect()
}

#[test]
fn the_in_memory_run_writes_what_the_eager_evaluator_writes() {
    let root = std::env::temp_dir().join(format!("sekai-wasm-run-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let store = LocalStore::open(&root).unwrap();
    let inputs = bundles()
        .into_iter()
        .map(|(name, bytes)| BundleInput {
            name,
            sha256: Sha256::digest(&bytes).into(),
            source: Arc::new(Arc::<[u8]>::from(bytes)),
        })
        .collect();
    let (version, errors) = Version::open(inputs, &NoBlockCache);
    assert!(errors.is_empty(), "{errors:?}");
    let eager = evaluate(
        &version,
        &converters(),
        &store,
        &MemoryDerivations::default(),
        &NoRecorder,
        1,
    )
    .unwrap();
    assert!(!eager.outputs.is_empty());

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
    assert_eq!(report.assets, eager.outputs.len());
    assert!(report.bundle_errors.is_empty());
    let expected: String = eager.outputs.iter().map(row).collect();
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
