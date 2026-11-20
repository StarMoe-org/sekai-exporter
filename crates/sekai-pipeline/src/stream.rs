//! The streaming evaluator: evaluates a version bundle by bundle, each bundle with the bundles its
//! manifest entry depends on.
//!
//! A unit is one bundle of the manifest together with its window: the bundle and every bundle it
//! depends on, directly or through others. The roots of a unit are the root objects of its bundle.
//! They are named from that bundle alone, and their closures reach only the window. A reference
//! that leaves the window for a file of another bundle makes the asset fail with the error
//! `outside_dependencies`, which names the bundle and the file, and is listed in
//! [`Streamed::outside`].
//!
//! Units run in manifest order, in segments of consecutive units. The bundles of the windows of a
//! segment are opened together, and the derivations of the segment run through the scheduler of
//! the run, while the next segment runs too and the bundles of the one after it are opened. A
//! bundle stays open, indexed and parsed until the last unit whose window holds it, which the
//! manifest tells in advance; the ledger of the bundle is settled then too. Since every closure
//! stays inside its own window, the result depends neither on how units are grouped into segments
//! nor on the number of threads, and publishing is a step after the last unit.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque, btree_map};
use std::fmt;
use std::io;
use std::ops::Range;
use std::panic;
use std::sync::{Arc, Mutex};
use std::thread::{self, ScopedJoinHandle};

use sekai_catalog::{BundleIndexes, Derivations, Key, Store};
use sekai_convert::{Converter, FileConverter, ObjectId};
use sekai_format::{AssetDocument, AssetError};
use sekai_unity::index::{ObjectIndex, Resolved, ResourceFile, VersionIndex};
use sekai_unity::{BlockCache, ByteSource, ClassLayouts};
use serde::{Deserialize, Serialize};

use crate::eval::{
    self, Evaluation, FileRoot, Fusion, Options, Output, Planned, Produced, RootKey, Runner,
    Subject,
};
use crate::ledger::{BundleLedger, ObjectName, Tally};
use crate::metrics::Recorder;
use crate::naming::{Candidate, Origin, assign, logical_path, loose_path, path_in_bundle};
use crate::sched::Scheduler;
use crate::version::{BundleError, BundleFiles, BundleInput, Parsed, Version, bundle_files};

/// The estimated memory of the bundles that stay parsed while a version streams, in bytes.
pub(crate) const PARSED_BYTES: u64 = 512 << 20;

/// One bundle of a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The bundle name.
    pub name: String,
    /// The size of the bundle in bytes, as the manifest declares it.
    pub size: u64,
    /// What identifies the bytes of the bundle, unless nothing does.
    pub content: Option<Content>,
    /// The names of the bundles this one depends on.
    pub dependencies: Vec<String>,
}

/// What identifies the bytes of a bundle: entries with the same name and content hold the same
/// bytes. Its text, such as `declared 1a2b`, is what keys and states hold of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub enum Content {
    /// A hash that the manifest declares for them.
    Declared(String),
    /// Their SHA-256, as the key of the bundle in the input store.
    Bytes(Key),
}

impl fmt::Display for Content {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Declared(hash) => write!(f, "declared {hash}"),
            Self::Bytes(key) => write!(f, "bytes {key}"),
        }
    }
}

impl From<Content> for String {
    fn from(content: Content) -> Self {
        content.to_string()
    }
}

impl TryFrom<String> for Content {
    type Error = String;

    fn try_from(text: String) -> Result<Self, String> {
        let unknown = || format!("{text:?} identifies no content");
        match text.split_once(' ') {
            Some(("declared", hash)) => Ok(Self::Declared(hash.to_owned())),
            Some(("bytes", key)) => key.parse().map(Self::Bytes).map_err(|_| unknown()),
            _ => Err(unknown()),
        }
    }
}

/// Where the bytes of the bundles of a manifest come from.
pub trait Inputs: Sync {
    /// The bundle of the manifest entry `name`, deobfuscated.
    fn open(&self, name: &str) -> io::Result<BundleInput>;
}

/// How the roots of a bundle are named.
pub trait Naming: Sync {
    /// The name of each root of one bundle, in the order of `candidates`. `None` leaves a root
    /// unnamed.
    fn names(&self, candidates: &[Candidate<'_>]) -> Vec<Option<String>>;
}

/// Names the roots of a bundle with [`assign`], from the roots of that bundle alone.
#[derive(Debug, Default, Clone, Copy)]
pub struct InBundle;

impl Naming for InBundle {
    fn names(&self, candidates: &[Candidate<'_>]) -> Vec<Option<String>> {
        assign(candidates)
    }
}

/// How many consecutive units a segment holds at most.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// The most units in a segment.
    pub units: usize,
    /// The most bytes, as the manifest declares them, of the bundles of the windows of a
    /// segment. A unit whose window alone is larger forms a segment of its own.
    pub bytes: u64,
}

impl Default for Segment {
    fn default() -> Self {
        Self {
            units: 1024,
            bytes: 1 << 30,
        }
    }
}

/// What a streaming evaluation reads, writes and runs with.
#[derive(Clone, Copy)]
pub struct Stream<'a> {
    /// The converters, one per asset form.
    pub converters: &'a [Box<dyn Converter>],
    /// How roots are named.
    pub naming: &'a dyn Naming,
    /// The classes whose objects without a container path are roots all the same, with the
    /// logical path that [`loose_path`] gives them. Any other object without a container path
    /// is never a root.
    pub loose: &'a [&'a str],
    /// The store of documents and member files.
    pub store: &'a dyn Store,
    /// The records of earlier derivations.
    pub derivations: &'a dyn Derivations,
    /// The stored indexes of bundles.
    pub indexes: &'a dyn BundleIndexes,
    /// The cache of decompressed blocks.
    pub cache: &'a dyn BlockCache,
    /// The layouts of engine classes, for objects stored without one.
    pub classes: &'a dyn ClassLayouts,
    /// Where spans are recorded.
    pub recorder: &'a dyn Recorder,
    /// Further files of each converted asset, outside plain mode.
    pub fusion: Option<&'a dyn Fusion>,
    /// CPU slots, the memory budget and plain mode.
    pub options: Options,
    /// The size of segments.
    pub segment: Segment,
    /// The classes and script classes that carry no asset content. With them, a ledger is kept
    /// for each bundle and listed in [`Streamed::ledgers`]; without them, none is kept.
    pub ledger: Option<&'a [&'a str]>,
    /// The most memory an asset may be estimated to need, in bytes. An asset estimated to need
    /// more fails with the error `too_large` without being converted.
    pub limit: Option<u64>,
}

