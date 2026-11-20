//! The streaming evaluator on the Unity-built fixtures.
//!
//! The two fixture bundles refer to each other: the `data` object of `fixture_data` refers to a
//! material of `fixture_scene`.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sekai_catalog::{LocalStore, MemoryUnitRecords, UnitRecords};
use sekai_convert::image::Texture;
use sekai_convert::{
    Asset, Content, Context, ConvertError, Converter, Cost, Encoders, Member, ObjectId, ObjectIndex,
};
use sekai_format::AssetDocument;
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::naming::Candidate;
use sekai_pipeline::records::Earlier;
use sekai_pipeline::reuse::{self, EntryIndexes, NoEntryIndexes, Reuse, UnitResult};
use sekai_pipeline::state::RegionState;
use sekai_pipeline::stream::{self, Entry, InBundle, Inputs, Naming, Segment, Stream, Streamed};
use sekai_pipeline::{BundleInput, Fusion, MemoryDerivations, NoBundleIndexes, Options, Version};
use sekai_unity::index::BundleIndex;
use sekai_unity::{Bundle, ByteSource, NoBlockCache, NoClasses, StoredFile, store_files};
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

/// The indexes of the fixture bundles, as a run that indexed them before knows them.
struct FixtureIndexes(BTreeMap<String, BundleIndex>);

impl FixtureIndexes {
    fn new() -> Self {
        let inputs = vec![input("fixture_data"), input("fixture_scene")];
        let (version, errors) = Version::open(inputs, &NoBlockCache);
        assert!(errors.is_empty());
        let bundles = version.index().bundles().iter();
        Self(bundles.map(|b| (b.name.clone(), b.clone())).collect())
    }
}

impl EntryIndexes for FixtureIndexes {
    fn index(&self, entry: &Entry) -> io::Result<Option<BundleIndex>> {
        Ok(self.0.get(&entry.name).cloned())
    }
}

fn entry(name: &str, dependencies: &[&str]) -> Entry {
    Entry {
        name: name.to_owned(),
        size: 1,
        content: None,
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

/// The converter of a form as another build has it: at a later version or revision, or unable to
/// read its roots yet. Everything else is the converter's own.
struct Build {
    of: Box<dyn Converter>,
    later_versions: u32,
    later_revisions: u32,
    reads: bool,
}

impl Build {
    /// `of` at its next version, giving the same assets.
    fn next_version(of: impl Converter + 'static) -> Box<dyn Converter> {
        Self::of(of, 1, 0, true)
    }

    /// `of` at the same version and its next revision.
    fn next_revision(of: impl Converter + 'static) -> Box<dyn Converter> {
        Self::of(of, 0, 1, true)
    }

    /// `of` at the same version and revision, unable to read its roots yet.
    fn unready(of: impl Converter + 'static) -> Box<dyn Converter> {
        Self::of(of, 0, 0, false)
    }

    fn of(
        of: impl Converter + 'static,
        later_versions: u32,
        later_revisions: u32,
        reads: bool,
    ) -> Box<dyn Converter> {
        Box::new(Self {
            of: Box::new(of),
            later_versions,
            later_revisions,
            reads,
        })
    }
}

impl Converter for Build {
    fn name(&self) -> &'static str {
        self.of.name()
    }

    fn version(&self) -> u32 {
        self.of.version() + self.later_versions
    }

    fn revision(&self) -> u32 {
        self.of.revision() + self.later_revisions
    }

    fn encoders(&self) -> Encoders {
        self.of.encoders()
    }

    fn is_root(&self, object: &ObjectIndex, script: Option<&str>, paths: &[String]) -> bool {
        self.of.is_root(object, script, paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        if !self.reads {
            return Err(ConvertError::new("unsupported", "not readable yet"));
        }
        self.of.convert(root, cx)
    }

    fn inputs(&self, root: ObjectId, cx: &dyn Context) -> Vec<ObjectId> {
        self.of.inputs(root, cx)
    }

    fn cost(&self, root: ObjectId, object: &ObjectIndex, cx: &dyn Context) -> Cost {
        self.of.cost(root, object, cx)
    }
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
    loose: &'a [&'a str],
    fusion: Option<&'a dyn Fusion>,
    inputs: &'a dyn Inputs,
    /// The indexes of the bundles that earlier runs left.
    indexes: &'a dyn EntryIndexes,
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
            loose: &[],
            fusion: None,
            inputs: FIXTURES.get_or_init(Fixtures::default),
            indexes: &NoEntryIndexes,
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
    let derivations = MemoryDerivations::default();
    let stream = stream_of(run, &store, &converters, &derivations);
    stream::evaluate_in_segments(&run.manifest, run.inputs, &stream, segment_done).unwrap()
}

/// The stream of `run`, which writes to `store` and records derivations in `derivations`.
fn stream_of<'a>(
    run: &Run<'a>,
    store: &'a LocalStore,
    converters: &'a [Box<dyn Converter>],
    derivations: &'a MemoryDerivations,
) -> Stream<'a> {
    Stream {
        converters,
        naming: run.naming,
        loose: run.loose,
        store,
        derivations,
        indexes: &NoBundleIndexes,
        cache: &NoBlockCache,
        classes: &NoClasses,
        recorder: &NoRecorder,
        fusion: run.fusion,
        options: run.options,
        segment: run.segment,
        ledger: None,
        limit: run.limit,
    }
}

