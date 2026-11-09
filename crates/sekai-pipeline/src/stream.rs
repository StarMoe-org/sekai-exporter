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
//! segment are opened together, and the derivations of the segment run through the scheduler as
//! the eager evaluator runs them. A bundle stays open, indexed and parsed until the last unit
//! whose window holds it, which the manifest tells in advance. Since every closure stays inside
//! its own window, the result depends neither on how units are grouped into segments nor on the
//! number of threads, and publishing is a step after the last unit.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io;
use std::sync::{Arc, Mutex};

use sekai_catalog::{BundleIndexes, Derivations, Store};
use sekai_convert::{Converter, ObjectId};
use sekai_format::AssetError;
use sekai_unity::index::{Resolved, VersionIndex};
use sekai_unity::{BlockCache, ByteSource};

use crate::eval::{self, Evaluation, Fusion, Options, Output, Planned};
use crate::metrics::Recorder;
use crate::naming::{Candidate, assign, logical_path, path_in_bundle};
use crate::version::{BundleError, BundleInput, Parsed, Version};

/// The estimated memory of the bundles that stay parsed while a version streams, in bytes.
const PARSED_BYTES: u64 = 512 << 20;

/// One bundle of a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The bundle name.
    pub name: String,
    /// The size of the bundle in bytes, as the manifest declares it.
    pub size: u64,
    /// The names of the bundles this one depends on.
    pub dependencies: Vec<String>,
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
pub struct Stream<'a> {
    /// The converters, one per asset form.
    pub converters: &'a [Box<dyn Converter>],
    /// How roots are named.
    pub naming: &'a dyn Naming,
    /// The store of documents and member files.
    pub store: &'a dyn Store,
    /// The records of earlier derivations.
    pub derivations: &'a dyn Derivations,
    /// The stored indexes of bundles.
    pub indexes: &'a dyn BundleIndexes,
    /// The cache of decompressed blocks.
    pub cache: &'a dyn BlockCache,
    /// Where spans are recorded.
    pub recorder: &'a dyn Recorder,
    /// Further files of each converted asset, outside plain mode.
    pub fusion: Option<&'a dyn Fusion>,
    /// CPU slots, the memory budget and plain mode, as [`eval::evaluate_with`] takes them.
    pub options: Options,
    /// The size of segments.
    pub segment: Segment,
}

/// A reference that leaves the window of its unit for a file of another bundle.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
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
    /// The assets and counts, as the eager evaluator reports them. Assets of different bundles
    /// that are given the same name are left out and listed in [`Evaluation::unnamed`].
    pub evaluation: Evaluation,
    /// Bundles that could not be read, opened or indexed, and manifest entries that cannot be
    /// evaluated, in order of name and code.
    pub errors: Vec<BundleError>,
    /// References that leave the window of their unit, in order.
    pub outside: Vec<Outside>,
}

/// The units of a manifest: each entry with its window.
struct Units<'m> {
    /// The entries that can be evaluated, in name order.
    entries: Vec<&'m Entry>,
    /// The window of each entry, as positions in `entries`, in order.
    windows: Vec<Vec<usize>>,
    /// For each entry, the last unit whose window holds it.
    last: Vec<usize>,
    /// Entries that cannot be evaluated or that name unknown dependencies.
    errors: Vec<BundleError>,
}