/// A reference that leaves the window of its unit for a file of another bundle.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Outside {
    /// The bundle of the unit.
    pub unit: String,
    /// The bundle of the object that holds the reference.
    pub bundle: String,
    /// The path ID of the object that holds the reference.
    pub object: i64,
    /// The external file the reference points into, as the referring file names it.
    pub file: String,
    /// The path ID the reference points to.
    pub path_id: i64,
}

/// The result of a streaming evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Streamed {
    /// The assets, in name order, the roots left unnamed, and the counts of derivations. Assets
    /// of different bundles that are given the same name are left out and listed in
    /// [`Evaluation::unnamed`].
    pub evaluation: Evaluation,
    /// Bundles that could not be read, opened or indexed, and manifest entries that cannot be
    /// evaluated, in order of name and code.
    pub errors: Vec<BundleError>,
    /// References that leave the window of their unit, in order.
    pub outside: Vec<Outside>,
    /// What became of the contents of each bundle, in name order, when [`Stream::ledger`] is
    /// given.
    pub ledgers: Vec<BundleLedger>,
}

/// Fills in the dependencies of the entries of `manifest` from the bundles themselves, for a
/// source whose manifest does not list them. A bundle depends on each other bundle that holds a
/// file its serialized files name as an external file. When more than one bundle holds a file of
/// the same name, the first in name order holds it, as in a version. A bundle that cannot be read
/// or opened is given no dependencies; evaluating it reports the error. Fails only if `indexes`
/// does.
pub fn with_dependencies(
    manifest: &mut [Entry],
    inputs: &dyn Inputs,
    cache: &dyn BlockCache,
    indexes: &dyn BundleIndexes,
    threads: usize,
) -> io::Result<()> {
    let files = eval::parallel(manifest.len(), threads, |i| {
        let Ok(input) = inputs.open(&manifest[i].name) else {
            return Ok(Vec::new());
        };
        Ok(bundle_files(&input, cache, indexes)?.unwrap_or_default())
    });
    let files = files.into_iter().collect::<io::Result<Vec<_>>>()?;
    let bundles: Vec<(&str, &BundleFiles)> = manifest
        .iter()
        .map(|entry| entry.name.as_str())
        .zip(&files)
        .collect();
    let dependencies = referred_bundles(&bundles);
    for (entry, dependencies) in manifest.iter_mut().zip(dependencies) {
        entry.dependencies = dependencies;
    }
    Ok(())
}

/// The bundles each of `bundles`, given by name with its serialized files, refers to: the other
/// bundles that hold a file its files name as an external file, in name order. When more than one
/// bundle holds a file of the same name, the first in name order holds it, as in a version.
pub fn referred_bundles(bundles: &[(&str, &BundleFiles)]) -> Vec<Vec<String>> {
    let mut order: Vec<usize> = (0..bundles.len()).collect();
    order.sort_by_key(|&bundle| bundles[bundle].0);
    let mut holders: HashMap<&str, usize> = HashMap::new();
    for &bundle in &order {
        for (path, _) in bundles[bundle].1 {
            holders.entry(path.as_str()).or_insert(bundle);
        }
    }
    let referred = |bundle: usize| -> Vec<String> {
        let names: BTreeSet<&str> = bundles[bundle]
            .1
            .iter()
            .flat_map(|(_, externals)| externals)
            .filter_map(|external| holders.get(external.rsplit('/').next()?))
            .filter(|&&holder| holder != bundle)
            .map(|&holder| bundles[holder].0)
            .collect();
        names.into_iter().map(str::to_owned).collect()
    };
    (0..bundles.len()).map(referred).collect()
}

/// The names of the bundles of `manifest` in the order an evaluation first opens them: the window
/// of each unit, in name order, each bundle once. Bundles that no unit reads are not listed.
pub fn opening_order(manifest: &[Entry]) -> Vec<&str> {
    let units = Units::new(manifest);
    let mut opened = vec![false; units.entries.len()];
    let mut order = Vec::with_capacity(units.entries.len());
    for &entry in units.windows.iter().flatten() {
        if !std::mem::replace(&mut opened[entry], true) {
            order.push(units.entries[entry].name.as_str());
        }
    }
    order
}

/// The units of a manifest: each entry with its window.
pub(crate) struct Units<'m> {
    /// The entries that can be evaluated, in name order.
    pub(crate) entries: Vec<&'m Entry>,
    /// The position of each entry by name.
    pub(crate) positions: HashMap<&'m str, usize>,
    /// The window of each entry, as positions in `entries`, in order.
    pub(crate) windows: Vec<Vec<usize>>,
    /// For each entry, the last unit whose window holds it.
    pub(crate) last: Vec<usize>,
    /// Entries that cannot be evaluated or that name unknown dependencies.
    pub(crate) errors: Vec<BundleError>,
}

