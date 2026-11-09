//! The account of what became of every object of the Unity-built fixtures.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::Arc;

use sekai_catalog::LocalStore;
use sekai_convert::Converter;
use sekai_convert::image::Texture;
use sekai_format::AssetDocument;
use sekai_pipeline::ledger::account;
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::{BundleInput, MemoryDerivations, Version, evaluate};
use sekai_unity::NoBlockCache;
use sha2::{Digest, Sha256};

fn input(file: &str) -> BundleInput {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sekai-unity/tests/fixtures/lz4")
        .join(file);
    let bytes = std::fs::read(path).unwrap();
    BundleInput {
        name: file.to_owned(),
        sha256: Sha256::digest(&bytes).into(),
        source: Arc::new(Arc::<[u8]>::from(bytes)),
    }
}

#[test]
fn every_object_is_accounted_for_once() {
    let root = std::env::temp_dir().join(format!("sekai-ledger-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let (version, errors) = Version::open(
        vec![input("fixture_scene"), input("fixture_data")],
        &NoBlockCache,
    );
    assert!(errors.is_empty(), "{errors:?}");
    let store = LocalStore::open(&root).unwrap();
    let converters: Vec<Box<dyn Converter>> = vec![Box::new(Texture)];
    let evaluation = evaluate(
        &version,
        &converters,
        &store,
        &MemoryDerivations::default(),
        &NoRecorder,
        2,
    )
    .unwrap();
    let documents: Vec<AssetDocument> = evaluation
        .outputs
        .iter()
        .map(|o| AssetDocument::from_bytes(&store.read(&o.document).unwrap()).unwrap())
        .collect();

    let ledgers = account(&version, &converters, &documents, &[]);
    assert_eq!(ledgers.len(), 2);
    assert_eq!(
        ledgers.iter().map(|l| l.roots).sum::<u64>(),
        documents.len() as u64
    );
    for ledger in &ledgers {
        assert_eq!(
            ledger.objects,
            ledger.roots + ledger.read + ledger.ignored + ledger.unknown,
            "{ledger:?}"
        );
        assert_eq!(ledger.unknown, ledger.unknown_classes.values().sum::<u64>());
    }
    assert!(ledgers.iter().any(|l| !l.complete()));

    let classes: BTreeSet<&str> = ledgers
        .iter()
        .flat_map(|l| l.unknown_classes.keys().map(String::as_str))
        .collect();
    let classes: Vec<&str> = classes.into_iter().collect();
    let ignoring = account(&version, &converters, &documents, &classes);
    assert!(ignoring.iter().all(|l| l.unknown == 0), "{ignoring:?}");
    assert_eq!(
        ignoring.iter().map(|l| l.ignored).sum::<u64>(),
        ledgers.iter().map(|l| l.unknown).sum::<u64>()
    );
    std::fs::remove_dir_all(root).unwrap();
}
