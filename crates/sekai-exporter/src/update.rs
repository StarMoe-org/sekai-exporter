//! `sekai-exporter update`: the current version of a region, from its source to a publication.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use sekai_catalog::db::{BundleRow, Catalog, CatalogDerivations, CostRow, IndexCache};
use sekai_catalog::store::{ObjectWriter, parse_hex};
use sekai_catalog::{Key, LocalStore, MemoryHoldings, Objects, Store};
use sekai_convert::Converter;
use sekai_convert::tables::{AssetEntry, TableConverter};
use sekai_format::AssetDocument;
use sekai_format::identity::VersionIdentity;
use sekai_format::publication::Pointer;
use sekai_game::cdn::{self, BundleEntry, Kind, Region, Secrets, Source, Tables};
use sekai_game::obfuscation::Deobfuscating;
use sekai_game::sink::Sink;
use sekai_pipeline::claims::Claims;
use sekai_pipeline::eval::Output;
use sekai_pipeline::metrics::{NoRecorder, Recorder};
use sekai_pipeline::records::{Earlier, Records};
use sekai_pipeline::reuse::{self, Reuse, UnitResult};
use sekai_pipeline::state::{
    BundleState, MemoryFiles, RegionState, Root, StateFiles, TableOutput, Written,
};
use sekai_pipeline::stream::{self, Entry, InBundle, Inputs, Segment, Stream};
use sekai_pipeline::tables::{TableStage, VersionAssets};
use sekai_pipeline::{BundleInput, parallel};
use sekai_publish::patch::{self, Change, Lookup, Previous};
use sekai_publish::{Binding, Layout, Object, Publication};
use sekai_unity::{BoundedBlockCache, FileSource, NoClasses};

use crate::bucket::Bucket;
use crate::budget;
use crate::export::{BLOCK_CACHE_BYTES, IGNORED, converters, report, write_ledger};
use crate::monitor::{self, Monitor};

mod backed;
mod files;
mod site;
mod views;

use backed::Backed;
use site::Site;
use views::Advance;

const INPUT_EXTENSION: &str = "bundle";
/// How many fetched bundles are recorded together.
const RECORD_BATCH: usize = 1024;

struct Options {
    region: String,
    output: PathBuf,
    config: PathBuf,
    secrets: Option<PathBuf>,
    only: Vec<String>,
    threads: usize,
    memory: Option<u64>,
    plain: bool,
    force: bool,
    /// Whether the bucket is checked against a listing of its store, trusting nothing else.
    verify: bool,
    /// Whether the version of the source is only compared with the last one recorded.
    check: bool,
}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Options, lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let mut positional: Vec<OsString> = Vec::new();
    let mut config = None;
    let mut secrets = None;
    let mut only = Vec::new();
    let mut threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut memory = None;
    let mut plain = false;
    let mut force = false;
    let mut verify = false;
    let mut check = false;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("config") => config = Some(PathBuf::from(parser.value()?)),
            Long("secrets") => secrets = Some(PathBuf::from(parser.value()?)),
            Long("only") => only.push(parser.value()?.string()?),
            Long("threads") => threads = parser.value()?.parse()?,
            Long("memory") => memory = Some(budget::parse_bytes(&parser.value()?.string()?)?),
            Long("plain") => plain = true,
            Long("force") => force = true,
            Long("verify") => verify = true,
            Long("check") => check = true,
            Value(v) => positional.push(v),
            _ => return Err(arg.unexpected()),
        }
    }
    let [region, output]: [OsString; 2] = positional
        .try_into()
        .map_err(|_| lexopt::Error::from("expected <region> and <output>"))?;
    if verify && !only.is_empty() {
        return Err("--verify checks a bucket, and a selection is not sent to one".into());
    }
    let acts = [
        ("--force", force),
        ("--verify", verify),
        ("--only", !only.is_empty()),
    ];
    if let Some((flag, _)) = acts.into_iter().find(|(_, given)| check && *given) {
        return Err(format!("--check only compares the version, and {flag} acts on it").into());
    }
    Ok(Options {
        region: region.string()?,
        output: PathBuf::from(output),
        config: config.ok_or("--config is required")?,
        secrets,
        only,
        threads: threads.max(1),
        memory,
        plain,
        force,
        verify,
        check,
    })
}

pub(crate) fn run(args: impl Iterator<Item = OsString>) -> Result<bool, lexopt::Error> {
    let options = parse(args)?;
    if options.check {
        return check(&options);
    }
    std::fs::create_dir_all(&options.output)
        .map_err(|e| format!("{}: {e}", options.output.display()))?;
    Monitor::watch(&options.output.join("metrics.jsonl"), |monitor| {
        update(&options, monitor)
    })
    .map_err(|e| lexopt::Error::from(e.to_string()))?
}

/// The bytes of a bundle on their way into the input store.
struct Incoming<'s>(ObjectWriter<'s>);

impl Write for Incoming<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Sink for Incoming<'_> {
    fn reset(&mut self) -> io::Result<()> {
        self.0.reset()
    }
}

/// The entries whose names start with one of `only`, with everything they depend on. Without
/// prefixes, every entry.
fn select(mut entries: Vec<Entry>, only: &[String]) -> Vec<Entry> {
    if let Some(selected) = selected(&entries, only) {
        entries.retain(|entry| selected.contains(&entry.name));
    }
    entries
}

/// The names of the entries whose names start with one of `only`, with everything they depend
/// on, or `None` without prefixes, which select every entry.
fn selected(entries: &[Entry], only: &[String]) -> Option<BTreeSet<String>> {
    if only.is_empty() {
        return None;
    }
    let by_name: BTreeMap<&str, &Entry> = entries.iter().map(|e| (e.name.as_str(), e)).collect();
    let mut pending: Vec<&str> = by_name
        .keys()
        .copied()
        .filter(|name| only.iter().any(|prefix| name.starts_with(prefix.as_str())))
        .collect();
    let mut selected = BTreeSet::new();
    while let Some(name) = pending.pop() {
        if selected.insert(name.to_owned())
            && let Some(entry) = by_name.get(name)
        {
            pending.extend(entry.dependencies.iter().map(String::as_str));
        }
    }
    Some(selected)
}

/// The bundles of a run in the input store, as they arrive. Opening one waits until it has been
/// fetched, so evaluation starts with the first bundles while later ones are still on their way.
struct Arriving<'s> {
    inputs: &'s LocalStore,
    arrivals: Mutex<Arrivals>,
    changed: Condvar,
}

#[derive(Default)]
struct Arrivals {
    /// The key of each fetched bundle, or why it could not be fetched.
    fetched: HashMap<String, Result<Key, String>>,
    /// Whether fetching has ended. A bundle that has not arrived by then never does.
    ended: bool,
}