impl<'m> Units<'m> {
    pub(crate) fn new(manifest: &'m [Entry]) -> Self {
        let mut sorted: Vec<&Entry> = manifest.iter().collect();
        sorted.sort_by(|a, b| a.name.cmp(&b.name));
        let mut errors = Vec::new();
        let mut entries = Vec::new();
        let mut shared = HashSet::new();
        for (i, entry) in sorted.iter().enumerate() {
            if (i > 0 && sorted[i - 1].name == entry.name)
                || sorted
                    .get(i + 1)
                    .is_some_and(|next| next.name == entry.name)
            {
                shared.insert(entry.name.as_str());
                errors.push(BundleError {
                    name: entry.name.clone(),
                    code: "duplicate_name",
                    message: format!("more than one bundle is named {}", entry.name),
                });
            } else {
                entries.push(*entry);
            }
        }
        let positions: HashMap<&str, usize> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| (entry.name.as_str(), i))
            .collect();
        let mut direct = Vec::with_capacity(entries.len());
        for entry in &entries {
            let mut known = Vec::new();
            for dependency in &entry.dependencies {
                match positions.get(dependency.as_str()) {
                    Some(&position) => known.push(position),
                    None => errors.push(BundleError {
                        name: entry.name.clone(),
                        code: "unknown_dependency",
                        message: if shared.contains(dependency.as_str()) {
                            format!(
                                "{} depends on {dependency}, which more than one bundle is named",
                                entry.name
                            )
                        } else {
                            format!(
                                "{} depends on {dependency}, which the manifest does not list",
                                entry.name
                            )
                        },
                    }),
                }
            }
            direct.push(known);
        }
        let windows: Vec<Vec<usize>> = (0..entries.len())
            .map(|unit| {
                let mut window = BTreeSet::from([unit]);
                let mut pending = vec![unit];
                while let Some(entry) = pending.pop() {
                    for &dependency in &direct[entry] {
                        if window.insert(dependency) {
                            pending.push(dependency);
                        }
                    }
                }
                window.into_iter().collect()
            })
            .collect();
        let mut last = vec![0; entries.len()];
        for (unit, window) in windows.iter().enumerate() {
            for &entry in window {
                last[entry] = unit;
            }
        }
        errors.sort_by(|a, b| a.name.cmp(&b.name));
        Self {
            entries,
            positions,
            windows,
            last,
            errors,
        }
    }

    /// The end of the segment that starts at unit `start`.
    fn segment(&self, start: usize, segment: &Segment) -> usize {
        let mut bundles = HashSet::new();
        let mut bytes = 0u64;
        let mut end = start;
        while end < self.entries.len() && end - start < segment.units.max(1) {
            let added: u64 = self.windows[end]
                .iter()
                .filter(|entry| !bundles.contains(*entry))
                .map(|&entry| self.entries[entry].size)
                .sum();
            if end > start && bytes.saturating_add(added) > segment.bytes {
                break;
            }
            bytes = bytes.saturating_add(added);
            bundles.extend(self.windows[end].iter().copied());
            end += 1;
        }
        end
    }
}

/// Stored indexes, with the indexes of the bundles still to be used kept in memory as well.
struct Kept<'a> {
    inner: &'a dyn BundleIndexes,
    records: Mutex<HashMap<[u8; 32], Vec<u8>>>,
}

impl BundleIndexes for Kept<'_> {
    fn get(&self, bundle: &[u8; 32]) -> io::Result<Option<Vec<u8>>> {
        if let Some(record) = self.records.lock().unwrap().get(bundle) {
            return Ok(Some(record.clone()));
        }
        let record = self.inner.get(bundle)?;
        if let Some(record) = &record {
            self.records.lock().unwrap().insert(*bundle, record.clone());
        }
        Ok(record)
    }

    fn put(&self, bundle: [u8; 32], index: Vec<u8>) -> io::Result<()> {
        self.records.lock().unwrap().insert(bundle, index.clone());
        self.inner.put(bundle, index)
    }

    fn flush(&self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// A bundle that is open while units still need it.
struct Held {
    source: Arc<dyn ByteSource>,
    sha256: [u8; 32],
}

/// An asset of a unit, before it runs.
pub(crate) struct Root<'a> {
    pub(crate) planned: Planned<'a>,
    /// The logical path of the root.
    pub(crate) logical: String,
    /// Where the asset starts in its bundle.
    pub(crate) key: RootKey,
    /// The first reference that leaves the window, and how many do.
    outside: Option<(Outside, usize)>,
}

impl Root<'_> {
    /// Why the asset is not converted: a reference that leaves its window, or an estimate of its
    /// memory above `limit`.
    pub(crate) fn refusal(&self, version: &Version<'_>, limit: Option<u64>) -> Option<AssetError> {
        if let Some((first, count)) = &self.outside {
            return Some(AssetError {
                code: "outside_dependencies".to_owned(),
                message: format!(
                    "object {} of {} refers to object {} of {}, which no bundle that {} depends \
                     on contains ({count} such references)",
                    first.object, first.bundle, first.path_id, first.file, first.unit,
                ),
            });
        }
        let limit = limit?;
        let estimated = eval::estimated_memory(&self.planned, version);
        (estimated > limit).then(|| AssetError {
            code: "too_large".to_owned(),
            message: format!("needs about {estimated} bytes, the limit is {limit}"),
        })
    }
}

