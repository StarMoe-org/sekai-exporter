//! The account of what became of every object of the Unity-built fixtures.
//!
//! The two fixture bundles refer to each other, so each depends on the other.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;
use std::sync::Arc;

use sekai_catalog::LocalStore;
use sekai_convert::Converter;
use sekai_convert::image::Texture;
use sekai_pipeline::ledger::BundleLedger;
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::stream::{self, Entry, InBundle, Inputs, Segment, Stream, Streamed};
use sekai_pipeline::{BundleInput, MemoryDerivations, NoBundleIndexes, Options};
use sekai_unity::NoBlockCache;
use sha2::{Digest, Sha256};

struct Fixtures;

impl Inputs for Fixtures {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sekai-unity/tests/fixtures/lz4")
            .join(name);
        let bytes = std::fs::read(path)?;
        Ok(BundleInput {
            name: name.to_owned(),
            sha256: Sha256::digest(&bytes).into(),
            source: Arc::new(Arc::<[u8]>::from(bytes)),
        })
    }
}

fn entry(name: &str, dependencies: &[&str]) -> Entry {
    Entry {
        name: name.to_owned(),
        size: 1,
        dependencies: dependencies.iter().map(|d| (*d).to_owned()).collect(),
    }
}

/// Each fixture depending on the other.
fn both() -> [Entry; 2] {
    [
        entry("fixture_data", &["fixture_scene"]),
        entry("fixture_scene", &["fixture_data"]),
    ]
}

fn streamed(root: &Path, manifest: &[Entry], ignored: &[&str], segment: Segment) -> Streamed {
    let store = LocalStore::open(root).unwrap();
    let converters: Vec<Box<dyn Converter>> = vec![Box::new(Texture)];
    let stream = Stream {
        converters: &converters,
        naming: &InBundle,
        store: &store,
        derivations: &MemoryDerivations::default(),
        indexes: &NoBundleIndexes,
        cache: &NoBlockCache,
        recorder: &NoRecorder,
        fusion: None,
        options: Options::threads(2),
        segment,
        ledger: Some(ignored),
        limit: None,
    };
    stream::evaluate(manifest, &Fixtures, &stream).unwrap()
}

#[test]
fn every_object_is_accounted_for_once() {
    let root = std::env::temp_dir().join(format!("sekai-ledger-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let result = streamed(&root, &both(), &[], Segment::default());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let ledgers = &result.ledgers;
    let bundles: Vec<&str> = ledgers.iter().map(|l| l.bundle.as_str()).collect();
    assert_eq!(bundles, ["fixture_data", "fixture_scene"]);
    assert_eq!(
        ledgers.iter().map(|l| l.roots).sum::<u64>(),
        result.evaluation.outputs.len() as u64
    );
    for ledger in ledgers {
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
    let ignoring = streamed(&root, &both(), &classes, Segment::default()).ledgers;
    assert!(ignoring.iter().all(|l| l.unknown == 0), "{ignoring:?}");
    assert_eq!(
        ignoring.iter().map(|l| l.ignored).sum::<u64>(),
        ledgers.iter().map(|l| l.unknown).sum::<u64>()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_ledger_is_settled_after_the_last_unit_that_reads_its_bundle() {
    let root = std::env::temp_dir().join(format!("sekai-ledger-seg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    // fixture_data is needed by its own unit only, fixture_scene by both units.
    let manifest = [
        entry("fixture_data", &["fixture_scene"]),
        entry("fixture_scene", &[]),
    ];
    let together: Vec<BundleLedger> = streamed(&root, &manifest, &[], Segment::default()).ledgers;
    let apart = streamed(
        &root,
        &manifest,
        &[],
        Segment {
            units: 1,
            bytes: u64::MAX,
        },
    )
    .ledgers;
    assert_eq!(together.len(), 2);
    assert_eq!(together, apart);
    std::fs::remove_dir_all(root).unwrap();
}