impl<'s> Arriving<'s> {
    fn new(inputs: &'s LocalStore) -> Self {
        Self {
            inputs,
            arrivals: Mutex::default(),
            changed: Condvar::new(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Arrivals> {
        self.arrivals.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn arrive(&self, name: &str, fetched: Result<Key, String>) {
        self.lock().fetched.insert(name.to_owned(), fetched);
        self.changed.notify_all();
    }

    /// Marks the end of fetching when dropped, however fetching ends.
    fn ending(&self) -> Ending<'_, 's> {
        Ending(self)
    }

    fn wait_until(&self, done: impl Fn(&Arrivals) -> bool) -> MutexGuard<'_, Arrivals> {
        self.changed
            .wait_while(self.lock(), |arrivals| !done(arrivals))
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Ends the fetching of an [`Arriving`] when dropped.
struct Ending<'a, 's>(&'a Arriving<'s>);

impl Drop for Ending<'_, '_> {
    fn drop(&mut self) {
        self.0.lock().ended = true;
        self.0.changed.notify_all();
    }
}

impl Inputs for Arriving<'_> {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        let arrivals =
            self.wait_until(|arrivals| arrivals.ended || arrivals.fetched.contains_key(name));
        let key = match arrivals.fetched.get(name) {
            Some(Ok(key)) => key.clone(),
            Some(Err(e)) => return Err(io::Error::other(format!("{name}: {e}"))),
            None => {
                let message = format!("{name} was not fetched");
                return Err(io::Error::new(io::ErrorKind::NotFound, message));
            }
        };
        drop(arrivals);
        open_input(self.inputs, name, &key)
    }
}

/// The bundle `name` that `inputs` hold under `key`.
fn open_input(inputs: &LocalStore, name: &str, key: &Key) -> io::Result<BundleInput> {
    let source = FileSource::open(inputs.path(key)).map_err(io::Error::other)?;
    Ok(BundleInput {
        name: name.to_owned(),
        source: Arc::new(source),
        sha256: key.sha256,
    })
}

/// The bundles that another output directory holds.
struct EarlierRun {
    catalog: Catalog,
    inputs: LocalStore,
}

impl EarlierRun {
    fn open(output: &Path) -> Result<Self, String> {
        let database = output.join("catalog.redb");
        if !database.is_file() {
            return Err(format!("{}: no catalog", output.display()));
        }
        let store = LocalStore::open(output.join("store")).map_err(|e| e.to_string())?;
        Ok(Self {
            catalog: Catalog::open_or_rebuild(&database, &store).map_err(|e| e.to_string())?,
            inputs: LocalStore::open(output.join("inputs")).map_err(|e| e.to_string())?,
        })
    }
}

/// Copies the bundle from the first earlier run that holds it under the same declared hash.
fn from_earlier_runs(
    earlier: &[EarlierRun],
    region: &str,
    entry: &BundleEntry,
    inputs: &LocalStore,
) -> io::Result<Option<Key>> {
    for run in earlier {
        let Some(key) = already_stored(&run.catalog, region, entry, &run.inputs)? else {
            continue;
        };
        inputs.adopt(&key, &run.inputs.path(&key))?;
        return Ok(Some(key));
    }
    Ok(None)
}

/// The key of the bundle that an earlier run stored under the same declared hash.
fn already_stored(
    catalog: &Catalog,
    region: &str,
    entry: &BundleEntry,
    inputs: &LocalStore,
) -> io::Result<Option<Key>> {
    let Some(row) = catalog.bundle(region, &entry.name)? else {
        return Ok(None);
    };
    if entry.declared.is_empty() || row.declared != entry.declared {
        return Ok(None);
    }
    let key = parse_hex(&row.sha256).map(|sha256| Key {
        sha256,
        extension: INPUT_EXTENSION.to_owned(),
    });
    Ok(key.filter(|key| inputs.contains(key)))
}

/// Where the bundles of a run come from, and where they are recorded.
struct Fetcher<'r> {
    source: &'r dyn Source,
    identity: &'r VersionIdentity,
    inputs: &'r LocalStore,
    catalog: &'r Catalog,
    /// The bundles of the last version of the region, in name order.
    bundles: &'r [BundleState],
    earlier: &'r [EarlierRun],
    region: &'r str,
    monitor: &'r Monitor,
    /// The key of each bundle this run has fetched, by name, so that none is fetched twice.
    fetched: Mutex<HashMap<String, Key>>,
}

impl Fetcher<'_> {
    /// Fetches `entries` into the input store in their order, `concurrency` at a time, and tells
    /// `arriving` of each as it arrives. The bundles this run places are recorded in the catalog
    /// in batches, and a bundle that cannot be recorded counts as not fetched. Returns how many
    /// bundles could not be fetched.
    fn fetch_all(
        &self,
        entries: &[BundleEntry],
        concurrency: usize,
        arriving: &Arriving<'_>,
    ) -> Result<usize, String> {
        let rows = Mutex::new(Vec::new());
        let results = parallel(entries.len(), concurrency, |i| -> Result<bool, String> {
            let entry = &entries[i];
            let fetched = self.fetch(entry);
            arriving.arrive(&entry.name, fetched.clone().map(|(key, _)| key));
            let (key, placed) = fetched?;
            if placed {
                self.place(&rows, entry, &key)?;
            }
            Ok(placed)
        });
        let mut new = 0;
        let mut failed = 0;
        for (entry, result) in entries.iter().zip(results) {
            match result {
                Ok(placed) => new += usize::from(placed),
                Err(e) => {
                    failed += 1;
                    eprintln!("{}: {e}", entry.name);
                }
            }
        }
        self.record(&rows.into_inner().unwrap_or_else(PoisonError::into_inner))?;
        println!(
            "bundles {} fetched {new} reused {} failed {failed}",
            entries.len(),
            entries.len() - new - failed
        );
        Ok(failed)
    }

    /// Fetches `entries` into the input store, `concurrency` at a time, and returns the key of
    /// each bundle that could be fetched, by name. A bundle that cannot be fetched is left out;
    /// fetching it again reports why.
    fn fetch_keys(
        &self,
        entries: &[BundleEntry],
        concurrency: usize,
    ) -> Result<HashMap<String, Key>, String> {
        let rows = Mutex::new(Vec::new());
        let keys = parallel(
            entries.len(),
            concurrency,
            |i| -> Result<Option<Key>, String> {
                let entry = &entries[i];
                let Ok((key, placed)) = self.fetch(entry) else {
                    return Ok(None);
                };
                if placed {
                    self.place(&rows, entry, &key)?;
                }
                Ok(Some(key))
            },
        );
        self.record(&rows.into_inner().unwrap_or_else(PoisonError::into_inner))?;
        let mut fetched = HashMap::new();
        for (entry, key) in entries.iter().zip(keys) {
            if let Some(key) = key? {
                fetched.insert(entry.name.clone(), key);
            }
        }
        Ok(fetched)
    }

    /// The key of `entry` in the input store, and whether this call placed it there: fetched
    /// earlier in this run, found in the catalog, linked from an earlier run, or downloaded.
    fn fetch(&self, entry: &BundleEntry) -> Result<(Key, bool), String> {
        let fetched = || self.fetched.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(key) = fetched().get(&entry.name) {
            return Ok((key.clone(), false));
        }
        let (key, placed) = self.locate(entry)?;
        fetched().insert(entry.name.clone(), key.clone());
        Ok((key, placed))
    }

    /// The key of `entry` in the input store, and whether this call placed it there: found in the
    /// catalog, linked from an earlier run, or downloaded.
    fn locate(&self, entry: &BundleEntry) -> Result<(Key, bool), String> {
        let error = |e: io::Error| e.to_string();
        if let Some(key) =
            already_stored(self.catalog, self.region, entry, self.inputs).map_err(error)?
        {
            return Ok((key, false));
        }
        if let Some(key) =
            from_earlier_runs(self.earlier, self.region, entry, self.inputs).map_err(error)?
        {
            return Ok((key, true));
        }
        self.download(entry).map(|key| (key, true))
    }

    fn download(&self, entry: &BundleEntry) -> Result<Key, String> {
        let mut span = self.monitor.span("fetch", "bundle", &entry.name);
        let writer = self
            .inputs
            .writer(INPUT_EXTENSION)
            .map_err(|e| format!("input store: {e}"))?;
        let mut sink = Deobfuscating::new(Incoming(writer));
        let read = self
            .source
            .fetch(self.identity, entry, &mut sink)
            .map_err(|e| e.to_string())?;
        let Incoming(writer) = sink.finish().map_err(|e| e.to_string())?;
        let (key, size) = writer.finish().map_err(|e| format!("input store: {e}"))?;
        span.bytes(read, size);
        Ok(key)
    }

    /// Adds the row of a bundle this run placed to `rows`, and records them once there are
    /// [`RECORD_BATCH`] of them.
    fn place(
        &self,
        rows: &Mutex<Vec<(String, BundleRow)>>,
        entry: &BundleEntry,
        key: &Key,
    ) -> Result<(), String> {
        let row = BundleRow {
            declared: entry.declared.clone(),
            sha256: key.to_string()[..64].to_owned(),
        };
        let full = {
            let mut rows = rows.lock().unwrap_or_else(PoisonError::into_inner);
            rows.push((entry.name.clone(), row));
            (rows.len() >= RECORD_BATCH).then(|| std::mem::take(&mut *rows))
        };
        full.map_or(Ok(()), |full| self.record(&full))
    }

    fn record(&self, rows: &[(String, BundleRow)]) -> Result<(), String> {
        let write = || -> io::Result<()> {
            let mut batch = self.catalog.write()?;
            for (name, row) in rows {
                batch.put_bundle(self.region, name, row)?;
            }
            batch.commit()
        };
        write().map_err(|e| format!("catalog: {e}"))
    }
}

/// The entries of `all` that `manifest` names, in the order an evaluation of `manifest` opens
/// them, and then the rest of them.
fn in_opening_order(all: &[BundleEntry], manifest: &[Entry]) -> Vec<BundleEntry> {
    let rank: HashMap<&str, usize> = stream::opening_order(manifest)
        .into_iter()
        .enumerate()
        .map(|(rank, name)| (name, rank))
        .collect();
    let wanted: BTreeSet<&str> = manifest.iter().map(|entry| entry.name.as_str()).collect();
    let mut entries: Vec<BundleEntry> = all
        .iter()
        .filter(|entry| wanted.contains(entry.name.as_str()))
        .cloned()
        .collect();
    entries.sort_by_key(|entry| rank.get(entry.name.as_str()).copied().unwrap_or(usize::MAX));
    entries
}

/// What identifies the content of a bundle whose manifest entry declares the hash `declared`:
/// that hash, or nothing when it is empty.
fn declared_content(declared: &str) -> String {
    if declared.is_empty() {
        String::new()
    } else {
        format!("declared {declared}")
    }
}

/// Identifies the content of each entry of `manifest` that declares no hash by the bytes of its
/// bundle, which are fetched for that from the entries of `all`. An entry whose bundle cannot be
/// fetched stays unidentified.
fn identify(
    fetcher: &Fetcher<'_>,
    all: &[BundleEntry],
    manifest: &mut [Entry],
    concurrency: usize,
) -> Result<(), String> {
    let undeclared: HashSet<&str> = manifest
        .iter()
        .filter(|entry| entry.content.is_empty())
        .map(|entry| entry.name.as_str())
        .collect();
    if undeclared.is_empty() {
        return Ok(());
    }
    let entries: Vec<BundleEntry> = all
        .iter()
        .filter(|entry| undeclared.contains(entry.name.as_str()))
        .cloned()
        .collect();
    let keys = fetcher.fetch_keys(&entries, concurrency)?;
    println!(
        "bundles without a declared hash {} identified {}",
        entries.len(),
        keys.len()
    );
    for entry in manifest {
        if let Some(key) = keys.get(&entry.name) {
            entry.content = format!("bytes {key}");
        }
    }
    Ok(())
}

/// What a run evaluates.
enum Evaluating<'m> {
    /// The units of a manifest that lists the dependencies of its bundles, taking what earlier
    /// runs gave.
    Listed(Reuse<'m>),
    /// A manifest without dependencies. They are read from the bundles themselves, and then the
    /// entries whose names start with one of `only` are selected, with what they depend on, and
    /// every unit of them is evaluated.
    Read {
        manifest: Vec<Entry>,
        only: &'m [String],
    },
}

impl Evaluating<'_> {
    /// The entries whose bundles the evaluation needs.
    fn entries(&self) -> &[Entry] {
        match self {
            Evaluating::Listed(reuse) => reuse.rest(),
            Evaluating::Read { manifest, .. } => manifest,
        }
    }
}