/// Everything gathered over the segments.
#[derive(Default)]
pub(crate) struct Gathered {
    /// Each asset with the logical path and key of its root.
    pub(crate) outputs: Vec<(Output, String, RootKey)>,
    /// Each root left unnamed, as its unit, logical path and key.
    pub(crate) unnamed: Vec<(String, String, RootKey)>,
    /// The forms of the named roots of each unit that has any, by unit name.
    pub(crate) forms: BTreeMap<String, BTreeSet<&'static str>>,
    converted: usize,
    reused: usize,
    pub(crate) errors: BTreeMap<(String, &'static str), BundleError>,
    pub(crate) outside: BTreeSet<Outside>,
    /// The account of each entry that units can still read from.
    tallies: BTreeMap<usize, Tally>,
    ledgers: Vec<BundleLedger>,
}

/// Evaluates every asset of the bundles of `manifest`, whose bytes come from `inputs`, one unit
/// after another. An asset that cannot be produced is recorded with its error and the run
/// continues; only a failure to read or write the store or the records fails the run.
pub fn evaluate(
    manifest: &[Entry],
    inputs: &dyn Inputs,
    stream: &Stream<'_>,
) -> io::Result<Streamed> {
    evaluate_in_segments(manifest, inputs, stream, &mut || Ok(()))
}

/// The segments whose derivations run at once with more than one CPU slot.
const RUNNING: usize = 2;

/// Evaluates as [`evaluate`] does, and calls `segment_done` after each segment, once the files of
/// its assets are in the store.
///
/// Every derivation and every bundle index of the run takes its CPU slots from one scheduler.
/// With more than one CPU slot, [`RUNNING`] segments run at once while the bundles of the next
/// one are opened, so neither the indexing nor the last derivations of a segment leave slots
/// idle. The segments are finished in order, each with what it gathered, so the result is the one
/// of evaluating them one after another.
pub fn evaluate_in_segments(
    manifest: &[Entry],
    inputs: &dyn Inputs,
    stream: &Stream<'_>,
    segment_done: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Streamed> {
    gather(manifest, inputs, stream, segment_done).map(finish)
}

/// Evaluates as [`evaluate_in_segments`] does, up to telling apart the assets of different
/// bundles that are given the same name.
pub(crate) fn gather(
    manifest: &[Entry],
    inputs: &dyn Inputs,
    stream: &Stream<'_>,
    segment_done: &mut dyn FnMut() -> io::Result<()>,
) -> io::Result<Gathered> {
    let units = Units::new(manifest);
    let runner = Runner::new(
        stream.store,
        stream.derivations,
        stream.recorder,
        stream.fusion,
        &stream.options,
    );
    let opener = Opener {
        units: &units,
        inputs,
        stream,
        scheduler: &runner.scheduler,
        parsed: Arc::new(Parsed::new(PARSED_BYTES)),
        kept: Kept {
            inner: stream.indexes,
            records: Mutex::default(),
        },
    };
    // With one CPU slot, as in plain mode, every segment runs on the calling thread.
    let alone = runner.scheduler.limits().cpu <= 1;
    let depth = if alone { 1 } else { RUNNING };
    let mut held: BTreeMap<usize, Held> = BTreeMap::new();
    let mut gathered = Gathered::default();
    for error in &units.errors {
        report(&mut gathered, error.clone());
    }
    thread::scope(|scope| {
        let start = |segment| {
            let run = || run_segment(&units, segment, stream, &runner);
            if alone {
                Running::Done(run())
            } else {
                Running::Thread(scope.spawn(run))
            }
        };
        let mut running = VecDeque::new();
        let mut next = opener.open(0, &mut held)?;
        while next.is_some() || !running.is_empty() {
            if let Some(segment) = next.take() {
                let end = segment.units.end;
                running.push_back((end, start(segment)));
                next = opener.open(end, &mut held)?;
            }
            if next.is_none() || running.len() == depth {
                let (end, segment) = running.pop_front().expect("a segment is running");
                gathered.absorb(segment.finish()?);
                opener.release(end, &mut held);
                gathered.settle(&units, end);
                segment_done()?;
            }
        }
        Ok(gathered)
    })
}

/// A segment whose derivations run, on the calling thread or on a thread of its own.
enum Running<'scope> {
    Done(io::Result<Gathered>),
    Thread(ScopedJoinHandle<'scope, io::Result<Gathered>>),
}

impl Running<'_> {
    /// What the segment gathered, once it has run.
    fn finish(self) -> io::Result<Gathered> {
        match self {
            Running::Done(gathered) => gathered,
            Running::Thread(thread) => thread
                .join()
                .unwrap_or_else(|panic| panic::resume_unwind(panic)),
        }
    }
}

/// The bundles of a segment of units, opened and indexed.
struct OpenedSegment<'a> {
    /// The units of the segment.
    units: Range<usize>,
    version: Version<'a>,
    /// The bundles of the segment that could not be read, opened or indexed.
    errors: Vec<BundleError>,
}

/// Opens the segments of a manifest, and lets go of their bundles once no later unit needs them.
struct Opener<'s, 'a> {
    units: &'s Units<'s>,
    inputs: &'s dyn Inputs,
    stream: &'s Stream<'a>,
    /// The scheduler whose CPU slots the bundles are indexed on.
    scheduler: &'s Scheduler,
    /// The parsed bundles, which the versions of all segments share.
    parsed: Arc<Parsed>,
    /// The indexes of the open bundles.
    kept: Kept<'a>,
}

impl<'a> Opener<'_, 'a> {
    /// The segment that starts at unit `start`, or `None` past the last unit. The bundles it
    /// opens are added to `held`, and the bundles already there are used as they are.
    fn open(
        &self,
        start: usize,
        held: &mut BTreeMap<usize, Held>,
    ) -> io::Result<Option<OpenedSegment<'a>>> {
        if start == self.units.entries.len() {
            return Ok(None);
        }
        let units = start..self.units.segment(start, &self.stream.segment);
        let _span = self
            .stream
            .recorder
            .span("open", "segment", &self.units.entries[start].name);
        let bundles: BTreeSet<usize> = units
            .clone()
            .flat_map(|unit| self.units.windows[unit].iter().copied())
            .collect();
        let mut errors = Vec::new();
        let mut inputs = Vec::new();
        for entry in bundles {
            let name = &self.units.entries[entry].name;
            let bundle = match held.entry(entry) {
                btree_map::Entry::Occupied(occupied) => occupied.into_mut(),
                btree_map::Entry::Vacant(vacant) => match self.inputs.open(name) {
                    Ok(input) => vacant.insert(Held {
                        source: input.source,
                        sha256: input.sha256,
                    }),
                    Err(e) => {
                        errors.push(BundleError {
                            name: name.clone(),
                            code: "unreadable",
                            message: e.to_string(),
                        });
                        continue;
                    }
                },
            };
            inputs.push(BundleInput {
                name: name.clone(),
                source: bundle.source.clone(),
                sha256: bundle.sha256,
            });
        }
        let (version, unindexed) = Version::open_on(
            inputs,
            self.stream.cache,
            self.stream.classes,
            &self.kept,
            self.scheduler,
        )?;
        errors.extend(unindexed);
        Ok(Some(OpenedSegment {
            units,
            version: version.sharing(self.parsed.clone()),
            errors,
        }))
    }

    /// Lets go of the bundles in `held` that no unit from `end` on needs.
    fn release(&self, end: usize, held: &mut BTreeMap<usize, Held>) {
        for (_, bundle) in held.extract_if(.., |&entry, _| self.units.last[entry] < end) {
            self.parsed.forget(&bundle.sha256);
        }
        let needed: HashSet<[u8; 32]> = held.values().map(|bundle| bundle.sha256).collect();
        self.kept
            .records
            .lock()
            .unwrap()
            .retain(|sha256, _| needed.contains(sha256));
    }
}