/// What an evaluation of `run` that takes the results `records` hold gives: the names of the
/// entries it evaluated, how many units took a recorded result, and the result. The results a
/// later run may take are recorded in `records`.
fn streamed_reusing(
    root: &Path,
    run: &Run<'_>,
    records: &MemoryUnitRecords,
) -> (Vec<String>, usize, Streamed) {
    streamed_reusing_with(root, run, records, &converters())
}

/// What [`streamed_reusing`] gives with `converters`.
fn streamed_reusing_with(
    root: &Path,
    run: &Run<'_>,
    records: &MemoryUnitRecords,
    converters: &[Box<dyn Converter>],
) -> (Vec<String>, usize, Streamed) {
    let store = LocalStore::open(root).unwrap();
    let derivations = MemoryDerivations::default();
    let stream = stream_of(run, &store, converters, &derivations);
    let reuse = Reuse::new(&run.manifest, &stream, records, 2, run.indexes).unwrap();
    let evaluated = reuse.rest().iter().map(|e| e.name.clone()).collect();
    let reused = reuse.reused();
    let outcome = reuse.evaluate(run.inputs, &stream).unwrap();
    records.put(&reusable(&outcome.results)).unwrap();
    (evaluated, reused, outcome.streamed)
}

/// The records of the `results` that a later run may take, by key.
fn reusable(results: &[UnitResult]) -> Vec<([u8; 32], Vec<u8>)> {
    let reusable = results.iter().filter_map(|result| {
        let key = result.reusable_key()?;
        Some((key.0, result.record.bytes().unwrap()))
    });
    reusable.collect()
}

/// `streamed` without the counts of derivations, which follow from what earlier runs recorded.
fn content(streamed: &Streamed) -> Streamed {
    let mut content = streamed.clone();
    content.evaluation.converted = 0;
    content.evaluation.reused = 0;
    content
}

/// Two independent units, whose bundles have the declared hashes `data` and `scene`, or none
/// when empty: the data object refers to a material of `fixture_scene`, which its unit does not
/// depend on, so that asset fails.
fn identified_manifest(data: &str, scene: &str) -> Vec<Entry> {
    let declared =
        |hash: &str| (!hash.is_empty()).then(|| stream::Content::Declared(hash.to_owned()));
    let mut manifest = vec![entry("fixture_data", &[]), entry("fixture_scene", &[])];
    manifest[0].content = declared(data);
    manifest[1].content = declared(scene);
    manifest
}