/// What a run evaluated.
struct Evaluated {
    outcome: reuse::Outcome,
    /// How many bundles could not be fetched.
    failed: usize,
}

/// Fetches `entries` while `evaluating` runs, so the first segments convert while later bundles
/// are still on their way. Fails if the store, the records or the catalog do.
fn fetch_and_evaluate(
    fetcher: &Fetcher<'_>,
    entries: &[BundleEntry],
    concurrency: usize,
    evaluating: Evaluating<'_>,
    stream: &Stream<'_>,
) -> Result<Evaluated, String> {
    let arriving = Arriving::new(fetcher.inputs);
    thread::scope(|scope| {
        let fetching = scope.spawn(|| {
            let _ending = arriving.ending();
            fetcher.fetch_all(entries, concurrency, &arriving)
        });
        let evaluate = || match evaluating {
            Evaluating::Listed(reuse) => reuse.evaluate(&arriving, stream),
            Evaluating::Read { mut manifest, only } => {
                let _span = stream.recorder.span("manifest", "dependencies", "");
                let threads = stream.options.threads;
                stream::with_dependencies(
                    &mut manifest,
                    &arriving,
                    stream.cache,
                    stream.indexes,
                    threads,
                )?;
                let selected = select(manifest, only);
                Reuse::refresh(&selected, stream)?.evaluate(&arriving, stream)
            }
        };
        let evaluated = evaluate().map_err(|e| e.to_string());
        let failed = fetching
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
        Ok(Evaluated {
            outcome: evaluated?,
            failed,
        })
    })
}

/// The text of the configuration, and the region `options.region` it describes.
fn configuration(options: &Options) -> Result<(String, Region), String> {
    let config = std::fs::read_to_string(&options.config)
        .map_err(|e| format!("{}: {e}", options.config.display()))?;
    let region = Region::load(&options.region, &config).map_err(|e| e.to_string())?;
    Ok((config, region))
}

/// The tables of `region`, when its configuration names a source for them.
fn open_tables(region: &Region) -> Result<Option<Tables>, String> {
    region
        .tables
        .as_ref()
        .map(|source| Tables::open(source, &region.network))
        .transpose()
        .map_err(|e| e.to_string())
}

