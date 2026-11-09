//! The streaming evaluator on the Unity-built fixtures.
//!
//! The two fixture bundles refer to each other: the `data` object of `fixture_data` refers to a
//! material of `fixture_scene`.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sekai_catalog::LocalStore;
use sekai_convert::image::Texture;
use sekai_convert::{
    Asset, Content, Context, ConvertError, Converter, Member, ObjectId, ObjectIndex,
};
use sekai_format::AssetDocument;
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::naming::Candidate;
use sekai_pipeline::stream::{self, Entry, InBundle, Inputs, Naming, Segment, Stream, Streamed};
use sekai_pipeline::{BundleInput, Fusion, MemoryDerivations, NoBundleIndexes, Options};
use sekai_unity::{NoBlockCache, NoClasses};
use sha2::{Digest, Sha256};

fn input(name: &str) -> BundleInput {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sekai-unity/tests/fixtures/lz4")
        .join(name);
    let bytes = std::fs::read(path).unwrap();
    BundleInput {
        name: name.to_owned(),
        sha256: Sha256::digest(&bytes).into(),
        source: Arc::new(Arc::<[u8]>::from(bytes)),
    }
}

/// The fixture bundles, counting how often each is opened.
#[derive(Default)]
struct Fixtures(Mutex<BTreeMap<String, usize>>);

impl Inputs for Fixtures {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        *self.0.lock().unwrap().entry(name.to_owned()).or_default() += 1;
        if name == "absent" {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no such bundle"));
        }
        Ok(input(name))
    }
}

fn entry(name: &str, dependencies: &[&str]) -> Entry {
    Entry {
        name: name.to_owned(),
        size: 1,
        dependencies: dependencies.iter().map(|d| (*d).to_owned()).collect(),
    }
}

/// Each bundle depending on the other, as the fixtures refer to each other.
fn manifest() -> Vec<Entry> {
    vec![
        entry("fixture_data", &["fixture_scene"]),
        entry("fixture_scene", &["fixture_data"]),
    ]
}

/// The objects of the `FixtureData` script, as a text member with their name.
struct Data;

impl Converter for Data {
    fn name(&self) -> &'static str {
        "data"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(&self, _: &ObjectIndex, script: Option<&str>, _: &[String]) -> bool {
        script == Some("Assembly-CSharp/FixtureData")
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let name = object
            .fields
            .get("m_Name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        Ok(Some(Asset {
            members: vec![Member {
                role: "text".to_owned(),
                content: Content::File {
                    extension: "txt".to_owned(),
                    bytes: name.as_bytes().to_vec(),
                },
            }],
        }))
    }
}

