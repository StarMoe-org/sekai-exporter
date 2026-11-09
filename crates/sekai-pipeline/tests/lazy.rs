//! The lazy evaluator against the streaming one, on the Unity-built fixtures.
//!
//! The two fixture bundles refer to each other, so each depends on the other.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sekai_catalog::{BundleIndexes, LocalStore, MemoryStore, Store};
use sekai_convert::Converter;
use sekai_convert::image::Texture;
use sekai_format::AssetDocument;
use sekai_pipeline::metrics::{NoRecorder, Recorder, Span};
use sekai_pipeline::stream::{self, Entry, InBundle, Inputs, Segment, Stream};
use sekai_pipeline::{BundleInput, Lazy, MemoryDerivations, NoBundleIndexes, Options, Version};
use sekai_unity::{ByteSource, Chunk, NoBlockCache, NoClasses};
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

struct Fixtures;

impl Inputs for Fixtures {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        Ok(input(name))
    }
}

/// Each fixture depending on the other.
fn manifest() -> Vec<Entry> {
    FIXTURES
        .iter()
        .map(|name| Entry {
            name: (*name).to_owned(),
            size: 1,
            dependencies: FIXTURES
                .iter()
                .filter(|other| *other != name)
                .map(|other| (*other).to_owned())
                .collect(),
        })
        .collect()
}

fn converters() -> Vec<Box<dyn Converter>> {
    vec![Box::new(Texture)]
}

fn context<'a>(
    converters: &'a [Box<dyn Converter>],
    store: &'a dyn Store,
    derivations: &'a MemoryDerivations,
    indexes: &'a dyn BundleIndexes,
    recorder: &'a dyn Recorder,
) -> Stream<'a> {
    Stream {
        converters,
        naming: &InBundle,
        store,
        derivations,
        indexes,
        cache: &NoBlockCache,
        classes: &NoClasses,
        recorder,
        fusion: None,
        options: Options::threads(1),
        segment: Segment::default(),
        ledger: None,
        limit: None,
    }
}

/// The name of every asset, bundle by bundle.
fn names(lazy: &Lazy<'_>) -> Vec<String> {
    let bundles: Vec<String> = lazy.bundles().map(str::to_owned).collect();
    let mut names = Vec::new();
    for bundle in bundles {
        names.extend(lazy.names(&bundle).unwrap());
    }
    names
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sekai-lazy-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn lazy_results_are_the_streamed_results() {
    let root = scratch("same");
    let streamed_store = LocalStore::open(&root).unwrap();
    let converters = converters();
    let manifest = manifest();
    let derivations = MemoryDerivations::default();
    let streamed = stream::evaluate(
        &manifest,
        &Fixtures,
        &Stream {
            options: Options::threads(2),
            ..context(
                &converters,
                &streamed_store,
                &derivations,
                &NoBundleIndexes,
                &NoRecorder,
            )
        },
    )
    .unwrap();
    let evaluation = streamed.evaluation;
    assert!(!evaluation.outputs.is_empty());

    let memory = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let lazy = Lazy::new(
        &manifest,
        &Fixtures,
        context(
            &converters,
            &memory,
            &derivations,
            &NoBundleIndexes,
            &NoRecorder,
        ),
    );
    let mut names = names(&lazy);
    names.sort();
    let expected: Vec<String> = evaluation.outputs.iter().map(|o| o.name.clone()).collect();
    assert_eq!(names, expected);
    for output in &evaluation.outputs {
        let requested = lazy.asset(&output.name).unwrap().unwrap();
        assert_eq!(&requested, output);
        let document = memory.read(&requested.document).unwrap();
        assert_eq!(document, streamed_store.read(&output.document).unwrap());
        let document = AssetDocument::from_bytes(&document).unwrap();
        for member in &document.members {
            let key = format!("{}.{}", member.sha256, member.extension)
                .parse()
                .unwrap();
            assert_eq!(
                memory.read(&key).unwrap(),
                streamed_store.read(&key).unwrap()
            );
        }
    }
    assert!(lazy.asset("no/such/asset").unwrap().is_none());
    assert!(lazy.names("no such bundle").unwrap().is_empty());
    assert!(lazy.errors().is_empty());
    drop(streamed_store);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_object_is_requested_by_bundle_and_path_id() {
    let converters = converters();
    let manifest = manifest();
    let memory = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let lazy = Lazy::new(
        &manifest,
        &Fixtures,
        context(
            &converters,
            &memory,
            &derivations,
            &NoBundleIndexes,
            &NoRecorder,
        ),
    );
    let name = names(&lazy)[0].clone();
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
    let converters = converters();
    let manifest = manifest();
    let memory = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let conversions = Conversions::default();
    let lazy = Lazy::new(
        &manifest,
        &Fixtures,
        context(
            &converters,
            &memory,
            &derivations,
            &NoBundleIndexes,
            &conversions,
        ),
    );
    let name = names(&lazy)[0].clone();
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
    let converters = converters();
    let manifest = manifest();
    let memory = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let lazy = Lazy::new(
        &manifest,
        &Fixtures,
        Stream {
            limit: Some(0),
            ..context(
                &converters,
                &memory,
                &derivations,
                &NoBundleIndexes,
                &NoRecorder,
            )
        },
    );
    let name = names(&lazy)[0].clone();
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

/// The fixtures as bytes that record the name of their bundle when they are read.
struct CountedFixtures(Arc<Mutex<BTreeSet<String>>>);

impl Inputs for CountedFixtures {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        let plain = input(name);
        Ok(BundleInput {
            source: Arc::new(Counted {
                bytes: Arc::from(bytes(name)),
                name: name.to_owned(),
                read: self.0.clone(),
            }),
            ..plain
        })
    }
}

#[test]
fn a_request_reads_only_the_bundles_of_its_closure() {
    let indexes = MemoryIndexes::default();
    let all = FIXTURES.iter().map(|file| input(file)).collect();
    drop(Version::open_indexed(all, &NoBlockCache, &NoClasses, &indexes, 1).unwrap());

    let read = Arc::new(Mutex::new(BTreeSet::new()));
    let inputs = CountedFixtures(read.clone());
    let converters = converters();
    let manifest = manifest();
    let memory = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let lazy = Lazy::new(
        &manifest,
        &inputs,
        context(&converters, &memory, &derivations, &indexes, &NoRecorder),
    );
    let name = names(&lazy)[0].clone();
    assert!(read.lock().unwrap().is_empty(), "naming read a bundle");
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