/// The current version of the region `name`, with the revision of its tables. What `source`
/// looks up may depend on the `previous` version.
fn current_identity(
    name: &str,
    source: &dyn Source,
    tables: Option<&Tables>,
    previous: Option<&VersionIdentity>,
    recorder: &dyn Recorder,
) -> Result<VersionIdentity, String> {
    let mut identity = {
        let _span = recorder.span("plan", "identity", name);
        source.identity(previous).map_err(|e| e.to_string())?
    };
    identity.tables = tables.map(|tables| tables.revision().to_owned());
    Ok(identity)
}

/// Prints whether the content of the current version differs from the last one recorded: in the
/// bucket of the configuration when it names one, and otherwise in `options.output`. Returns
/// whether it is unchanged. Nothing is downloaded or written.
fn check(options: &Options) -> Result<bool, lexopt::Error> {
    let name = options.region.as_str();
    let (config, region) = configuration(options)?;
    let bucket = Bucket::open(&config, options.secrets.as_deref())?;
    let site = bucket
        .as_ref()
        .map_or_else(|| Site::local(&options.output), Site::Bucket);
    let previous = site::last_version(site.files(), name)?;
    let tables = open_tables(&region)?;
    let secrets = Secrets::new(options.secrets.clone());
    let source = cdn::open(region, &secrets).map_err(|e| e.to_string())?;
    let identity = current_identity(
        name,
        source.as_ref(),
        tables.as_ref(),
        previous.as_ref(),
        &NoRecorder,
    )?;
    let change = match previous {
        Some(previous) if previous.same_content(&identity) => None,
        Some(previous) => Some(format!("changed from {}", previous.name())),
        None => Some("changed".to_owned()),
    };
    let described = change.as_deref().unwrap_or("unchanged");
    println!("{name} {} {described}", identity.name());
    Ok(change.is_none())
}

/// What a run made of the current version.
#[derive(Debug)]
enum Outcome {
    /// Its content is that of the last version recorded, so nothing was done.
    Unchanged(VersionIdentity),
    /// Some bundles could not be read, so it was neither published nor recorded as a version.
    Incomplete,
    /// It was exported, published and recorded.
    Updated(VersionIdentity),
}

impl Outcome {
    /// Whether the run did all it set out to do.
    fn succeeded(&self) -> bool {
        self.version().is_some()
    }

    /// The version now recorded as the last one, unless the run is incomplete.
    fn version(&self) -> Option<&VersionIdentity> {
        match self {
            Self::Unchanged(identity) | Self::Updated(identity) => Some(identity),
            Self::Incomplete => None,
        }
    }
}

/// Updates `options.region` at its site: the bucket of the configuration when it names one, and
/// otherwise the output directory. A selection runs on the output directory alone.
fn update(options: &Options, monitor: &Monitor) -> Result<bool, lexopt::Error> {
    let (config, region) = configuration(options)?;
    let bucket = if options.only.is_empty() {
        Bucket::open(&config, options.secrets.as_deref())?
    } else {
        None
    };
    let site = bucket
        .as_ref()
        .map_or_else(|| Site::local(&options.output), Site::Bucket);
    export_version(options, region, &site, monitor).map(|run| run.succeeded())
}

/// Exports the current version of `region` as `options` say, and publishes it at `site`.
///
/// The run starts from the [`Baseline`] the site keeps, evaluates only the units that changed,
/// patches the publications of both views and commits them as [`Context::commit`] says, so that
/// an interrupted run leaves the site as the last one did, or with objects and records that the
/// next run uses.
fn export_version(
    options: &Options,
    region: Region,
    site: &Site<'_>,
    monitor: &Monitor,
) -> Result<Outcome, lexopt::Error> {
    let region_name = options.region.as_str();
    let opened = Opened::open(options, region)?;
    // A selection is not a version: it keeps no state and records nothing.
    let unkept = MemoryFiles::default();
    let selection = !options.only.is_empty();
    let files: &dyn StateFiles = if selection { &unkept } else { site.files() };
    let previous = site::last_version(site.files(), region_name)?;
    if selection && previous.is_some() {
        return Err("--only needs an <output> that holds no complete run".into());
    }
    let identity = current_identity(
        region_name,
        opened.source.as_ref(),
        opened.tables.as_ref(),
        previous.as_ref(),
        monitor,
    )?;
    if !(options.force || options.verify) && previous.is_some_and(|p| p.same_content(&identity)) {
        println!("{region_name} {} unchanged", identity.name());
        return Ok(Outcome::Unchanged(identity));
    }

    let held = holdings(options, site, monitor)?;
    let objects = Objects {
        local: &opened.store,
        held: &held,
        remote: site.remote(),
    };
    let records = Records::new(files, &held);
    let backed = Backed {
        records: &records,
        catalog: Some(&opened.catalog),
    };
    let mut baseline = Baseline::read(files, site, region_name, &objects, options.threads)?;
    let evaluation = opened.evaluate(
        options,
        &identity,
        &objects,
        &backed,
        &baseline.state,
        monitor,
    )?;
    let Some(Evaluation { outcome, bundles }) = evaluation else {
        return Ok(Outcome::Incomplete);
    };
    let rulings = baseline.state.advance(&outcome.kept, &outcome.results);
    if let Some(bundles) = bundles {
        baseline.state.bundles = bundles;
    }

    let context = Context {
        options,
        site,
        files,
        store: &opened.store,
        objects: &objects,
        backed: &backed,
        monitor,
    };
    let advance = Advance {
        rulings: &rulings,
        state: &baseline.state,
        results: &outcome.results,
        records: &backed,
    };
    let views = context.publish(&identity.name(), &baseline, &advance, &opened)?;
    let committed = context.commit(&records, &outcome.results, &mut baseline, &views, &identity)?;
    record_costs(opened.catalog.catalog(), monitor)?;
    print_summary(region_name, &baseline.state, &views, &outcome, &committed);
    Ok(Outcome::Updated(identity))
}

/// What a run reads from: the source of the region and its tables, the directories of the output
/// and the converters.
struct Opened {
    source: Box<dyn Source>,
    tables: Option<Tables>,
    /// The local store, which keeps the objects of the run and caches those of the site.
    store: LocalStore,
    /// The downloaded bundles.
    inputs: LocalStore,
    /// The catalog of the output directory, which caches downloads and backs the records.
    catalog: CatalogDerivations,
    earlier: Vec<EarlierRun>,
    converters: Vec<Box<dyn Converter>>,
    /// Whether the manifest lists the dependencies of the bundles. A directory lists none; they
    /// are read from the bundles once they are fetched.
    listed: bool,
    /// How many bundles are fetched at once.
    concurrency: usize,
}