fn converters() -> Vec<Box<dyn Converter>> {
    vec![Box::new(Texture), Box::new(Data)]
}

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("sekai-stream-{}-{n}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The relative path and bytes of every file under `root`.
fn files(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for item in std::fs::read_dir(dir).unwrap() {
            let path = item.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                out.insert(path.strip_prefix(root).unwrap().to_owned(), bytes);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

struct Run<'a> {
    manifest: Vec<Entry>,
    options: Options,
    segment: Segment,
    naming: &'a dyn Naming,
    fusion: Option<&'a dyn Fusion>,
    inputs: &'a Fixtures,
    limit: Option<u64>,
}

impl Default for Run<'_> {
    fn default() -> Self {
        static FIXTURES: std::sync::OnceLock<Fixtures> = std::sync::OnceLock::new();
        Self {
            manifest: manifest(),
            options: Options::threads(2),
            segment: Segment::default(),
            naming: &InBundle,
            fusion: None,
            inputs: FIXTURES.get_or_init(Fixtures::default),
            limit: None,
        }
    }
}

fn streamed(root: &Path, run: &Run<'_>) -> Streamed {
    streamed_in_segments(root, run, &mut || Ok(()))
}

fn streamed_in_segments(
    root: &Path,
    run: &Run<'_>,
    segment_done: &mut dyn FnMut() -> io::Result<()>,
) -> Streamed {
    let store = LocalStore::open(root).unwrap();
    let converters = converters();
    let stream = Stream {
        converters: &converters,
        naming: run.naming,
        store: &store,
        derivations: &MemoryDerivations::default(),
        indexes: &NoBundleIndexes,
        cache: &NoBlockCache,
        classes: &NoClasses,
        recorder: &NoRecorder,
        fusion: run.fusion,
        options: run.options,
        segment: run.segment,
        ledger: None,
        limit: run.limit,
    };
    stream::evaluate_in_segments(&run.manifest, run.inputs, &stream, segment_done).unwrap()
}

#[test]
fn dependencies_are_read_from_the_bundles_when_the_manifest_lists_none() {
    let mut derived = vec![entry("fixture_data", &[]), entry("fixture_scene", &[])];
    stream::with_dependencies(
        &mut derived,
        &Fixtures::default(),
        &NoBlockCache,
        &NoBundleIndexes,
        2,
    )
    .unwrap();
    assert_eq!(derived, manifest());

    let (a, b) = (scratch("listed"), scratch("derived"));
    let listed = streamed(&a, &Run::default());
    let read = streamed(
        &b,
        &Run {
            manifest: derived,
            ..Run::default()
        },
    );
    assert!(listed.errors.is_empty(), "{:?}", listed.errors);
    assert!(listed.outside.is_empty(), "{:?}", listed.outside);
    let forms: BTreeSet<&str> = listed.evaluation.outputs.iter().map(|o| o.form).collect();
    assert_eq!(forms, BTreeSet::from(["data", "texture"]));
    assert!(listed.evaluation.outputs.iter().all(|o| o.error.is_none()));
    assert_eq!(read, listed);
    assert_eq!(files(&a), files(&b));
    std::fs::remove_dir_all(a).unwrap();
    std::fs::remove_dir_all(b).unwrap();
}

#[test]
fn unreadable_bundles_are_given_no_dependencies() {
    let mut derived = vec![entry("absent", &[]), entry("fixture_scene", &[])];
    stream::with_dependencies(
        &mut derived,
        &Fixtures::default(),
        &NoBlockCache,
        &NoBundleIndexes,
        1,
    )
    .unwrap();
    // fixture_scene names a file of fixture_data, which the manifest does not list.
    assert_eq!(derived, [entry("absent", &[]), entry("fixture_scene", &[])]);
}

#[test]
fn each_segment_is_reported_once_its_files_are_written() {
    let root = scratch("reported");
    let run = Run {
        segment: Segment {
            units: 1,
            bytes: u64::MAX,
        },
        ..Run::default()
    };
    let mut counts = Vec::new();
    let streamed = streamed_in_segments(&root, &run, &mut || {
        counts.push(files(&root).len());
        Ok(())
    });
    assert_eq!(counts.len(), 2);
    assert!(counts[0] > 0);
    assert_eq!(counts[1], files(&root).len());
    assert!(!streamed.evaluation.outputs.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_limit_refuses_assets_without_converting_them() {
    let root = scratch("limit");
    let streamed = streamed(
        &root,
        &Run {
            limit: Some(0),
            ..Run::default()
        },
    );
    let outputs = &streamed.evaluation.outputs;
    assert!(!outputs.is_empty());
    assert!(
        outputs
            .iter()
            .all(|o| o.error.as_deref() == Some("too_large"))
    );
    let store = LocalStore::open(&root).unwrap();
    for output in outputs {
        let document = AssetDocument::from_bytes(&store.read(&output.document).unwrap()).unwrap();
        assert!(document.members.is_empty());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn segments_threads_and_plain_mode_do_not_change_the_result() {
    let runs = [
        (Options::threads(1), Segment::default()),
        (
            Options::threads(4),
            Segment {
                units: 1,
                bytes: u64::MAX,
            },
        ),
        (
            Options::threads(4),
            Segment {
                units: 100,
                bytes: 0,
            },
        ),
        (
            Options {
                plain: true,
                ..Options::threads(4)
            },
            Segment::default(),
        ),
    ];
    let mut results = Vec::new();
    for (options, segment) in runs {
        let root = scratch("segments");
        let run = Run {
            options,
            segment,
            ..Run::default()
        };
        results.push((streamed(&root, &run), files(&root)));
        std::fs::remove_dir_all(root).unwrap();
    }
    for result in &results[1..] {
        assert_eq!(result.0.evaluation.outputs, results[0].0.evaluation.outputs);
        assert_eq!(result.1, results[0].1);
    }
}

#[test]
fn a_dependency_is_opened_once_and_kept_until_its_last_unit() {
    let inputs = Fixtures::default();
    let root = scratch("opened");
    let run = Run {
        manifest: vec![
            entry("fixture_data", &["fixture_scene"]),
            entry("fixture_scene", &[]),
        ],
        segment: Segment {
            units: 1,
            bytes: u64::MAX,
        },
        inputs: &inputs,
        ..Run::default()
    };
    let streamed = streamed(&root, &run);
    let opened = inputs.0.lock().unwrap().clone();
    assert_eq!(
        opened,
        BTreeMap::from([
            ("fixture_data".to_owned(), 1),
            ("fixture_scene".to_owned(), 1)
        ])
    );
    // fixture_scene refers back to fixture_data, which it does not depend on.
    assert!(
        streamed
            .outside
            .iter()
            .all(|o| o.unit == "fixture_scene" && o.bundle == "fixture_scene")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_reference_outside_the_dependencies_fails_the_asset_with_its_name() {
    let root = scratch("outside");
    let run = Run {
        manifest: vec![entry("fixture_data", &[]), entry("fixture_scene", &[])],
        ..Run::default()
    };
    let streamed = streamed(&root, &run);
    let data = streamed
        .evaluation
        .outputs
        .iter()
        .find(|o| o.form == "data")
        .expect("the data asset is listed");
    assert_eq!(data.error.as_deref(), Some("outside_dependencies"));
    let store = LocalStore::open(&root).unwrap();
    let document = AssetDocument::from_bytes(&store.read(&data.document).unwrap()).unwrap();
    let error = document.error.unwrap();
    assert!(error.message.contains("fixture_data"), "{}", error.message);
    assert!(error.message.contains("archive:/CAB-"), "{}", error.message);
    assert!(document.members.is_empty());
    let from_data: Vec<_> = streamed
        .outside
        .iter()
        .filter(|o| o.unit == "fixture_data")
        .collect();
    assert!(!from_data.is_empty());
    assert!(
        from_data
            .iter()
            .all(|o| o.file.starts_with("archive:/CAB-"))
    );
    // With the dependency declared, the same asset succeeds.
    let declared = streamed_data_error(&Run {
        manifest: vec![
            entry("fixture_data", &["fixture_scene"]),
            entry("fixture_scene", &[]),
        ],
        ..Run::default()
    });
    assert_eq!(declared, None);
    std::fs::remove_dir_all(root).unwrap();
}

fn streamed_data_error(run: &Run<'_>) -> Option<String> {
    let root = scratch("declared");
    let result = streamed(&root, run);
    std::fs::remove_dir_all(root).unwrap();
    result
        .evaluation
        .outputs
        .into_iter()
        .find(|o| o.form == "data")
        .expect("the data asset is listed")
        .error
}

#[test]
fn unreadable_bundles_and_unknown_dependencies_are_listed() {
    let root = scratch("errors");
    let run = Run {
        manifest: vec![
            entry("absent", &[]),
            entry("fixture_data", &["fixture_scene", "unlisted"]),
            entry("fixture_scene", &["fixture_data"]),
        ],
        inputs: &Fixtures::default(),
        ..Run::default()
    };
    let streamed = streamed(&root, &run);
    let errors: Vec<(&str, &str)> = streamed
        .errors
        .iter()
        .map(|e| (e.name.as_str(), e.code))
        .collect();
    assert_eq!(
        errors,
        [
            ("absent", "unreadable"),
            ("fixture_data", "unknown_dependency")
        ]
    );
    assert!(!streamed.evaluation.outputs.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

/// Gives every root the same name.
struct OneName;

impl Naming for OneName {
    fn names(&self, candidates: &[Candidate<'_>]) -> Vec<Option<String>> {
        vec![Some("same".to_owned()); candidates.len()]
    }
}

#[test]
fn a_name_given_more_than_once_is_left_unnamed() {
    let root = scratch("names");
    let run = Run {
        naming: &OneName,
        ..Run::default()
    };
    let streamed = streamed(&root, &run);
    assert!(streamed.evaluation.outputs.is_empty());
    assert!(streamed.evaluation.unnamed.len() > 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[derive(Default)]
struct CountingFusion(AtomicUsize);

impl Fusion for CountingFusion {
    fn extra(&self, asset: &Asset, document: &AssetDocument) -> io::Result<()> {
        assert_eq!(asset.members.len(), document.members.len());
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[test]
fn fusion_sees_every_converted_asset_outside_plain_mode() {
    let (a, b) = (scratch("fused"), scratch("fused-plain"));
    let seen = CountingFusion::default();
    let fused = streamed(
        &a,
        &Run {
            fusion: Some(&seen),
            ..Run::default()
        },
    );
    assert_eq!(
        seen.0.load(Ordering::SeqCst),
        fused.evaluation.outputs.len()
    );
    let unused = CountingFusion::default();
    let plain = streamed(
        &b,
        &Run {
            fusion: Some(&unused),
            options: Options {
                plain: true,
                ..Options::threads(2)
            },
            ..Run::default()
        },
    );
    assert_eq!(unused.0.load(Ordering::SeqCst), 0);
    assert_eq!(fused.evaluation.outputs, plain.evaluation.outputs);
    std::fs::remove_dir_all(a).unwrap();
    std::fs::remove_dir_all(b).unwrap();
}
