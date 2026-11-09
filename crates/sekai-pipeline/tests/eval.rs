//! Evaluating the Unity-built fixtures: outputs, determinism and reuse.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sekai_catalog::LocalStore;
use sekai_convert::image::Texture;
use sekai_convert::{
    Asset, Context, ConvertError, Converter, Cost, CostClass, ObjectId, ObjectIndex,
};
use sekai_format::AssetDocument;
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::{
    BundleInput, Evaluation, Fusion, MemoryDerivations, Options, Version, evaluate, evaluate_fused,
    evaluate_with,
};
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

fn run(root: &Path, threads: usize, derivations: &MemoryDerivations) -> Evaluation {
    run_with(Box::new(Texture), root, threads, derivations)
}

fn run_with(
    converter: Box<dyn Converter>,
    root: &Path,
    threads: usize,
    derivations: &MemoryDerivations,
) -> Evaluation {
    let (version, errors) = Version::open(
        vec![input("fixture_scene"), input("fixture_data")],
        &NoBlockCache,
    );
    assert!(errors.is_empty(), "{errors:?}");
    let store = LocalStore::open(root).unwrap();
    evaluate(
        &version,
        &[converter],
        &store,
        derivations,
        &NoRecorder,
        threads,
    )
    .unwrap()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sekai-eval-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Counts the assets it is given and checks that their members match their documents.
#[derive(Default)]
struct CountingFusion(std::sync::atomic::AtomicUsize);

impl Fusion for CountingFusion {
    fn extra(&self, asset: &Asset, document: &AssetDocument) -> std::io::Result<()> {
        assert_eq!(asset.members.len(), document.members.len());
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

fn run_fused(root: &Path, plain: bool, fusion: &CountingFusion) -> Evaluation {
    let (version, errors) = Version::open(
        vec![input("fixture_scene"), input("fixture_data")],
        &NoBlockCache,
    );
    assert!(errors.is_empty(), "{errors:?}");
    let store = LocalStore::open(root).unwrap();
    let options = Options {
        threads: 2,
        memory: u64::MAX,
        resident: None,
        plain,
    };
    evaluate_fused(
        &version,
        &[Box::new(Texture) as Box<dyn Converter>],
        &store,
        &MemoryDerivations::default(),
        &NoRecorder,
        &options,
        Some(fusion),
    )
    .unwrap()
}

#[test]
fn fusion_sees_every_converted_asset_and_changes_nothing() {
    let (fused, plain) = (scratch("fused"), scratch("fused-plain"));
    let seen = CountingFusion::default();
    let with_fusion = run_fused(&fused, false, &seen);
    let count = seen.0.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(count, with_fusion.outputs.len());
    assert!(count > 0);
    let unused = CountingFusion::default();
    let without = run_fused(&plain, true, &unused);
    assert_eq!(unused.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(with_fusion.outputs, without.outputs);
    std::fs::remove_dir_all(fused).unwrap();
    std::fs::remove_dir_all(plain).unwrap();
}

/// The texture form, which may also read everything the bundle of the root lists and may declare
/// it.
struct Wide {
    declared: bool,
    reads: bool,
}

impl Converter for Wide {
    fn name(&self) -> &'static str {
        Texture.name()
    }

    fn version(&self) -> u32 {
        Texture.version()
    }

    fn is_root(&self, object: &ObjectIndex, script: Option<&str>, paths: &[String]) -> bool {
        Texture.is_root(object, script, paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        if self.reads {
            for (_, id) in cx.container(root)? {
                cx.object(id)?;
            }
        }
        Texture.convert(root, cx)
    }

    fn inputs(&self, root: ObjectId, cx: &dyn Context) -> Vec<ObjectId> {
        if !self.declared {
            return Vec::new();
        }
        let entries = cx.container(root).unwrap();
        entries.into_iter().map(|(_, id)| id).collect()
    }
}

fn documents(root: &Path, converter: Box<dyn Converter>) -> Vec<AssetDocument> {
    let evaluation = run_with(converter, root, 2, &MemoryDerivations::default());
    let store = LocalStore::open(root).unwrap();
    let read = |output: &sekai_pipeline::Output| {
        AssetDocument::from_bytes(&store.read(&output.document).unwrap()).unwrap()
    };
    evaluation.outputs.iter().map(read).collect()
}

#[test]
fn declared_inputs_join_the_sources_and_the_fingerprint() {
    let (narrow, wide) = (scratch("narrow"), scratch("wide"));
    let plain = documents(&narrow, Box::new(Texture));
    let declared = documents(
        &wide,
        Box::new(Wide {
            declared: true,
            reads: true,
        }),
    );
    assert!(declared.iter().all(|d| d.error.is_none()));
    let count = |all: &[AssetDocument]| all.iter().map(|d| d.sources.len()).sum::<usize>();
    assert!(count(&declared) > count(&plain));
    assert!(
        plain
            .iter()
            .zip(&declared)
            .any(|(a, b)| a.fingerprint != b.fingerprint)
    );
    std::fs::remove_dir_all(narrow).unwrap();
    std::fs::remove_dir_all(wide).unwrap();
}

#[test]
fn reading_outside_the_closure_is_a_named_error() {
    let root = scratch("undeclared");
    let found = documents(
        &root,
        Box::new(Wide {
            declared: false,
            reads: true,
        }),
    );
    let code = |d: &AssetDocument| d.error.as_ref().map(|e| e.code.clone());
    assert!(
        found
            .iter()
            .any(|d| code(d).as_deref() == Some("undeclared_read"))
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sources_list_what_was_read_and_not_what_was_declared() {
    let (narrow, wide) = (scratch("listed"), scratch("unread"));
    let plain = documents(&narrow, Box::new(Texture));
    let unread = documents(
        &wide,
        Box::new(Wide {
            declared: true,
            reads: false,
        }),
    );
    for (a, b) in plain.iter().zip(&unread) {
        assert_eq!(a.sources, b.sources);
    }
    assert!(
        plain
            .iter()
            .zip(&unread)
            .any(|(a, b)| a.fingerprint != b.fingerprint)
    );
    std::fs::remove_dir_all(narrow).unwrap();
    std::fs::remove_dir_all(wide).unwrap();
}

#[test]
fn every_named_texture_gets_a_document_and_files() {
    let root = scratch("documents");
    let evaluation = run(&root, 2, &MemoryDerivations::default());
    assert!(!evaluation.outputs.is_empty());
    assert!(evaluation.unnamed.is_empty());
    let store = LocalStore::open(&root).unwrap();
    for output in &evaluation.outputs {
        let document = AssetDocument::from_bytes(&store.read(&output.document).unwrap()).unwrap();
        assert_eq!(document.name, output.name);
        assert_eq!(document.form, "texture");
        assert_eq!(document.sources[0].class, "Texture2D");
        assert_eq!(document.members[0].role, "image");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn outputs_do_not_depend_on_the_thread_count() {
    let (a, b) = (scratch("one"), scratch("many"));
    let one = run(&a, 1, &MemoryDerivations::default());
    let many = run(&b, 8, &MemoryDerivations::default());
    assert_eq!(one, many);
    std::fs::remove_dir_all(a).unwrap();
    std::fs::remove_dir_all(b).unwrap();
}

#[test]
fn a_second_run_reuses_every_derivation() {
    let root = scratch("reuse");
    let derivations = MemoryDerivations::default();
    let first = run(&root, 2, &derivations);
    assert_eq!(first.reused, 0);
    let second = run(&root, 2, &derivations);
    assert_eq!(second.converted, 0);
    assert_eq!(second.reused, first.converted);
    assert_eq!(first.outputs, second.outputs);
    std::fs::remove_dir_all(root).unwrap();
}

struct Broken;

impl Converter for Broken {
    fn name(&self) -> &'static str {
        "broken"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(&self, object: &ObjectIndex, _: Option<&str>, _: &[String]) -> bool {
        object.class == "Texture2D"
    }

    fn convert(&self, _: ObjectId, _: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        Err(ConvertError::new("unsupported", "always"))
    }
}

#[test]
fn a_failed_asset_is_recorded_and_evaluated_again() {
    let root = scratch("failed");
    let derivations = MemoryDerivations::default();
    let first = run_with(Box::new(Broken), &root, 2, &derivations);
    assert!(!first.outputs.is_empty());
    let store = LocalStore::open(&root).unwrap();
    for output in &first.outputs {
        assert_eq!(output.error.as_deref(), Some("unsupported"));
        let document = AssetDocument::from_bytes(&store.read(&output.document).unwrap()).unwrap();
        assert!(document.members.is_empty());
    }
    let second = run_with(Box::new(Broken), &root, 2, &derivations);
    assert_eq!(second.reused, 0);
    assert_eq!(second.converted, first.converted);
    std::fs::remove_dir_all(root).unwrap();
}

fn run_options(
    converter: Box<dyn Converter>,
    root: &Path,
    options: &Options,
    derivations: &MemoryDerivations,
) -> Evaluation {
    let (version, errors) = Version::open(
        vec![input("fixture_scene"), input("fixture_data")],
        &NoBlockCache,
    );
    assert!(errors.is_empty(), "{errors:?}");
    let store = LocalStore::open(root).unwrap();
    evaluate_with(
        &version,
        &[converter],
        &store,
        derivations,
        &NoRecorder,
        options,
    )
    .unwrap()
}

#[test]
fn outputs_do_not_depend_on_how_the_run_is_scheduled() {
    let plain = Options {
        plain: true,
        ..Options::threads(8)
    };
    let small = Options {
        memory: 1,
        ..Options::threads(4)
    };
    let modes = [plain, Options::threads(1), Options::threads(8), small];
    let mut results = Vec::new();
    for (i, options) in modes.iter().enumerate() {
        let root = scratch(&format!("mode-{i}"));
        let evaluation = run_options(
            Box::new(Texture),
            &root,
            options,
            &MemoryDerivations::default(),
        );
        let files: Vec<Vec<u8>> = {
            let store = LocalStore::open(&root).unwrap();
            evaluation
                .outputs
                .iter()
                .map(|output| store.read(&output.document).unwrap())
                .collect()
        };
        results.push((evaluation, files));
        std::fs::remove_dir_all(root).unwrap();
    }
    assert!(!results[0].0.outputs.is_empty());
    for result in &results[1..] {
        assert_eq!(*result, results[0]);
    }
}

/// Counts the conversions that run at once and the threads they run on.
#[derive(Default)]
struct Probe {
    running: AtomicUsize,
    most: AtomicUsize,
    threads: Mutex<BTreeSet<String>>,
}

struct Watched(Arc<Probe>, u64);

impl Converter for Watched {
    fn name(&self) -> &'static str {
        Texture.name()
    }

    fn version(&self) -> u32 {
        Texture.version()
    }

    fn is_root(&self, object: &ObjectIndex, script: Option<&str>, paths: &[String]) -> bool {
        Texture.is_root(object, script, paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let probe = &self.0;
        let now = probe.running.fetch_add(1, Ordering::SeqCst) + 1;
        probe.most.fetch_max(now, Ordering::SeqCst);
        probe
            .threads
            .lock()
            .unwrap()
            .insert(format!("{:?}", std::thread::current().id()));
        std::thread::sleep(std::time::Duration::from_millis(20));
        let asset = Texture.convert(root, cx);
        probe.running.fetch_sub(1, Ordering::SeqCst);
        asset
    }

    fn cost(&self, _: ObjectId, _: &ObjectIndex, _: &dyn Context) -> Cost {
        Cost {
            class: CostClass::Heavy,
            memory: self.1,
        }
    }
}

fn watched(options: &Options, memory: u64) -> (Evaluation, Arc<Probe>) {
    static RUNS: AtomicUsize = AtomicUsize::new(0);
    let root = scratch(&format!("watched-{}", RUNS.fetch_add(1, Ordering::SeqCst)));
    let probe = Arc::new(Probe::default());
    let evaluation = run_options(
        Box::new(Watched(probe.clone(), memory)),
        &root,
        options,
        &MemoryDerivations::default(),
    );
    std::fs::remove_dir_all(root).unwrap();
    (evaluation, probe)
}

#[test]
fn conversions_stay_within_the_memory_budget() {
    let with = |memory| Options {
        memory,
        ..Options::threads(8)
    };
    let (evaluation, probe) = watched(&with(u64::MAX), 100);
    assert_eq!(evaluation.converted, 2);
    assert_eq!(
        probe.most.load(Ordering::SeqCst),
        2,
        "without a limit both run"
    );

    let (_, probe) = watched(&with(150), 100);
    assert_eq!(probe.most.load(Ordering::SeqCst), 1, "one fits the budget");

    let (evaluation, probe) = watched(&with(150), 1000);
    assert_eq!(
        probe.most.load(Ordering::SeqCst),
        1,
        "each above the budget runs alone"
    );
    assert_eq!(evaluation.converted, 2);
}

#[test]
fn one_slot_and_plain_mode_convert_on_the_calling_thread() {
    let caller = format!("{:?}", std::thread::current().id());
    let plain = Options {
        plain: true,
        ..Options::threads(8)
    };
    for options in [Options::threads(1), plain] {
        let (_, probe) = watched(&options, 100);
        assert_eq!(probe.most.load(Ordering::SeqCst), 1);
        assert_eq!(
            *probe.threads.lock().unwrap(),
            BTreeSet::from([caller.clone()])
        );
    }
}

#[test]
fn plain_mode_neither_reads_nor_writes_records() {
    let root = scratch("plain-records");
    let plain = Options {
        plain: true,
        ..Options::threads(2)
    };
    let derivations = MemoryDerivations::default();
    let first = run_options(Box::new(Texture), &root, &plain, &derivations);
    let normal = run(&root, 2, &derivations);
    assert_eq!(normal.reused, 0, "plain mode wrote no records");
    let again = run_options(Box::new(Texture), &root, &plain, &derivations);
    assert_eq!(again.reused, 0, "plain mode read no records");
    assert_eq!(again.converted, normal.converted);
    assert_eq!(first.outputs, again.outputs);
    std::fs::remove_dir_all(root).unwrap();
}