impl Opened {
    fn open(options: &Options, region: Region) -> Result<Self, lexopt::Error> {
        let listed = region.kind != Kind::Dir;
        let concurrency = region.network.concurrency.unwrap_or(options.threads).max(1);
        let earlier = region
            .stores
            .iter()
            .map(|output| EarlierRun::open(output))
            .collect::<Result<Vec<_>, _>>()?;
        let secrets = Secrets::new(options.secrets.clone());
        let hca_key = secrets
            .hca_key(&region.secrets)
            .map_err(|e| e.to_string())?;
        let tables = open_tables(&region)?;
        let source = cdn::open(region, &secrets).map_err(|e| e.to_string())?;
        let directory = |name: &str| {
            LocalStore::open(options.output.join(name))
                .map_err(|e| format!("{}/{name}: {e}", options.output.display()))
        };
        let store = directory("store")?;
        let catalog = Catalog::open_or_rebuild(&options.output.join("catalog.redb"), &store)
            .map_err(|e| e.to_string())?
            .into_derivations();
        Ok(Self {
            source,
            tables,
            inputs: directory("inputs")?,
            store,
            catalog,
            earlier,
            converters: converters(hca_key),
            listed,
            concurrency,
        })
    }

    /// Evaluates the units of the version `identity` that changed since the run that left
    /// `state`, with the objects of `objects` and the records of `backed`. Returns `None` when
    /// some bundles could not be fetched or read.
    fn evaluate(
        &self,
        options: &Options,
        identity: &VersionIdentity,
        objects: &Objects<'_>,
        backed: &Backed<'_>,
        state: &RegionState,
        monitor: &Monitor,
    ) -> Result<Option<Evaluation>, lexopt::Error> {
        let region = options.region.as_str();
        let all = {
            let _span = monitor.span("manifest", "manifest", region);
            self.source.manifest(identity).map_err(|e| e.to_string())?
        };
        let mut manifest = manifest_entries(&all);
        let fetcher = Fetcher {
            source: self.source.as_ref(),
            identity,
            inputs: &self.inputs,
            catalog: self.catalog.catalog(),
            bundles: &state.bundles,
            earlier: &self.earlier,
            region,
            monitor,
            fetched: Mutex::default(),
        };
        let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
        let indexes =
            IndexCache::open(&options.output.join("indexes.redb")).map_err(|e| e.to_string())?;
        let mut bundles = None;
        if self.listed {
            let selected = selected(&manifest, &options.only);
            let fetchable = |name: &str| selected.as_ref().is_none_or(|names| names.contains(name));
            let concurrency = self.concurrency;
            bundles = Some(fetcher.add_file_dependencies(
                &all,
                &mut manifest,
                fetchable,
                &cache,
                &indexes,
                concurrency,
            )?);
            manifest = select(manifest, &options.only);
            identify(&fetcher, &all, &mut manifest, concurrency)?;
        }
        let fusion = crate::storage::Fusion::new(&self.store, backed, monitor);
        let stream = Stream {
            converters: &self.converters,
            naming: &InBundle,
            store: objects,
            derivations: backed,
            indexes: &indexes,
            cache: &cache,
            classes: &NoClasses,
            recorder: monitor,
            fusion: Some(&fusion),
            options: budget::options(options.threads, options.memory, options.plain),
            segment: Segment::default(),
            ledger: Some(IGNORED),
            limit: None,
        };
        let earlier = Earlier::new(&state.units, backed);
        let evaluating = if self.listed {
            let reuse = if options.force {
                Reuse::refresh(&manifest, &stream)
            } else {
                Reuse::new(&manifest, &stream, &earlier)
            };
            let reuse = reuse.map_err(|e| e.to_string())?;
            println!("units {} reused {}", manifest.len(), reuse.reused());
            Evaluating::Listed(reuse)
        } else {
            Evaluating::Read {
                manifest,
                only: &options.only,
            }
        };
        let entries = in_opening_order(&all, evaluating.entries());
        let evaluated =
            fetch_and_evaluate(&fetcher, &entries, self.concurrency, evaluating, &stream);
        let Evaluated { outcome, failed } = evaluated?;
        if failed > 0 || report(&outcome.streamed) > 0 {
            return Ok(None);
        }
        write_ledger(&outcome.streamed.ledgers, &options.output, monitor)?;
        Ok(Some(Evaluation { outcome, bundles }))
    }
}

/// The entries of the manifest `all`, each with the content its source declares.
fn manifest_entries(all: &[BundleEntry]) -> Vec<Entry> {
    all.iter()
        .map(|entry| Entry {
            name: entry.name.clone(),
            size: entry.size,
            content: declared_content(&entry.declared),
            dependencies: entry.dependencies.clone(),
        })
        .collect()
}

/// What a complete evaluation of a version gave.
struct Evaluation {
    outcome: reuse::Outcome,
    /// The bundles of the version, when its manifest lists their dependencies.
    bundles: Option<Vec<BundleState>>,
}

/// The objects that `site` is known to hold before the run: with `--verify`, those a listing of
/// the bucket finds, and else none until the run reads records or puts objects there.
fn holdings(
    options: &Options,
    site: &Site<'_>,
    monitor: &Monitor,
) -> Result<MemoryHoldings, lexopt::Error> {
    match site {
        Site::Bucket(bucket) if options.verify => {
            let listed = bucket.holdings(monitor)?;
            println!("bucket holds {} objects", listed.len());
            Ok(MemoryHoldings::new(listed))
        }
        _ => Ok(MemoryHoldings::default()),
    }
}

/// What a run starts from: the state that the last run of the region left, and the previous
/// publications of its views. When the state does not name the revisions those publications have,
/// the run starts from no publication and keeps only what the state knows of bundles.
struct Baseline {
    state: RegionState,
    /// The root of the state, whose chunks a write keeps.
    root: Option<Root>,
    canonical: Previous,
    storage: Previous,
}

impl Baseline {
    /// The baseline of `region`, from the state in `files` and the publications of `site`, whose
    /// documents are read from `store`.
    fn read(
        files: &dyn StateFiles,
        site: &Site<'_>,
        region: &str,
        store: &dyn Store,
        threads: usize,
    ) -> Result<Self, String> {
        let read = RegionState::read(files, region, threads);
        let read = read.map_err(|e| format!("the state of {region}: {e}"))?;
        let (state, root) = read.map_or_else(Default::default, |(state, root)| (state, Some(root)));
        let canonical = site.current(&canonical_target(region))?;
        let storage = site.current(&storage_target(region))?;
        let agrees = |target: String, pointer: &Option<Pointer>| {
            pointer.as_ref().map(|pointer| &pointer.revision) == state.published.get(&target)
        };
        if !(agrees(canonical_target(region), &canonical)
            && agrees(storage_target(region), &storage))
        {
            println!(
                "{region}: the state names other publications, so the views are published anew"
            );
            let state = RegionState {
                bundles: state.bundles,
                ..RegionState::default()
            };
            return Ok(Self {
                state,
                root,
                canonical: Previous::default(),
                storage: Previous::default(),
            });
        }
        let previous = |pointer: Option<Pointer>| {
            pointer.map_or(Ok(Previous::default()), |pointer| {
                Previous::read(store, &pointer).map_err(|e| e.to_string())
            })
        };
        Ok(Self {
            canonical: previous(canonical)?,
            storage: previous(storage)?,
            state,
            root,
        })
    }
}

fn canonical_target(region: &str) -> String {
    format!("{region}/canonical")
}

fn storage_target(region: &str) -> String {
    format!("{region}/storage")
}

/// The publications of the views of a version, and the assets it built from tables.
struct Views {
    canonical: Publication,
    /// The storage view, unless this build has none.
    storage: Option<Publication>,
    tables: Vec<Output>,
}

