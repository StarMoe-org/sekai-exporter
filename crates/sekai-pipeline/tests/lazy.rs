//! The lazy evaluator against the eager one, on the Unity-built fixtures.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sekai_catalog::{BundleIndexes, LocalStore, MemoryStore};
use sekai_convert::Converter;
use sekai_convert::image::Texture;
use sekai_format::AssetDocument;
use sekai_pipeline::metrics::{NoRecorder, Recorder, Span};
use sekai_pipeline::{BundleInput, Lazy, MemoryDerivations, Version, evaluate};
use sekai_unity::{ByteSource, Chunk, NoBlockCache};
use sha2::{Digest, Sha256};

const FIXTURES: [&str; 2] = ["fixture_scene", "fixture_data"];

fn bytes(file: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sekai-unity/tests/fixtures/lz4")
        .join(file);
    std::fs::read(path).unwrap()
}

fn input(file: &str) -> BundleInput {
    let bytes = bytes(file);
    BundleInput {
        name: file.to_owned(),
        sha256: Sha256::digest(&bytes).into(),
        source: Arc::new(Arc::<[u8]>::from(bytes)),
    }
}

fn inputs() -> Vec<BundleInput> {
    FIXTURES.iter().map(|file| input(file)).collect()
}

fn converters() -> Vec<Box<dyn Converter>> {
    vec![Box::new(Texture)]
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sekai-lazy-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn lazy_results_are_the_eager_results() {
    let root = scratch("same");
    let eager = LocalStore::open(&root).unwrap();
    let (version, errors) = Version::open(inputs(), &NoBlockCache);
    assert!(errors.is_empty(), "{errors:?}");
    let converters = converters();
    let evaluation = evaluate(
        &version,
        &converters,
        &eager,
        &MemoryDerivations::default(),
        &NoRecorder,
        2,
    )
    .unwrap();
    assert!(!evaluation.outputs.is_empty());

    let memory = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let lazy = Lazy::new(&version, &converters, &memory, &derivations, &NoRecorder, 1);
    let names: Vec<_> = lazy.names().map(str::to_owned).collect();
    assert_eq!(
        names,
        evaluation
            .outputs
            .iter()
            .map(|o| o.name.clone())
            .collect::<Vec<_>>()
    );
    for output in &evaluation.outputs {
        let requested = lazy.asset(&output.name).unwrap().unwrap();
        assert_eq!(&requested, output);
        let document = memory.read(&requested.document).unwrap();
        assert_eq!(document, eager.read(&output.document).unwrap());
        let document = AssetDocument::from_bytes(&document).unwrap();
        for member in &document.members {
            let key = format!("{}.{}", member.sha256, member.extension)
                .parse()
                .unwrap();
            assert_eq!(memory.read(&key).unwrap(), eager.read(&key).unwrap());
        }
    }
    assert!(lazy.asset("no/such/asset").unwrap().is_none());
    drop(eager);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_object_is_requested_by_bundle_and_path_id() {
    let (version, _) = Version::open(inputs(), &NoBlockCache);
    let converters = converters();
    let memory = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let lazy = Lazy::new(&version, &converters, &memory, &derivations, &NoRecorder, 1);
    let name = lazy.names().next().unwrap().to_owned();
    let output = lazy.asset(&name).unwrap().unwrap();
    let document = AssetDocument::from_bytes(&memory.read(&output.document).unwrap()).unwrap();
    let root = &document.sources[0];
    assert_eq!(lazy.object(&root.bundle, root.path_id).unwrap(), [output]);
    assert!(lazy.object("no bundle", 1).unwrap().is_empty());
}

#[derive(Default)]
struct Conversions(AtomicUsize);

impl Recorder for Conversions {
    fn span<'a>(&'a self, stage: &'static str, _: &str, _: &str) -> Box<dyn Span + 'a> {
        if stage == "convert" {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        Box::new(NoRecorder)
    }
}

#[test]
fn concurrent_requests_for_one_asset_derive_it_once() {
    let (version, _) = Version::open(inputs(), &NoBlockCache);
    let converters = converters();
    let memory = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let conversions = Conversions::default();
    let lazy = Lazy::new(
        &version,
        &converters,
        &memory,
        &derivations,
        &conversions,
        1,
    );
    let name = lazy.names().next().unwrap().to_owned();
    let results: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| lazy.asset(&name).unwrap().unwrap()))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(results.iter().all(|output| *output == results[0]));
    assert_eq!(conversions.0.load(Ordering::SeqCst), 1);
}

#[test]
fn a_limit_refuses_an_asset_by_name() {
    let (version, _) = Version::open(inputs(), &NoBlockCache);
    let converters = converters();
    let memory = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let lazy = Lazy::new(&version, &converters, &memory, &derivations, &NoRecorder, 1)
        .with_memory_limit(0);
    let name = lazy.names().next().unwrap().to_owned();
    let output = lazy.asset(&name).unwrap().unwrap();
    assert_eq!(output.error.as_deref(), Some("too_large"));
    let document = AssetDocument::from_bytes(&memory.read(&output.document).unwrap()).unwrap();
    assert!(document.members.is_empty());
    assert_eq!(document.error.unwrap().code, "too_large");
}

/// The indexes of bundles, kept in memory.
#[derive(Default)]
struct MemoryIndexes(Mutex<BTreeMap<[u8; 32], Vec<u8>>>);

impl BundleIndexes for MemoryIndexes {
    fn get(&self, bundle: &[u8; 32]) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.lock().unwrap().get(bundle).cloned())
    }

    fn put(&self, bundle: [u8; 32], index: Vec<u8>) -> io::Result<()> {
        self.0.lock().unwrap().insert(bundle, index);
        Ok(())
    }
}