/// Each error of a bundle is reported once, however many segments meet it.
fn report(gathered: &mut Gathered, error: BundleError) {
    gathered
        .errors
        .entry((error.name.clone(), error.code))
        .or_insert(error);
}

impl Gathered {
    /// Adds what a segment gathered, after the segments before it.
    fn absorb(&mut self, segment: Gathered) {
        let Gathered {
            outputs,
            unnamed,
            forms,
            converted,
            reused,
            errors,
            outside,
            tallies,
            ledgers,
        } = segment;
        self.outputs.extend(outputs);
        self.unnamed.extend(unnamed);
        self.forms.extend(forms);
        self.converted += converted;
        self.reused += reused;
        for error in errors.into_values() {
            report(self, error);
        }
        self.outside.extend(outside);
        for (entry, tally) in tallies {
            self.tallies.entry(entry).or_default().absorb(tally);
        }
        self.ledgers.extend(ledgers);
    }

    /// Settles the ledgers of the bundles that no unit from `end` on reads.
    fn settle(&mut self, units: &Units<'_>, end: usize) {
        let settled = self
            .tallies
            .extract_if(.., |&entry, _| units.last[entry] < end);
        let ledgers: Vec<BundleLedger> = settled.flat_map(|(_, tally)| tally.settle()).collect();
        self.ledgers.extend(ledgers);
    }
}

/// Plans and runs the derivations of the units of `segment` with `runner`, and returns what they
/// gave.
fn run_segment(
    units: &Units<'_>,
    segment: OpenedSegment<'_>,
    stream: &Stream<'_>,
    runner: &Runner<'_>,
) -> io::Result<Gathered> {
    let OpenedSegment {
        units: range,
        version,
        errors,
    } = segment;
    let mut gathered = Gathered::default();
    for error in errors {
        report(&mut gathered, error);
    }
    let index = version.index();
    let position = |entry: usize| {
        let name = units.entries[entry].name.as_str();
        index
            .bundles()
            .binary_search_by(|bundle| bundle.name.as_str().cmp(name))
            .ok()
    };

    let mut roots = Vec::new();
    {
        let _span = stream
            .recorder
            .span("plan", "segment", &units.entries[range.start].name);
        for unit in range.clone() {
            let Some(bundle) = position(unit) else {
                continue;
            };
            let window: BTreeSet<usize> = units.windows[unit]
                .iter()
                .filter_map(|&entry| position(entry))
                .collect();
            let tally = stream
                .ledger
                .map(|ignored| (gathered.tallies.entry(unit).or_default(), ignored));
            let mut unnamed = Vec::new();
            let planned = roots.len();
            plan_unit(
                &version,
                bundle,
                &window,
                stream,
                tally,
                &mut roots,
                &mut gathered.outside,
                &mut unnamed,
            );
            let name = &units.entries[unit].name;
            gathered.unnamed.extend(
                unnamed
                    .into_iter()
                    .map(|(logical, key)| (name.clone(), logical, key)),
            );
            let forms: BTreeSet<&'static str> = roots[planned..]
                .iter()
                .map(|root| root.planned.converter.name())
                .collect();
            if !forms.is_empty() {
                gathered.forms.insert(name.clone(), forms);
            }
        }
    }

    // Assets that are refused fail here; the rest run.
    let mut runnable = Vec::new();
    let mut kept_roots = Vec::new();
    for root in roots {
        match root.refusal(&version, stream.limit) {
            Some(error) => {
                let stored =
                    eval::document(&root.planned, &version, stream.store, Vec::new(), error)?;
                gathered.converted += 1;
                if stream.ledger.is_some() {
                    count_reads(&mut gathered.tallies, units, &stored.document);
                }
                gathered.outputs.push((
                    Output {
                        name: root.planned.name.clone(),
                        bundle: index.bundles()[root.planned.subject.bundle()].name.clone(),
                        form: root.planned.converter.name(),
                        error: stored.error(),
                        document: stored.key,
                    },
                    root.logical,
                    root.key,
                ));
            }
            None => {
                kept_roots.push((root.logical, root.key));
                runnable.push(root.planned);
            }
        }
    }
    let ran = runner.run(&runnable, &version)?;
    gathered.converted += ran.converted;
    gathered.reused += ran.reused;
    if stream.ledger.is_some() {
        let stored = stored_documents(&ran.outputs, stream.store)?;
        for (produced, stored) in ran.outputs.iter().zip(&stored) {
            let fresh = produced.as_ref().and_then(|p| p.document.as_ref());
            if let Some(document) = fresh.or(stored.as_ref()) {
                count_reads(&mut gathered.tallies, units, document);
            }
        }
    }
    for (produced, (logical, key)) in ran.outputs.into_iter().zip(kept_roots) {
        let Some(Produced { output, .. }) = produced else {
            continue;
        };
        gathered.outputs.push((output, logical, key));
    }
    Ok(gathered)
}

/// How many stored documents are read at once. A document that is not in the local store comes
/// from the destination, one round trip each.
const DOCUMENT_READS: usize = 32;

/// The document of each asset of `produced` that an earlier run produced, read from `store`
/// [`DOCUMENT_READS`] at a time, and `None` for the others.
fn stored_documents(
    produced: &[Option<Produced>],
    store: &dyn Store,
) -> io::Result<Vec<Option<AssetDocument>>> {
    let read = |i: usize| -> io::Result<Option<AssetDocument>> {
        let Some(Produced {
            output,
            document: None,
        }) = &produced[i]
        else {
            return Ok(None);
        };
        let bytes = store.read(&output.document)?;
        AssetDocument::from_bytes(&bytes)
            .map(Some)
            .map_err(io::Error::other)
    };
    eval::parallel(produced.len(), DOCUMENT_READS, read)
        .into_iter()
        .collect()
}