impl Views {
    /// The target of each view of `region`, with its publication when it has one.
    fn published(&self, region: &str) -> [(String, Option<&Publication>); 2] {
        [
            (canonical_target(region), Some(&self.canonical)),
            (storage_target(region), self.storage.as_ref()),
        ]
    }

    /// The objects that the publications refer to.
    fn objects(&self) -> impl Iterator<Item = Object> + '_ {
        let publications = [Some(&self.canonical), self.storage.as_ref()];
        let publications = publications.into_iter().flatten();
        publications.flat_map(|publication| publication.objects.iter().cloned())
    }
}

/// What the steps of a run after its evaluation share.
struct Context<'a> {
    options: &'a Options,
    site: &'a Site<'a>,
    /// Where the state and the records are kept.
    files: &'a dyn StateFiles,
    /// The local store, where new objects are put.
    store: &'a LocalStore,
    objects: &'a Objects<'a>,
    backed: &'a Backed<'a>,
    monitor: &'a Monitor,
}

/// What a commit wrote.
struct Committed {
    /// How many records.
    records: usize,
    state: Written,
}

impl Context<'_> {
    /// The publications of the version `name` that `advance` reaches from `baseline`, patched
    /// from the previous ones, with the converters of `opened`.
    fn publish(
        &self,
        name: &str,
        baseline: &Baseline,
        advance: &Advance<'_>,
        opened: &Opened,
    ) -> Result<Views, String> {
        let owners = advance.owners()?;
        let canonical = baseline.canonical.lookup(self.objects);
        let tables = self.build_tables(opened, &advance.state.claims, &owners, &canonical)?;
        let (mut old, mut new) = advance.canonical_bindings(&canonical, &owners)?;
        old.extend(table_bindings(&advance.state.tables)?);
        new.extend(tables.iter().map(|output| Binding {
            name: output.name.clone(),
            document: output.document.clone(),
        }));
        let threads = self.options.threads;
        let change = views::canonical_change(old, new, self.objects, threads)?;
        let layout = self.site.layout();
        let canonical_publication =
            self.apply("canonical", name, layout, &baseline.canonical, change)?;

        let (units, assets) = advance.storage_assets(&canonical, &owners)?;
        let previous = baseline.storage.lookup(self.objects);
        let storage = crate::storage::change(
            &units,
            &assets,
            &previous,
            self.objects,
            self.backed,
            threads,
            self.monitor,
        )?;
        let storage = storage
            .map(|change| self.apply("storage", name, Layout::Paths, &baseline.storage, change));
        Ok(Views {
            canonical: canonical_publication,
            storage: storage.transpose()?,
            tables,
        })
    }

    /// The assets that the converters of tables build, with the assets that `claims` own as
    /// those of the version: their documents are those of `owners`, or else those of the
    /// previous publication `canonical`.
    fn build_tables(
        &self,
        opened: &Opened,
        claims: &[Claims],
        owners: &BTreeMap<String, Key>,
        canonical: &Lookup<'_>,
    ) -> Result<Vec<Output>, String> {
        let Some(tables) = &opened.tables else {
            return Ok(Vec::new());
        };
        let document = |name: &str| match owners.get(name) {
            Some(document) => Ok(Some(document.clone())),
            None => canonical.document(name).map_err(io::Error::other),
        };
        let read = |name: &str| tables.read(name).map_err(io::Error::other);
        let stage = TableStage {
            converters: &table_converters(),
            read: &read,
            store: self.objects,
            recorder: self.monitor,
            threads: self.options.threads,
        };
        let forms: Vec<&'static str> = opened.converters.iter().map(|c| c.name()).collect();
        build_tables(&stage, claims, &forms, &document)
    }

    /// Applies `change` to the publication `previous` of `view`, as the version `name`.
    fn apply(
        &self,
        view: &str,
        name: &str,
        layout: Layout,
        previous: &Previous,
        change: Change,
    ) -> Result<Publication, String> {
        let _span = self.monitor.span("publish", view, "");
        let applied = patch::apply(
            self.objects,
            self.store,
            view,
            name,
            layout,
            previous,
            change,
        );
        applied.map_err(|e| e.to_string())
    }

    /// Commits the version `identity` that `views` publish, in this order: puts at the site every
    /// object of the views and of the records of `results`, then writes the records, the state of
    /// `baseline` advanced to the version, the pointers of the views and the last version.
    fn commit(
        &self,
        records: &Records<'_>,
        results: &[UnitResult],
        baseline: &mut Baseline,
        views: &Views,
        identity: &VersionIdentity,
    ) -> Result<Committed, String> {
        let threads = self.options.threads;
        let region = self.options.region.as_str();
        let results: Vec<&UnitResult> = results.iter().collect();
        let mut referred: Vec<Object> = record_objects(records, &results, self.objects, threads)?;
        // The publications come last, so the media types they give are the ones stored.
        referred.extend(views.objects());
        self.site.store(self.objects, referred)?;
        let written = records.write(&results, threads);
        let written = written.map_err(|e| format!("records: {e}"))?;

        let published = views.published(region);
        let state = &mut baseline.state;
        state.version = identity.name();
        state.tables = views.tables.iter().map(TableOutput::of).collect();
        state.published = published
            .iter()
            .filter_map(|(target, publication)| {
                publication
                    .map(|publication| (target.clone(), publication.pointer.revision.clone()))
            })
            .collect();
        let state_written = state.write(self.files, region, baseline.root.as_ref(), threads);
        let state_written = state_written.map_err(|e| format!("the state of {region}: {e}"))?;
        for (target, publication) in &published {
            if let Some(publication) = publication {
                self.site.make_current(target, &publication.pointer)?;
            }
        }
        site::set_last_version(self.files, region, identity)?;
        Ok(Committed {
            records: written,
            state: state_written,
        })
    }
}

/// Prints what the run of `region` published and wrote, with the `state` it left.
fn print_summary(
    region: &str,
    state: &RegionState,
    views: &Views,
    outcome: &reuse::Outcome,
    committed: &Committed,
) {
    let owned = state.claims.iter().filter_map(Claims::owner);
    let errors = owned.clone().filter(|owner| owner.failed).count();
    let unnamed = state.claims.iter().filter(|c| c.owner().is_none()).count();
    let evaluation = &outcome.streamed.evaluation;
    println!(
        "{region} {} published revision {} assets {} errors {errors} unnamed {unnamed} \
         evaluated {} kept {} converted {} reused {} records {} state chunks {} bytes {}",
        state.version,
        views.canonical.pointer.revision,
        owned.count() + views.tables.len(),
        outcome.results.len(),
        outcome.kept.len(),
        evaluation.converted,
        evaluation.reused,
        committed.records,
        committed.state.chunks,
        committed.state.bytes,
    );
}