#[test]
fn recorded_units_give_the_result_of_evaluating_them() {
    let root = scratch("reuse");
    let records = MemoryUnitRecords::default();
    let run = |manifest| Run {
        manifest,
        ..Run::default()
    };
    let whole = streamed(&root, &run(identified_manifest("d1", "s1")));

    let (evaluated, reused, first) =
        streamed_reusing(&root, &run(identified_manifest("d1", "s1")), &records);
    assert_eq!(evaluated, ["fixture_data", "fixture_scene"]);
    assert_eq!(reused, 0);
    assert_eq!(content(&first), content(&whole));
    assert_eq!(records.len(), 2);

    let inputs = Fixtures::default();
    let again = Run {
        inputs: &inputs,
        ..run(identified_manifest("d1", "s1"))
    };
    let (evaluated, reused, second) = streamed_reusing(&root, &again, &records);
    assert!(evaluated.is_empty());
    assert_eq!(reused, 2);
    assert!(inputs.0.lock().unwrap().is_empty());
    assert_eq!(content(&second), content(&whole));
    assert_eq!(second.evaluation.converted, 0);

    // A new hash of one bundle evaluates its unit alone.
    let (evaluated, reused, third) =
        streamed_reusing(&root, &run(identified_manifest("d2", "s1")), &records);
    assert_eq!(evaluated, ["fixture_data"]);
    assert_eq!(reused, 1);
    assert_eq!(content(&third), content(&whole));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn units_the_last_run_left_under_the_same_key_keep_what_they_gave() {
    let root = scratch("kept");
    let store = LocalStore::open(&root).unwrap();
    let derivations = MemoryDerivations::default();
    let converters = converters();
    let records = MemoryUnitRecords::default();
    let first = Run {
        manifest: identified_manifest("d1", "s1"),
        ..Run::default()
    };
    let stream = stream_of(&first, &store, &converters, &derivations);
    let reuse = Reuse::new(&first.manifest, &stream, &records, 2, first.indexes).unwrap();
    let outcome = reuse.evaluate(first.inputs, &stream).unwrap();
    let mut state = RegionState::default();
    state.advance(Vec::new(), &outcome);

    // fixture_data has new content. No record is written, so what fixture_scene gave comes from
    // the state alone.
    let inputs = Fixtures::default();
    let second = Run {
        manifest: identified_manifest("d2", "s1"),
        inputs: &inputs,
        ..Run::default()
    };
    let stream = stream_of(&second, &store, &converters, &derivations);
    let earlier = Earlier::new(&state, &records);
    let reuse = Reuse::new(&second.manifest, &stream, &earlier, 2, second.indexes).unwrap();
    let outcome = reuse.evaluate(second.inputs, &stream).unwrap();
    assert_eq!(outcome.kept, ["fixture_scene"]);
    let units: Vec<&str> = outcome.results.iter().map(|r| r.unit.as_str()).collect();
    assert_eq!(units, ["fixture_data"]);
    assert_eq!(
        *inputs.0.lock().unwrap(),
        BTreeMap::from([("fixture_data".to_owned(), 1)])
    );

    // The claims are those of the first run, and fixture_data has its new key.
    let mut again = state.clone();
    again.advance(Vec::new(), &outcome);
    assert_eq!(again.claims, state.claims);
    let keys = |state: &RegionState| state.units.iter().map(|u| u.key).collect::<Vec<_>>();
    assert_eq!(keys(&again)[1], keys(&state)[1]);
    assert_ne!(keys(&again)[0], keys(&state)[0]);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_new_version_evaluates_again_only_the_units_with_roots_of_its_form() {
    let root = scratch("versions");
    let records = MemoryUnitRecords::default();
    let run = Run {
        manifest: identified_manifest("d1", "s1"),
        ..Run::default()
    };
    streamed_reusing(&root, &run, &records);
    assert_eq!(records.len(), 2);

    // Only fixture_data has roots of the data form.
    let renewed: Vec<Box<dyn Converter>> = vec![Box::new(Texture), Build::next_version(Data)];
    let (evaluated, reused, _) = streamed_reusing_with(&root, &run, &records, &renewed);
    assert_eq!(evaluated, ["fixture_data"]);
    assert_eq!(reused, 1);

    let (evaluated, reused, _) = streamed_reusing_with(&root, &run, &records, &renewed);
    assert!(evaluated.is_empty());
    assert_eq!(reused, 2);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_new_form_evaluates_again_only_the_units_its_index_shows_roots_of() {
    let root = scratch("added");
    let records = MemoryUnitRecords::default();
    let indexes = FixtureIndexes::new();
    let run = Run {
        manifest: identified_manifest("d1", "s1"),
        indexes: &indexes,
        ..Run::default()
    };
    let whole = streamed(&root, &run);
    let textures: Vec<Box<dyn Converter>> = vec![Box::new(Texture)];
    streamed_reusing_with(&root, &run, &records, &textures);
    assert_eq!(records.len(), 2);

    // Only fixture_data has roots of the data form, so fixture_scene keeps its result.
    let (evaluated, reused, added) = streamed_reusing(&root, &run, &records);
    assert_eq!(evaluated, ["fixture_data"]);
    assert_eq!(reused, 1);
    assert_eq!(content(&added), content(&whole));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_new_form_evaluates_again_the_units_whose_index_is_not_known() {
    let root = scratch("unindexed");
    let records = MemoryUnitRecords::default();
    let run = Run {
        manifest: identified_manifest("d1", "s1"),
        ..Run::default()
    };
    let textures: Vec<Box<dyn Converter>> = vec![Box::new(Texture)];
    streamed_reusing_with(&root, &run, &records, &textures);

    let (evaluated, reused, _) = streamed_reusing(&root, &run, &records);
    assert_eq!(evaluated, ["fixture_data", "fixture_scene"]);
    assert_eq!(reused, 0);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_asset_that_failed_is_converted_again_once_its_converter_has_a_new_revision() {
    let root = scratch("retried");
    let records = MemoryUnitRecords::default();
    let run = Run {
        manifest: identified_manifest("d1", "s1"),
        ..Run::default()
    };
    let whole = streamed(&root, &run);

    let unready: Vec<Box<dyn Converter>> = vec![Build::unready(Texture), Box::new(Data)];
    let (_, _, first) = streamed_reusing_with(&root, &run, &records, &unready);
    let failed = |streamed: &Streamed| {
        let outputs = &streamed.evaluation.outputs;
        outputs
            .iter()
            .any(|o| o.error.as_deref() == Some("unsupported"))
    };
    assert!(failed(&first));
    assert_eq!(records.len(), 2);

    // A build of the same revision takes the result with the failed textures.
    let (evaluated, reused, again) = streamed_reusing(&root, &run, &records);
    assert!(evaluated.is_empty());
    assert_eq!(reused, 2);
    assert!(failed(&again));

    // The next revision, able to read them, converts them; fixture_scene has no textures.
    let revised: Vec<Box<dyn Converter>> = vec![Build::next_revision(Texture), Box::new(Data)];
    let (evaluated, reused, second) = streamed_reusing_with(&root, &run, &records, &revised);
    assert_eq!(evaluated, ["fixture_data"]);
    assert_eq!(reused, 1);
    assert!(!failed(&second));
    assert_eq!(content(&second), content(&whole));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_unit_without_identified_contents_or_with_errors_is_not_recorded() {
    let root = scratch("unrecorded");
    let records = MemoryUnitRecords::default();
    let run = Run {
        manifest: identified_manifest("", "s1"),
        ..Run::default()
    };
    streamed_reusing(&root, &run, &records);
    assert_eq!(records.len(), 1);

    let mut manifest = identified_manifest("d1", "s1");
    manifest[0].dependencies = vec!["absent".to_owned()];
    manifest.push(entry("absent", &[]));
    manifest[2].content = Some(stream::Content::Declared("a1".to_owned()));
    let records = MemoryUnitRecords::default();
    let run = Run {
        manifest,
        ..Run::default()
    };
    let (_, _, streamed) = streamed_reusing(&root, &run, &records);
    assert!(streamed.errors.iter().any(|e| e.name == "absent"));
    // fixture_data and absent read the missing bundle; fixture_scene alone is recorded.
    assert_eq!(records.len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn every_unit_has_a_result_and_only_recordable_ones_are_reusable() {
    let root = scratch("results");
    let store = LocalStore::open(&root).unwrap();
    let derivations = MemoryDerivations::default();
    let converters = converters();
    let run = Run {
        manifest: identified_manifest("", "s1"),
        ..Run::default()
    };
    let stream = stream_of(&run, &store, &converters, &derivations);
    let records = MemoryUnitRecords::default();
    let reuse = Reuse::new(&run.manifest, &stream, &records, 2, run.indexes).unwrap();
    let reuse::Outcome {
        streamed, results, ..
    } = reuse.evaluate(run.inputs, &stream).unwrap();

    let units: Vec<(&str, bool, bool)> = results
        .iter()
        .map(|r| (r.unit.as_str(), r.key.is_some(), r.reusable))
        .collect();
    assert_eq!(
        units,
        [
            ("fixture_data", false, false),
            ("fixture_scene", true, true)
        ]
    );
    // The results give back the assets of the evaluation.
    let given = reuse::evaluation(&results, &stream).unwrap();
    assert_eq!(given.outputs, streamed.evaluation.outputs);
    assert_eq!(given.unnamed, streamed.evaluation.unnamed);
    std::fs::remove_dir_all(root).unwrap();
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

/// The names of the serialized files of the fixture bundles, by which they refer to each other.
const DATA_FILE: &str = "CAB-2e6074037da702b660ee3dbd01c9ea62";
const SCENE_FILE: &str = "CAB-9076adc37246e4c243c45fbc50ed10e1";

/// One bundle named `player` that holds the serialized file of each fixture bundle under the
/// name given with it, as the player data of a client package holds many serialized files.
struct Player(&'static [(&'static str, &'static str)]);

impl Player {
    /// The serialized file of the fixture bundle `fixture`.
    fn serialized_file(fixture: &str) -> Vec<u8> {
        let input = input(fixture);
        let bundle = Bundle::open(input.source, input.sha256).unwrap();
        let entry = bundle
            .entries()
            .iter()
            .position(|e| e.is_serialized())
            .unwrap();
        let size = usize::try_from(bundle.entries()[entry].size).unwrap();
        bundle
            .entry(entry, &NoBlockCache)
            .read(0, size)
            .unwrap()
            .to_vec()
    }
}

impl Inputs for Player {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        assert_eq!(name, "player");
        let files: Vec<StoredFile> = self
            .0
            .iter()
            .map(|&(path, fixture)| StoredFile {
                path: path.to_owned(),
                bytes: Self::serialized_file(fixture),
                serialized: true,
            })
            .collect();
        let bytes = store_files(&files).unwrap();
        Ok(BundleInput {
            name: name.to_owned(),
            sha256: Sha256::digest(&bytes).into(),
            source: Arc::new(Arc::<[u8]>::from(bytes)),
        })
    }
}

/// The `AssetBundle` objects, which no container path names, as a text member with their name.
struct Named;

impl Converter for Named {
    fn name(&self) -> &'static str {
        "named"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(&self, object: &ObjectIndex, _: Option<&str>, _: &[String]) -> bool {
        object.class == "AssetBundle"
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        Data.convert(root, cx)
    }
}

/// A run over the bundle `player` of `inputs`, whose loose `AssetBundle` objects are roots.
fn loose_run(inputs: &Player) -> Run<'_> {
    let mut player = entry("player", &[]);
    player.content = Some(stream::Content::Declared("p1".to_owned()));
    Run {
        manifest: vec![player],
        loose: &["AssetBundle"],
        inputs,
        ..Run::default()
    }
}

/// The name of each asset of `streamed` with the SHA-256 of its one member.
fn named_members(root: &Path, streamed: &Streamed) -> Vec<(String, String)> {
    let store = LocalStore::open(root).unwrap();
    let outputs = streamed.evaluation.outputs.iter();
    let member = |output: &sekai_pipeline::Output| {
        let document = AssetDocument::from_bytes(&store.read(&output.document).unwrap()).unwrap();
        let [member] = document.members.as_slice() else {
            panic!("{} has one member", output.name);
        };
        (output.name.clone(), member.sha256.clone())
    };
    outputs.map(member).collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[test]
fn loose_objects_at_one_path_id_in_two_files_are_assets_of_their_own() {
    let root = scratch("loose");
    let records = MemoryUnitRecords::default();
    let converters: Vec<Box<dyn Converter>> = vec![Box::new(Named)];
    let inputs = Player(&[(DATA_FILE, "fixture_data"), (SCENE_FILE, "fixture_scene")]);
    let expected = [
        ("player/loose/AssetBundle/fixture_data", "fixture_data"),
        ("player/loose/AssetBundle/fixture_scene", "fixture_scene"),
    ]
    .map(|(name, text)| (name.to_owned(), sha256_hex(text.as_bytes())));

    let run = loose_run(&inputs);
    let (evaluated, reused, first) = streamed_reusing_with(&root, &run, &records, &converters);
    assert_eq!((evaluated, reused), (vec!["player".to_owned()], 0));
    assert!(first.evaluation.unnamed.is_empty());
    assert_eq!(named_members(&root, &first), expected);

    let (evaluated, reused, second) = streamed_reusing_with(&root, &run, &records, &converters);
    assert_eq!((evaluated.len(), reused), (0, 1));
    assert_eq!(named_members(&root, &second), expected);

    // Without loose classes, an object no container path names is never a root.
    let without = Run {
        loose: &[],
        ..loose_run(&inputs)
    };
    let (evaluated, _, none) = streamed_reusing_with(&root, &without, &records, &converters);
    assert_eq!(evaluated, ["player"]);
    assert!(none.evaluation.outputs.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn objects_of_two_files_that_share_a_name_and_a_path_id_are_told_apart_by_their_file() {
    let root = scratch("loose-same");
    let inputs = Player(&[("CAB-a", "fixture_data"), ("CAB-b", "fixture_data")]);
    let converters: Vec<Box<dyn Converter>> = vec![Box::new(Named)];
    let records = MemoryUnitRecords::default();
    let (_, _, streamed) = streamed_reusing_with(&root, &loose_run(&inputs), &records, &converters);
    assert!(streamed.evaluation.unnamed.is_empty());
    let store = LocalStore::open(&root).unwrap();
    let named: Vec<(String, Option<String>)> = streamed
        .evaluation
        .outputs
        .iter()
        .map(|output| {
            let document = store.read(&output.document).unwrap();
            let document = AssetDocument::from_bytes(&document).unwrap();
            (output.name.clone(), document.sources[0].file.clone())
        })
        .collect();
    let name = "player/loose/AssetBundle/fixture_data";
    let expected = [
        (name.to_owned(), Some("CAB-a".to_owned())),
        (format!("{name}@CAB-b:1"), Some("CAB-b".to_owned())),
    ];
    assert_eq!(named, expected);
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