/// Bytes that record the name of their bundle when they are read.
struct Counted {
    bytes: Arc<[u8]>,
    name: String,
    read: Arc<Mutex<BTreeSet<String>>>,
}

impl ByteSource for Counted {
    fn len(&self) -> u64 {
        ByteSource::len(&self.bytes)
    }

    fn read(&self, offset: u64, length: usize) -> sekai_unity::Result<Chunk<'_>> {
        self.read.lock().unwrap().insert(self.name.clone());
        ByteSource::read(&self.bytes, offset, length)
    }
}

#[test]
fn a_request_reads_only_the_bundles_of_its_closure() {
    let indexes = MemoryIndexes::default();
    drop(Version::open_indexed(inputs(), &NoBlockCache, &indexes, 1).unwrap());

    let read = Arc::new(Mutex::new(BTreeSet::new()));
    let counted: Vec<_> = FIXTURES
        .iter()
        .map(|file| {
            let plain = input(file);
            BundleInput {
                source: Arc::new(Counted {
                    bytes: Arc::from(bytes(file)),
                    name: (*file).to_owned(),
                    read: read.clone(),
                }),
                ..plain
            }
        })
        .collect();
    let (version, errors) = Version::open_indexed(counted, &NoBlockCache, &indexes, 1).unwrap();
    assert!(errors.is_empty());
    let converters = converters();
    let memory = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let lazy = Lazy::new(&version, &converters, &memory, &derivations, &NoRecorder, 1);
    assert!(read.lock().unwrap().is_empty(), "preparing read a bundle");

    let name = lazy.names().next().unwrap().to_owned();
    let output = lazy.asset(&name).unwrap().unwrap();
    let document = AssetDocument::from_bytes(&memory.read(&output.document).unwrap()).unwrap();
    let closure: BTreeSet<_> = document.sources.iter().map(|s| s.bundle.clone()).collect();
    let touched = read.lock().unwrap().clone();
    assert!(!touched.is_empty());
    assert!(
        touched.is_subset(&closure),
        "{touched:?} is outside {closure:?}"
    );
}