/// The assets built from tables, with the produced assets that `claims` own as the assets of the
/// version, whose forms are among `forms` and whose documents `document` finds.
fn build_tables(
    stage: &TableStage<'_>,
    claims: &[Claims],
    forms: &[&'static str],
    document: &(dyn Fn(&str) -> io::Result<Option<Key>> + Sync),
) -> Result<Vec<Output>, String> {
    let mut by_bundle: BTreeMap<String, Vec<AssetEntry>> = BTreeMap::new();
    for claims in claims {
        let Some(owner) = claims.owner().filter(|owner| !owner.failed) else {
            continue;
        };
        let form = forms
            .iter()
            .copied()
            .find(|&form| form == owner.form)
            .ok_or_else(|| format!("{} has the unknown form {}", claims.name, owner.form))?;
        by_bundle
            .entry(owner.unit.clone())
            .or_default()
            .push(AssetEntry {
                name: claims.name.clone(),
                form,
            });
    }
    let version = VersionAssets {
        by_bundle,
        document,
    };
    let built = sekai_pipeline::tables::build(stage, &version).map_err(|e| e.to_string())?;
    let owned = |name: &str| {
        let found = claims.binary_search_by(|claims| claims.name.as_str().cmp(name));
        found.is_ok_and(|index| claims[index].owner().is_some())
    };
    match built.iter().find(|output| owned(&output.name)) {
        Some(output) => Err(format!("two assets are named {}", output.name)),
        None => Ok(built),
    }
}

/// The bindings of the assets built from tables that the state lists.
fn table_bindings(tables: &[TableOutput]) -> Result<Vec<Binding>, String> {
    tables
        .iter()
        .map(|table| {
            let document = table.document.parse().map_err(|_| {
                format!(
                    "the table asset {} names the document {}",
                    table.name, table.document
                )
            })?;
            Ok(Binding {
                name: table.name.clone(),
                document,
            })
        })
        .collect()
}

/// The objects that the records of `units` name, read from `objects` on up to `threads` threads:
/// the asset documents and their members.
fn record_objects(
    records: &Records<'_>,
    units: &[&UnitResult],
    objects: &Objects<'_>,
    threads: usize,
) -> Result<Vec<Object>, String> {
    let documents = records
        .documents(units)
        .map_err(|e| format!("records: {e}"))?;
    let named = parallel(documents.len(), threads, |i| {
        document_objects(objects, &documents[i])
    });
    let mut all = Vec::new();
    for (document, named) in documents.iter().zip(named) {
        all.extend(named.map_err(|e| format!("{document}: {e}"))?);
    }
    Ok(all)
}

/// The asset document `key` and its members, as objects of the store.
fn document_objects(objects: &Objects<'_>, key: &Key) -> io::Result<Vec<Object>> {
    let bytes = objects.read(key)?;
    let document = AssetDocument::from_bytes(&bytes).map_err(io::Error::other)?;
    let mut named = vec![Object {
        key: key.clone(),
        size: bytes.len() as u64,
        media_type: "application/json".to_owned(),
    }];
    for member in document.members {
        let sha256 = parse_hex(&member.sha256)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a member has a bad hash"))?;
        named.push(Object {
            key: Key {
                sha256,
                extension: member.extension,
            },
            size: member.size,
            media_type: member.media_type,
        });
    }
    Ok(named)
}

/// Records what each stage of the run cost in the catalog of the output directory.
fn record_costs(catalog: &Catalog, monitor: &Monitor) -> Result<(), String> {
    let record = || -> io::Result<()> {
        let mut batch = catalog.write()?;
        for (subject, cost) in monitor::costs(monitor) {
            batch.put_cost(&CostRow {
                kind: cost.kind,
                subject,
                wall_us: cost.wall_us,
                cpu_us: cost.cpu_us,
                read: cost.read,
                written: cost.written,
                estimated_memory: cost.estimated_memory,
                decoded_memory: cost.decoded_memory,
            })?;
        }
        batch.commit()
    };
    record().map_err(|e| format!("catalog: {e}"))
}

/// The converters that build assets from game data tables.
fn table_converters() -> Vec<Box<dyn TableConverter>> {
    let mut converters = episode_converters();
    converters.extend(costume_converters());
    converters.extend(mysekai_converters());
    converters
}

#[cfg(feature = "costume")]
fn costume_converters() -> Vec<Box<dyn TableConverter>> {
    vec![
        Box::new(sekai_moesekai::costume::CostumeRoles),
        Box::new(sekai_moesekai::costume::Costumes),
    ]
}

#[cfg(not(feature = "costume"))]
fn costume_converters() -> Vec<Box<dyn TableConverter>> {
    Vec::new()
}

#[cfg(feature = "episode")]
fn episode_converters() -> Vec<Box<dyn TableConverter>> {
    vec![Box::new(sekai_moesekai::episode::Episode)]
}

#[cfg(not(feature = "episode"))]
fn episode_converters() -> Vec<Box<dyn TableConverter>> {
    Vec::new()
}

#[cfg(feature = "mysekai")]
fn mysekai_converters() -> Vec<Box<dyn TableConverter>> {
    vec![
        Box::new(sekai_mysekai::alone::AloneActions),
        Box::new(sekai_mysekai::characters::Characters),
        Box::new(sekai_mysekai::client_config::ClientConfig),
        Box::new(sekai_mysekai::communication::CommunicationActions),
        Box::new(sekai_mysekai::emoticon::Emoticons),
        Box::new(sekai_mysekai::facial::FacialTables),
        Box::new(sekai_mysekai::fixture_areas::FixtureAreas),
        Box::new(sekai_mysekai::fixture_attach_points::FixtureAttachPoints),
        Box::new(sekai_mysekai::fixtures::Fixtures),
        Box::new(sekai_mysekai::talks::FixtureTalks),
        Box::new(sekai_mysekai::talks::Talks),
    ]
}

#[cfg(not(feature = "mysekai"))]
fn mysekai_converters() -> Vec<Box<dyn TableConverter>> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use sekai_pipeline::NoBundleIndexes;

    use super::*;

    fn entry(name: &str, dependencies: &[&str]) -> BundleEntry {
        BundleEntry {
            name: name.to_owned(),
            declared: "h".to_owned(),
            download_path: None,
            size: 1,
            dependencies: dependencies.iter().map(|d| (*d).to_owned()).collect(),
        }
    }

    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }

    /// The manifest of `all`, as a run lists it before identifying any bundle.
    fn listed(all: &[BundleEntry]) -> Vec<Entry> {
        all.iter()
            .map(|entry| Entry {
                name: entry.name.clone(),
                size: entry.size,
                content: declared_content(&entry.declared),
                dependencies: entry.dependencies.clone(),
            })
            .collect()
    }

    fn identity() -> VersionIdentity {
        VersionIdentity {
            asset: sekai_format::identity::AssetIdentity::Nuverse {
                application_version: "1.0.0".to_owned(),
                value: "1".to_owned(),
            },
            client: sekai_format::identity::ClientIdentity {
                application_version: "1.0.0".to_owned(),
                application_hash: None,
                multiplayer_version: None,
            },
            tables: None,
        }
    }

    /// Bundles in memory, counting how often each is fetched.
    #[derive(Default)]
    struct Bundles {
        bytes: HashMap<String, Vec<u8>>,
        fetched: Mutex<HashMap<String, usize>>,
    }

    impl Source for Bundles {
        fn identity(
            &self,
            _previous: Option<&VersionIdentity>,
        ) -> Result<VersionIdentity, cdn::SourceError> {
            unreachable!("the tests name the version")
        }

        fn manifest(&self, _: &VersionIdentity) -> Result<Vec<BundleEntry>, cdn::SourceError> {
            unreachable!("the tests list the manifest")
        }

        fn fetch(
            &self,
            _: &VersionIdentity,
            entry: &BundleEntry,
            sink: &mut dyn Sink,
        ) -> Result<u64, cdn::SourceError> {
            *self
                .fetched
                .lock()
                .unwrap()
                .entry(entry.name.clone())
                .or_default() += 1;
            let bytes = self.bytes.get(&entry.name).ok_or(cdn::SourceError {
                code: "status",
                message: format!("{} is missing", entry.name),
                status: Some(404),
            })?;
            sink.write_all(bytes).unwrap();
            Ok(bytes.len() as u64)
        }
    }

    #[test]
    fn bundles_without_a_declared_hash_are_identified_by_their_bytes() {
        let root = std::env::temp_dir().join(format!("sekai-identify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = LocalStore::open(root.join("store")).unwrap();
        let catalog = Catalog::open_or_rebuild(&root.join("catalog.redb"), &store).unwrap();
        let inputs = LocalStore::open(root.join("inputs")).unwrap();
        let source = Bundles {
            bytes: HashMap::from([("b/plain".to_owned(), b"UnityFS\0plain".to_vec())]),
            ..Bundles::default()
        };
        let identity = identity();
        let mut undeclared = entry("b/plain", &[]);
        undeclared.declared = String::new();
        let mut lost = entry("c/lost", &[]);
        lost.declared = String::new();
        let all = [entry("a/declared", &[]), undeclared, lost];
        let mut manifest = listed(&all);

        Monitor::watch(&root.join("metrics.jsonl"), |monitor| {
            let fetcher = Fetcher {
                source: &source,
                identity: &identity,
                inputs: &inputs,
                catalog: &catalog,
                bundles: &[],
                earlier: &[],
                region: "r",
                monitor,
                fetched: Mutex::default(),
            };
            identify(&fetcher, &all, &mut manifest, 2).unwrap();
            let key = Key::of(b"UnityFS\0plain", INPUT_EXTENSION);
            assert_eq!(manifest[0].content, "declared h");
            assert_eq!(manifest[1].content, format!("bytes {key}"));
            assert_eq!(manifest[2].content, "");
            // The identified bundle is not fetched again; the lost one is tried again.
            assert_eq!(fetcher.fetch(&all[1]).unwrap(), (key, false));
            assert!(fetcher.fetch(&all[2]).is_err());
        })
        .unwrap();
        let fetched = source.fetched.into_inner().unwrap();
        assert_eq!(
            fetched,
            HashMap::from([("b/plain".to_owned(), 1), ("c/lost".to_owned(), 2)])
        );
        drop(catalog);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bundles_depend_on_the_bundles_that_hold_the_files_they_name() {
        let root = std::env::temp_dir().join(format!("sekai-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = LocalStore::open(root.join("store")).unwrap();
        let catalog = Catalog::open_or_rebuild(&root.join("catalog.redb"), &store).unwrap();
        let inputs = LocalStore::open(root.join("inputs")).unwrap();
        // The two fixture bundles name each other's serialized file as an external file.
        let fixture = |name: &str| {
            let fixtures =
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../sekai-unity/tests/fixtures/lzma");
            std::fs::read(fixtures.join(name)).unwrap()
        };
        let source = Bundles {
            bytes: HashMap::from([
                ("data".to_owned(), fixture("fixture_data")),
                ("scene".to_owned(), fixture("fixture_scene")),
            ]),
            ..Bundles::default()
        };
        let identity = identity();
        let all = [
            entry("data", &[]),
            entry("scene", &["data"]),
            entry("unread", &[]),
        ];
        let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
        let dependencies = |manifest: &[Entry]| -> Vec<Vec<String>> {
            manifest.iter().map(|e| e.dependencies.clone()).collect()
        };

        Monitor::watch(&root.join("metrics.jsonl"), |monitor| {
            let fetcher = || Fetcher {
                source: &source,
                identity: &identity,
                inputs: &inputs,
                catalog: &catalog,
                bundles: &[],
                earlier: &[],
                region: "r",
                monitor,
                fetched: Mutex::default(),
            };
            // A run fetches and reads the bundles it may fetch; a listed dependency stays once.
            let mut first = listed(&all);
            let fetchable = |name: &str| name != "unread";
            fetcher()
                .add_file_dependencies(&all, &mut first, fetchable, &cache, &NoBundleIndexes, 2)
                .unwrap();
            assert_eq!(dependencies(&first), [vec!["scene"], vec!["data"], vec![]]);
            // A later run finds the files in the catalog and fetches nothing.
            let mut later = listed(&all);
            fetcher()
                .add_file_dependencies(&all, &mut later, |_| false, &cache, &NoBundleIndexes, 2)
                .unwrap();
            assert_eq!(later, first);
        })
        .unwrap();
        let fetched = source.fetched.into_inner().unwrap();
        assert_eq!(
            fetched,
            HashMap::from([("data".to_owned(), 1), ("scene".to_owned(), 1)])
        );
        drop(catalog);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_bundle_is_taken_from_an_earlier_run_with_the_same_declared_hash() {
        let root = std::env::temp_dir().join(format!("sekai-earlier-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let output = root.join("earlier");
        std::fs::create_dir_all(&output).unwrap();
        let store = LocalStore::open(output.join("store")).unwrap();
        let catalog = Catalog::open_or_rebuild(&output.join("catalog.redb"), &store).unwrap();
        let earlier_inputs = LocalStore::open(output.join("inputs")).unwrap();
        let key = earlier_inputs
            .put(b"bundle bytes", INPUT_EXTENSION)
            .unwrap();
        let mut batch = catalog.write().unwrap();
        let row = BundleRow {
            declared: "h".to_owned(),
            sha256: key.to_string()[..64].to_owned(),
        };
        batch.put_bundle("r", "a/one", &row).unwrap();
        batch.commit().unwrap();
        drop(catalog);

        let runs = [EarlierRun::open(&output).unwrap()];
        let inputs = LocalStore::open(root.join("inputs")).unwrap();
        let found = from_earlier_runs(&runs, "r", &entry("a/one", &[]), &inputs).unwrap();
        assert_eq!(found, Some(key.clone()));
        assert_eq!(inputs.read(&key).unwrap(), b"bundle bytes");

        let mut changed = entry("a/one", &[]);
        changed.declared = "other".to_owned();
        assert_eq!(
            from_earlier_runs(&runs, "r", &changed, &inputs).unwrap(),
            None
        );
        assert_eq!(
            from_earlier_runs(&runs, "r", &entry("a/two", &[]), &inputs).unwrap(),
            None
        );
        drop(runs);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_selection_brings_its_dependencies() {
        let entries: Vec<Entry> = [
            entry("a/one", &["c/shared"]),
            entry("b/two", &[]),
            entry("c/shared", &["d/deep", "missing"]),
            entry("d/deep", &[]),
        ]
        .into_iter()
        .map(|entry| Entry {
            name: entry.name,
            size: entry.size,
            content: entry.declared,
            dependencies: entry.dependencies,
        })
        .collect();
        let selected = select(entries.clone(), &["a/".to_owned()]);
        assert_eq!(names(&selected), ["a/one", "c/shared", "d/deep"]);
        assert_eq!(select(entries, &[]).len(), 4);
    }
}