/// Counts what the asset of `document` read for the ledgers of the bundles it read.
fn count_reads(tallies: &mut BTreeMap<usize, Tally>, units: &Units<'_>, document: &AssetDocument) {
    for source in &document.sources {
        if let Some(&entry) = units.positions.get(source.bundle.as_str()) {
            let object = ObjectName::new(source.file.as_deref(), source.path_id);
            tallies.entry(entry).or_default().read(object);
        }
    }
    for resource in &document.resources {
        if let Some(&entry) = units.positions.get(resource.bundle.as_str()) {
            tallies
                .entry(entry)
                .or_default()
                .read_resource(&resource.file);
        }
    }
}

/// The logical path of `object` of the bundle `unit`, whose container paths are `paths`, in the
/// directory of that bundle: from its first container path or, for an object without one, as
/// [`loose_path`] gives it with the classes `loose`. An object without a logical path is never a
/// root.
fn object_logical<'i>(
    unit: &str,
    object: &ObjectIndex,
    paths: &'i [String],
    loose: &[&str],
) -> Option<Cow<'i, str>> {
    match paths.iter().min() {
        Some(path) => Some(path_in_bundle(unit, logical_path(path))),
        None => loose_path(&object.class, object.name.as_deref(), loose)
            .map(|logical| path_in_bundle(unit, &logical).into_owned().into()),
    }
}

/// Whether one of `converters` has a root in the unit of `bundle`, whose window holds the bundles
/// `window` of `index`: an object of `bundle` with a logical path, as [`object_logical`] gives it
/// with the classes `loose`, that the converter takes as a root, or a file of `bundle` that it
/// takes as a root, as [`plan_unit`] finds them.
pub(crate) fn has_roots(
    index: &VersionIndex,
    bundle: usize,
    window: &BTreeSet<usize>,
    converters: &[&dyn Converter],
    loose: &[&str],
) -> bool {
    let unit = &index.bundles()[bundle].name;
    let mut files = index.bundles()[bundle].files.iter().enumerate();
    let object_root = files.any(|(f, file)| {
        file.objects.iter().enumerate().any(|(o, object)| {
            let paths = index.container_paths(bundle, f, object.path_id);
            if object_logical(unit, object, paths, loose).is_none() {
                return false;
            }
            let script = script_within(index, unit, bundle, f, o, window, &mut BTreeSet::new());
            let root = |converter: &&dyn Converter| converter.is_root(object, script, paths);
            converters.iter().any(root)
        })
    });
    let file_root = |file: &ResourceFile| {
        let root = |converter: &&dyn Converter| file_converter(*converter, file).is_some();
        converters.iter().any(root)
    };
    object_root || index.bundles()[bundle].resources.iter().any(file_root)
}

/// The converter of the files of `converter`, when it takes `file` as a root.
fn file_converter<'a>(
    converter: &'a dyn Converter,
    file: &ResourceFile,
) -> Option<&'a dyn FileConverter> {
    converter
        .files()
        .filter(|files| files.is_file_root(&file.path))
}

/// A root of a unit, before it is named.
struct Found<'a, 'i> {
    converter: &'a dyn Converter,
    root: FoundRoot<'a, 'i>,
    /// The logical path of the root.
    logical: Cow<'i, str>,
}

/// What a root of a unit is.
enum FoundRoot<'a, 'i> {
    /// An object.
    Object {
        id: ObjectId,
        /// Its serialized file, as [`eval::named_file`] names it.
        file: Option<&'i str>,
        object: &'i ObjectIndex,
    },
    /// A file of the bundle of the unit, with the converter of the files of its form.
    File(&'a dyn FileConverter, &'i ResourceFile),
}

impl FoundRoot<'_, '_> {
    fn origin(&self) -> Origin<'_> {
        match self {
            Self::Object { id, file, object } => Origin::Object {
                class_id: object.class_id,
                class: &object.class,
                file: *file,
                path_id: id.path_id,
            },
            Self::File(..) => Origin::File,
        }
    }

    fn key(&self) -> RootKey {
        match self {
            Self::Object { id, file, .. } => RootKey::Object {
                file: file.map(str::to_owned),
                path_id: id.path_id,
            },
            Self::File(_, file) => RootKey::File {
                file: file.path.clone(),
            },
        }
    }
}

/// Finds, names and fingerprints the roots of the unit of `bundle`, whose window holds the
/// bundles `window` of `version`, and counts the objects of `bundle` in `tally` with the ignored
/// classes it comes with.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan_unit<'a>(
    version: &Version<'_>,
    bundle: usize,
    window: &BTreeSet<usize>,
    stream: &Stream<'a>,
    mut tally: Option<(&mut Tally, &[&str])>,
    roots: &mut Vec<Root<'a>>,
    outside: &mut BTreeSet<Outside>,
    unnamed: &mut Vec<(String, RootKey)>,
) {
    let index = version.index();
    let unit = &index.bundles()[bundle].name;
    if let Some((tally, _)) = &mut tally {
        tally.open(unit, index.bundles()[bundle].resources.len() as u64);
    }
    let mut found = Vec::new();
    for (f, file) in index.bundles()[bundle].files.iter().enumerate() {
        let named = eval::named_file(&index.bundles()[bundle], f);
        for (o, object) in file.objects.iter().enumerate() {
            let paths = index.container_paths(bundle, f, object.path_id);
            let logical = object_logical(unit, object, paths, stream.loose);
            if logical.is_none() && tally.is_none() {
                continue;
            }
            // Only roots list the references their scripts make, and only an object with a
            // logical path is a root.
            let mut unlisted = BTreeSet::new();
            let listed = if logical.is_some() {
                &mut *outside
            } else {
                &mut unlisted
            };
            let script = script_within(index, unit, bundle, f, o, window, listed);
            let mut root = false;
            for converter in stream.converters {
                if converter.is_root(object, script, paths) {
                    root = true;
                    if let Some(logical) = &logical {
                        let id = ObjectId {
                            bundle,
                            file: f,
                            path_id: object.path_id,
                        };
                        found.push(Found {
                            converter: converter.as_ref(),
                            root: FoundRoot::Object {
                                id,
                                file: named,
                                object,
                            },
                            logical: logical.clone(),
                        });
                    }
                }
            }
            if let Some((tally, ignored)) = &mut tally {
                let id = ObjectName::new(named, object.path_id);
                tally.object(id, &object.class, script, root, ignored);
            }
        }
    }
    for file in &index.bundles()[bundle].resources {
        for converter in stream.converters {
            if let Some(files) = file_converter(converter.as_ref(), file) {
                found.push(Found {
                    converter: converter.as_ref(),
                    root: FoundRoot::File(files, file),
                    logical: path_in_bundle(unit, &file.path),
                });
            }
        }
    }
    let candidates: Vec<Candidate<'_>> = found
        .iter()
        .map(|found| Candidate {
            logical: found.logical.as_ref(),
            form: found.converter.name(),
            origin: found.root.origin(),
        })
        .collect();
    let names = stream.naming.names(&candidates);
    for (found, name) in found.into_iter().zip(names) {
        match name {
            Some(name) => roots.push(plan_root(
                found, name, version, bundle, window, stream, outside,
            )),
            None => unnamed.push((found.logical.into_owned(), found.root.key())),
        }
    }
}