impl<'m> Units<'m> {
    fn new(manifest: &'m [Entry]) -> Self {
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

/// An asset of a unit, before or after it runs.
struct Root<'a> {
    planned: Planned<'a>,
    logical: String,
    path_id: i64,
    /// The first reference that leaves the window, and how many do.
    outside: Option<(Outside, usize)>,
}

/// Everything gathered over the segments.
#[derive(Default)]
struct Gathered {
    /// Each asset with the logical path and path ID of its root.
    outputs: Vec<(Output, String, i64)>,
    unnamed: Vec<(String, i64)>,
    converted: usize,
    reused: usize,
    errors: BTreeMap<(String, &'static str), BundleError>,
    outside: BTreeSet<Outside>,
}

/// Evaluates every asset of the bundles of `manifest`, whose bytes come from `inputs`, one unit
/// after another. An asset that cannot be produced is recorded with its error and the run
/// continues; only a failure to read or write the store or the records fails the run.
pub fn evaluate(
    manifest: &[Entry],
    inputs: &dyn Inputs,
    stream: &Stream<'_>,
) -> io::Result<Streamed> {
    let units = Units::new(manifest);
    let parsed = Arc::new(Parsed::new(PARSED_BYTES));
    let kept = Kept {
        inner: stream.indexes,
        records: Mutex::default(),
    };
    let mut held: BTreeMap<usize, Held> = BTreeMap::new();
    let mut gathered = Gathered::default();
    for error in &units.errors {
        gathered
            .errors
            .entry((error.name.clone(), error.code))
            .or_insert_with(|| error.clone());
    }
    let mut start = 0;
    while start < units.entries.len() {
        let end = units.segment(start, &stream.segment);
        evaluate_segment(
            &units,
            start..end,
            inputs,
            stream,
            &parsed,
            &kept,
            &mut held,
            &mut gathered,
        )?;
        // The bundles no later unit needs are let go.
        let done: Vec<usize> = held
            .keys()
            .copied()
            .filter(|&entry| units.last[entry] < end)
            .collect();
        for entry in done {
            if let Some(bundle) = held.remove(&entry) {
                parsed.forget(&bundle.sha256);
            }
        }
        let needed: HashSet<[u8; 32]> = held.values().map(|bundle| bundle.sha256).collect();
        kept.records
            .lock()
            .unwrap()
            .retain(|sha256, _| needed.contains(sha256));
        start = end;
    }
    Ok(finish(gathered))
}

/// Each error of a bundle is reported once, however many segments meet it.
fn report(gathered: &mut Gathered, error: BundleError) {
    gathered
        .errors
        .entry((error.name.clone(), error.code))
        .or_insert(error);
}

#[allow(clippy::too_many_arguments)]
fn evaluate_segment(
    units: &Units<'_>,
    segment: std::ops::Range<usize>,
    inputs: &dyn Inputs,
    stream: &Stream<'_>,
    parsed: &Arc<Parsed>,
    kept: &Kept<'_>,
    held: &mut BTreeMap<usize, Held>,
    gathered: &mut Gathered,
) -> io::Result<()> {
    let threads = if stream.options.plain {
        1
    } else {
        stream.options.threads
    };
    let bundles: BTreeSet<usize> = segment
        .clone()
        .flat_map(|unit| units.windows[unit].iter().copied())
        .collect();
    let mut opened = Vec::new();
    for &entry in &bundles {
        let name = &units.entries[entry].name;
        if let std::collections::btree_map::Entry::Vacant(vacant) = held.entry(entry) {
            match inputs.open(name) {
                Ok(input) => {
                    vacant.insert(Held {
                        source: input.source,
                        sha256: input.sha256,
                    });
                }
                Err(e) => {
                    report(
                        gathered,
                        BundleError {
                            name: name.clone(),
                            code: "unreadable",
                            message: e.to_string(),
                        },
                    );
                    continue;
                }
            }
        }
        let bundle = &held[&entry];
        opened.push(BundleInput {
            name: name.clone(),
            source: bundle.source.clone(),
            sha256: bundle.sha256,
        });
    }
    let (version, errors) = Version::open_indexed(opened, stream.cache, kept, threads)?;
    let version = version.sharing(parsed.clone());
    for error in errors {
        report(gathered, error);
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
            .span("plan", "segment", &units.entries[segment.start].name);
        let resources: Vec<Vec<[u8; 32]>> = (0..index.bundles().len())
            .map(|b| version.resource_hashes(b))
            .collect();
        for unit in segment.clone() {
            let Some(bundle) = position(unit) else {
                continue;
            };
            let window: BTreeSet<usize> = units.windows[unit]
                .iter()
                .filter_map(|&entry| position(entry))
                .collect();
            plan_unit(
                &version,
                bundle,
                &window,
                &resources,
                stream,
                &mut roots,
                &mut gathered.outside,
                &mut gathered.unnamed,
            );
        }
    }

    // Assets with references outside their window fail here; the rest run.
    let mut runnable = Vec::new();
    let mut kept_roots = Vec::new();
    for root in roots {
        match &root.outside {
            Some((first, count)) => {
                let error = AssetError {
                    code: "outside_dependencies".to_owned(),
                    message: format!(
                        "object {} of {} refers to object {} of {}, which no bundle that {} \
                         depends on contains ({count} such references)",
                        first.object, first.bundle, first.path_id, first.file, first.unit,
                    ),
                };
                let document =
                    eval::document(&root.planned, &version, stream.store, Vec::new(), error)?;
                gathered.converted += 1;
                gathered.outputs.push((
                    Output {
                        name: root.planned.name.clone(),
                        form: root.planned.converter.name(),
                        document,
                        error: Some("outside_dependencies".to_owned()),
                    },
                    root.logical,
                    root.path_id,
                ));
            }
            None => {
                kept_roots.push((root.logical, root.path_id));
                runnable.push(root.planned);
            }
        }
    }
    let fusion = stream.fusion.filter(|_| !stream.options.plain);
    let ran = eval::run_each(
        &runnable,
        &version,
        stream.store,
        stream.derivations,
        stream.recorder,
        &stream.options,
        fusion,
    )?;
    gathered.converted += ran.converted;
    gathered.reused += ran.reused;
    for (output, (logical, path_id)) in ran.outputs.into_iter().zip(kept_roots) {
        if let Some(output) = output {
            gathered.outputs.push((output, logical, path_id));
        }
    }
    Ok(())
}

/// Finds, names and fingerprints the roots of the unit of `bundle`, whose window holds the
/// bundles `window` of `version`.
#[allow(clippy::too_many_arguments)]
fn plan_unit<'a>(
    version: &Version<'_>,
    bundle: usize,
    window: &BTreeSet<usize>,
    resources: &[Vec<[u8; 32]>],
    stream: &Stream<'a>,
    roots: &mut Vec<Root<'a>>,
    outside: &mut BTreeSet<Outside>,
    unnamed: &mut Vec<(String, i64)>,
) {
    let index = version.index();
    let unit = &index.bundles()[bundle].name;
    let mut found = Vec::new();
    for (f, file) in index.bundles()[bundle].files.iter().enumerate() {
        for (o, object) in file.objects.iter().enumerate() {
            let paths = index.container_paths(bundle, f, object.path_id);
            let Some(path) = paths.iter().min() else {
                continue;
            };
            let script = script_within(index, unit, bundle, f, o, window, outside);
            let id = ObjectId {
                bundle,
                file: f,
                path_id: object.path_id,
            };
            for converter in stream.converters {
                if converter.is_root(object, script, paths) {
                    found.push((
                        converter.as_ref(),
                        id,
                        object,
                        path_in_bundle(unit, logical_path(path)),
                    ));
                }
            }
        }
    }
    let candidates: Vec<Candidate<'_>> = found
        .iter()
        .map(|(_, id, object, logical)| Candidate {
            logical: logical.as_ref(),
            class_id: object.class_id,
            class: &object.class,
            bundle: unit,
            path_id: id.path_id,
        })
        .collect();
    let names = stream.naming.names(&candidates);
    for ((converter, id, _, logical), name) in found.iter().zip(names) {
        let Some(name) = name else {
            unnamed.push((logical.to_string(), id.path_id));
            continue;
        };
        let mut leaving = BTreeSet::new();
        let declared = converter.inputs(*id, version);
        let closure = closure(index, unit, *id, &declared, window, &mut leaving);
        let fingerprint = eval::fingerprint(*converter, &name, index, &closure, resources);
        let first = leaving.first().cloned().map(|first| (first, leaving.len()));
        outside.extend(leaving);
        roots.push(Root {
            planned: Planned {
                converter: *converter,
                name,
                closure,
                fingerprint,
            },
            logical: logical.to_string(),
            path_id: id.path_id,
            outside: first,
        });
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
/// inside `window`, root first and the rest in order, as the eager evaluator computes a closure
/// over a version of the window alone. References that leave the window for a file of another
/// bundle are added to `leaving`.
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
fn finish(gathered: Gathered) -> Streamed {
    let Gathered {
        mut outputs,
        mut unnamed,
        converted,
        reused,
        errors,
        outside,
    } = gathered;
    outputs.sort_by(|a, b| a.0.name.cmp(&b.0.name));
    let mut kept = Vec::with_capacity(outputs.len());
    for group in outputs.chunk_by(|a, b| a.0.name == b.0.name) {
        if group.len() == 1 {
            kept.push(group[0].0.clone());
        } else {
            unnamed.extend(
                group
                    .iter()
                    .map(|(_, logical, path_id)| (logical.clone(), *path_id)),
            );
        }
    }
    unnamed.sort();
    Streamed {
        evaluation: Evaluation {
            outputs: kept,
            unnamed,
            converted,
            reused,
        },
        errors: errors.into_values().collect(),
        outside: outside.into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, size: u64, dependencies: &[&str]) -> Entry {
        Entry {
            name: name.to_owned(),
            size,
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
}