/// The plan of the root `found` of the unit of `bundle`, named `name`, whose window holds the
/// bundles `window` of `version`. The references of its closure that leave the window are added
/// to `outside`.
fn plan_root<'a>(
    found: Found<'a, '_>,
    name: String,
    version: &Version<'_>,
    bundle: usize,
    window: &BTreeSet<usize>,
    stream: &Stream<'a>,
    outside: &mut BTreeSet<Outside>,
) -> Root<'a> {
    let index = version.index();
    let unit = &index.bundles()[bundle].name;
    let key = found.root.key();
    let mut leaving = BTreeSet::new();
    let subject = match found.root {
        FoundRoot::Object { id, .. } => {
            let declared = found.converter.inputs(id, version);
            Subject::Object(closure(index, unit, id, &declared, window, &mut leaving))
        }
        FoundRoot::File(converter, file) => Subject::File(FileRoot {
            converter,
            bundle,
            file: file.clone(),
        }),
    };
    let fingerprint = eval::fingerprint(
        found.converter,
        &name,
        index,
        &subject,
        version.all_streams(),
        stream.classes,
    );
    let first = leaving.first().cloned().map(|first| (first, leaving.len()));
    outside.extend(leaving);
    Root {
        planned: Planned {
            converter: found.converter,
            name,
            subject,
            fingerprint,
        },
        logical: found.logical.into_owned(),
        key,
        outside: first,
    }
}

/// Whether an external path names a file of a bundle, as opposed to a file of the engine.
fn names_bundle_file(path: &str) -> bool {
    path.starts_with("archive:")
}

/// The script name of object `object` of `file` of `bundle`, as [`VersionIndex::script`] gives
/// it, for a script inside `window`. A script in a file of a bundle outside the window does not
/// resolve and is added to `outside`.
fn script_within<'i>(
    index: &'i VersionIndex,
    unit: &str,
    bundle: usize,
    file: usize,
    object: usize,
    window: &BTreeSet<usize>,
    outside: &mut BTreeSet<Outside>,
) -> Option<&'i str> {
    let record = &index.bundles()[bundle].files[file].objects[object];
    let reference = record.script?;
    if reference.file_id != 0 {
        let externals = &index.bundles()[bundle].files[file].externals;
        let external = usize::try_from(reference.file_id)
            .ok()
            .and_then(|i| i.checked_sub(1))
            .and_then(|i| externals.get(i));
        let target = index.external(bundle, file, reference.file_id as usize);
        let leaves = match target {
            Some((b, _)) => !window.contains(&b),
            None => external.is_some_and(|e| names_bundle_file(&e.path)),
        };
        if leaves {
            if let Some(external) = external {
                outside.insert(Outside {
                    unit: unit.to_owned(),
                    bundle: index.bundles()[bundle].name.clone(),
                    object: record.path_id,
                    file: external.path.clone(),
                    path_id: reference.path_id,
                });
            }
            return None;
        }
    }
    index.script(bundle, file, object).flatten()
}

/// The root, the declared `inputs` and every object reachable from them through references
/// inside `window`, root first and the rest in order. References that leave the window for a file
/// of another bundle are added to `leaving`.
fn closure(
    index: &VersionIndex,
    unit: &str,
    root: ObjectId,
    inputs: &[ObjectId],
    window: &BTreeSet<usize>,
    leaving: &mut BTreeSet<Outside>,
) -> Vec<ObjectId> {
    let mut seen = BTreeSet::from([root]);
    for input in inputs {
        if window.contains(&input.bundle) {
            seen.insert(*input);
        } else {
            leaving.insert(Outside {
                unit: unit.to_owned(),
                bundle: index.bundles()[root.bundle].name.clone(),
                object: root.path_id,
                file: index.bundles()[input.bundle].files[input.file].path.clone(),
                path_id: input.path_id,
            });
        }
    }
    let mut queue: Vec<ObjectId> = seen.iter().copied().collect();
    while let Some(id) = queue.pop() {
        let Some(position) = index.position(id) else {
            continue;
        };
        let file = &index.bundles()[id.bundle].files[id.file];
        for reference in &file.objects[position].references {
            let external = || {
                usize::try_from(reference.file_id)
                    .ok()
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| file.externals.get(i))
                    .map(|e| e.path.clone())
                    .unwrap_or_default()
            };
            let leaves = match index.resolve(id, *reference) {
                Ok(Resolved::Object(target)) if window.contains(&target.bundle) => {
                    if index.position(target).is_some() && seen.insert(target) {
                        queue.push(target);
                    }
                    false
                }
                Ok(Resolved::Object(_)) => true,
                Ok(Resolved::Missing { external, .. }) => names_bundle_file(external),
                Ok(Resolved::Null) | Err(_) => false,
            };
            if leaves {
                leaving.insert(Outside {
                    unit: unit.to_owned(),
                    bundle: index.bundles()[id.bundle].name.clone(),
                    object: id.path_id,
                    file: external(),
                    path_id: reference.path_id,
                });
            }
        }
    }
    seen.remove(&root);
    std::iter::once(root).chain(seen).collect()
}

/// Orders the gathered assets by name and leaves out names that more than one asset received.
pub(crate) fn finish(gathered: Gathered) -> Streamed {
    let Gathered {
        mut outputs,
        unnamed: roots,
        forms: _,
        converted,
        reused,
        errors,
        outside,
        tallies: _,
        mut ledgers,
    } = gathered;
    let mut unnamed: Vec<(String, RootKey)> = roots
        .into_iter()
        .map(|(_, logical, key)| (logical, key))
        .collect();
    outputs.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    let mut kept = Vec::with_capacity(outputs.len());
    for group in outputs.chunk_by(|a, b| a.0.name == b.0.name) {
        if group.len() == 1 {
            kept.push(group[0].0.clone());
        } else {
            unnamed.extend(
                group
                    .iter()
                    .map(|(_, logical, key)| (logical.clone(), key.clone())),
            );
        }
    }
    unnamed.sort();
    ledgers.sort_by(|a, b| a.bundle.cmp(&b.bundle));
    Streamed {
        evaluation: Evaluation {
            outputs: kept,
            unnamed,
            converted,
            reused,
        },
        errors: errors.into_values().collect(),
        outside: outside.into_iter().collect(),
        ledgers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_content_reads_back_from_its_text() {
        let key = Key::of(b"bundle bytes", "bundle");
        for content in [Content::Declared("1a2b".to_owned()), Content::Bytes(key)] {
            let text = String::from(content.clone());
            assert_eq!(Content::try_from(text), Ok(content));
        }
        assert!(Content::try_from("1a2b".to_owned()).is_err());
        assert!(Content::try_from("bytes 1a2b".to_owned()).is_err());
    }

    fn entry(name: &str, size: u64, dependencies: &[&str]) -> Entry {
        Entry {
            name: name.to_owned(),
            size,
            content: None,
            dependencies: dependencies.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    fn names<'u>(units: &'u Units<'_>, window: &[usize]) -> Vec<&'u str> {
        window
            .iter()
            .map(|&entry| units.entries[entry].name.as_str())
            .collect()
    }

    #[test]
    fn a_window_holds_every_dependency_once() {
        let manifest = [
            entry("d", 1, &["b"]),
            entry("a", 1, &["b", "c"]),
            entry("b", 1, &["c"]),
            entry("c", 1, &["a"]),
        ];
        let units = Units::new(&manifest);
        assert!(units.errors.is_empty());
        assert_eq!(names(&units, &units.windows[0]), ["a", "b", "c"]);
        assert_eq!(names(&units, &units.windows[3]), ["a", "b", "c", "d"]);
        assert_eq!(units.last, [3, 3, 3, 3]);
    }

    #[test]
    fn the_last_use_is_the_last_unit_that_needs_a_bundle() {
        let manifest = [
            entry("a", 1, &[]),
            entry("b", 1, &["a"]),
            entry("c", 1, &[]),
            entry("d", 1, &["c"]),
            entry("e", 1, &[]),
        ];
        let units = Units::new(&manifest);
        assert_eq!(units.last, [1, 1, 3, 3, 4]);
    }

    #[test]
    fn unknown_dependencies_and_shared_names_are_errors() {
        let manifest = [
            entry("a", 1, &["gone"]),
            entry("b", 1, &[]),
            entry("b", 1, &[]),
            entry("c", 1, &["b"]),
        ];
        let units = Units::new(&manifest);
        let errors: Vec<(&str, &str)> = units
            .errors
            .iter()
            .map(|e| (e.name.as_str(), e.code))
            .collect();
        assert_eq!(
            errors,
            [
                ("a", "unknown_dependency"),
                ("b", "duplicate_name"),
                ("b", "duplicate_name"),
                ("c", "unknown_dependency"),
            ]
        );
        assert_eq!(names(&units, &units.windows[0]), ["a"]);
        assert_eq!(names(&units, &units.windows[1]), ["c"]);
    }

    #[test]
    fn a_bundle_refers_to_the_first_holder_of_each_external_file() {
        let files = |files: &[(&str, &[&str])]| -> BundleFiles {
            let owned = |paths: &[&str]| paths.iter().map(|p| (*p).to_owned()).collect();
            files
                .iter()
                .map(|(path, externals)| ((*path).to_owned(), owned(externals)))
                .collect()
        };
        let model = files(&[(
            "CAB-model",
            &[
                "archive:/CAB-shader/CAB-shader",
                "archive:/CAB-model/CAB-model",
            ],
        )]);
        let shader = files(&[(
            "CAB-shader",
            &[
                "archive:/CAB-global/CAB-global",
                "Resources/unity_builtin_extra",
            ],
        )]);
        let global = files(&[("CAB-global", &[])]);
        let copy = files(&[("CAB-global", &[])]);
        let bundles = [
            ("model", &model),
            ("shader", &shader),
            ("z/global", &copy),
            ("global", &global),
        ];
        assert_eq!(
            referred_bundles(&bundles),
            [vec!["shader"], vec!["global"], vec![], vec![]]
        );
    }

    #[test]
    fn segments_are_bounded_by_units_and_bytes() {
        let manifest = [
            entry("a", 10, &[]),
            entry("b", 10, &["a"]),
            entry("c", 50, &[]),
            entry("d", 10, &[]),
            entry("e", 10, &[]),
        ];
        let units = Units::new(&manifest);
        let by_bytes = Segment {
            units: 10,
            bytes: 30,
        };
        // a and b share a, so they take 20 bytes; c alone is larger than the bound.
        assert_eq!(units.segment(0, &by_bytes), 2);
        assert_eq!(units.segment(2, &by_bytes), 3);
        assert_eq!(units.segment(3, &by_bytes), 5);
        let by_units = Segment {
            units: 2,
            bytes: u64::MAX,
        };
        assert_eq!(units.segment(0, &by_units), 2);
        assert_eq!(units.segment(4, &by_units), 5);
    }

    #[test]
    fn bundles_are_opened_with_the_first_unit_that_reads_them() {
        let manifest = [
            entry("c", 10, &[]),
            entry("a", 10, &["d"]),
            entry("b", 10, &["d", "c"]),
            entry("d", 10, &[]),
            entry("e", 10, &[]),
            entry("e", 10, &[]),
        ];
        // Two bundles share the name e, so neither is a unit and no unit opens them.
        assert_eq!(opening_order(&manifest), ["a", "d", "b", "c"]);
    }
}
