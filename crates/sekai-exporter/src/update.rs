//! `sekai-exporter update`: the current version of a region, from its source to a publication.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;

use sekai_catalog::db::{BundleRow, Catalog, CostRow, IndexCache, PublicationRow};
use sekai_catalog::store::{ObjectWriter, parse_hex};
use sekai_catalog::{Key, LocalStore, Objects};
use sekai_convert::tables::TableConverter;
use sekai_format::identity::VersionIdentity;
use sekai_game::cdn::{self, BundleEntry, Kind, Region, Secrets, Source, Tables};
use sekai_game::obfuscation::Deobfuscating;
use sekai_game::sink::Sink;
use sekai_pipeline::metrics::{NoRecorder, Recorder};
use sekai_pipeline::reuse::Reuse;
use sekai_pipeline::stream::{self, Entry, InBundle, Inputs, Segment, Stream, Streamed};
use sekai_pipeline::{BundleInput, parallel};
use sekai_unity::{BoundedBlockCache, FileSource, NoClasses};

use crate::bucket::Bucket;
use crate::budget;
use crate::export::{
    BLOCK_CACHE_BYTES, Destination, IGNORED, converters, publish, report, write_ledger,
};
use crate::monitor::{self, Monitor};

const INPUT_EXTENSION: &str = "bundle";
/// How many fetched bundles are recorded together.
const RECORD_BATCH: usize = 1024;

struct Options {
    region: String,
    output: PathBuf,
    config: PathBuf,
    secrets: Option<PathBuf>,
    tree: Option<PathBuf>,
    storage: Option<PathBuf>,
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
    let mut tree = None;
    let mut storage = None;
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
            Long("tree") => tree = Some(PathBuf::from(parser.value()?)),
            Long("storage") => storage = Some(PathBuf::from(parser.value()?)),
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
        ("--tree", tree.is_some()),
        ("--storage", storage.is_some()),
    ];
    if let Some((flag, _)) = acts.into_iter().find(|(_, given)| check && *given) {
        return Err(format!("--check only compares the version, and {flag} acts on it").into());
    }
    Ok(Options {
        region: region.string()?,
        output: PathBuf::from(output),
        config: config.ok_or("--config is required")?,
        secrets,
        tree,
        storage,
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

fn previous_identity(catalog: &Catalog, region: &str) -> io::Result<Option<VersionIdentity>> {
    let Some(version) = catalog.pointer(region)? else {
        return Ok(None);
    };
    let Some(text) = catalog.version(region, &version)? else {
        return Ok(None);
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(io::Error::other)
}

/// The entries whose names start with one of `only`, with everything they depend on. Without
/// prefixes, every entry.
fn select(entries: Vec<Entry>, only: &[String]) -> Vec<Entry> {
    if only.is_empty() {
        return entries;
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
    let mut entries = entries;
    entries.retain(|entry| selected.contains(&entry.name));
    entries
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
        let source = FileSource::open(self.inputs.path(&key)).map_err(io::Error::other)?;
        Ok(BundleInput {
            name: name.to_owned(),
            source: Arc::new(source),
            sha256: key.sha256,
        })
    }
}

/// Why a selection of bundles cannot run here. A selection is not a version, so it must not
/// replace a complete run or remove files that `roots` already hold.
fn selection_conflict<'a>(
    complete_run: bool,
    mut roots: impl Iterator<Item = &'a Path>,
) -> Option<String> {
    if complete_run {
        return Some("--only needs an <output> that holds no complete run".to_owned());
    }
    roots
        .find(|root| std::fs::read_dir(root).is_ok_and(|mut files| files.next().is_some()))
        .map(|root| format!("--only needs an empty directory: {}", root.display()))
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
    /// The units of a manifest that lists the dependencies of its bundles, taking the results
    /// that earlier runs recorded.
    Listed(Reuse<'m>),
    /// A manifest without dependencies. They are read from the bundles themselves, and then the
    /// entries whose names start with one of `only` are selected, with what they depend on.
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

/// Fetches `entries` while `evaluating` runs, so the first segments convert while later bundles
/// are still on their way. Returns the evaluation and how many bundles could not be fetched.
/// Fails if the store, the records or the catalog do.
fn fetch_and_evaluate(
    fetcher: &Fetcher<'_>,
    entries: &[BundleEntry],
    concurrency: usize,
    evaluating: Evaluating<'_>,
    stream: &Stream<'_>,
) -> Result<(Streamed, usize), String> {
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
                stream::evaluate(&select(manifest, only), &arriving, stream)
            }
        };
        let streamed = evaluate().map_err(|e| e.to_string());
        let failed = fetching
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic))?;
        Ok((streamed?, failed))
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

/// The last version of `region` that `output` recorded, when it holds a catalog.
fn recorded_identity(output: &Path, region: &str) -> io::Result<Option<VersionIdentity>> {
    let database = output.join("catalog.redb");
    if !database.is_file() {
        return Ok(None);
    }
    let store = LocalStore::open(output.join("store"))?;
    previous_identity(&Catalog::open_or_rebuild(&database, &store)?, region)
}

/// Prints whether the content of the current version differs from the last one recorded: in the
/// bucket of the configuration when it names one, and otherwise in `options.output`. Nothing is
/// downloaded or written.
fn check(options: &Options) -> Result<bool, lexopt::Error> {
    let name = options.region.as_str();
    let (config, region) = configuration(options)?;
    let previous = match Bucket::open(&config, options.secrets.as_deref())? {
        Some(bucket) => bucket.last_version(name)?,
        None => recorded_identity(&options.output, name).map_err(|e| e.to_string())?,
    };
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
        Some(previous) if previous.same_content(&identity) => "unchanged".to_owned(),
        Some(previous) => format!("changed from {}", previous.name()),
        None => "changed".to_owned(),
    };
    println!("{name} {} {change}", identity.name());
    Ok(true)
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

/// Updates `options.region`. With a bucket, the run starts from the catalog the bucket keeps and
/// writes it back once the run has changed it, also when the run fails, since what it recorded
/// stays true. Then the last version of the region is written beside it, for checks that do not
/// pull the catalog. One run per bucket goes at a time. A selection runs on the local output
/// alone.
fn update(options: &Options, monitor: &Monitor) -> Result<bool, lexopt::Error> {
    let (config, region) = configuration(options)?;
    let bucket = if options.only.is_empty() {
        Bucket::open(&config, options.secrets.as_deref())?
    } else {
        None
    };
    let Some(bucket) = bucket else {
        return export_version(options, region, None, monitor).map(|run| run.succeeded());
    };
    let catalog = options.output.join("catalog.redb");
    bucket.pull_catalog(&catalog, monitor)?;
    let run = export_version(options, region, Some(&bucket), monitor);
    let pushed = match run {
        Ok(Outcome::Unchanged(_)) => Ok(()),
        _ => bucket.push_catalog(&catalog, monitor),
    };
    match (run, pushed) {
        (Ok(outcome), Ok(())) => {
            if let Some(identity) = outcome.version() {
                bucket.set_last_version(&options.region, identity)?;
            }
            Ok(outcome.succeeded())
        }
        (Ok(_), Err(push)) => Err(push),
        (Err(failure), Ok(())) => Err(failure),
        (Err(failure), Err(push)) => {
            Err(format!("{failure}; the catalog was not written back: {push}").into())
        }
    }
}

/// Exports the current version of `region` as `options` say, and publishes it to `bucket` when
/// there is one.
fn export_version(
    options: &Options,
    region: Region,
    bucket: Option<&Bucket>,
    monitor: &Monitor,
) -> Result<Outcome, lexopt::Error> {
    let region_name = options.region.as_str();
    // A directory lists no dependencies; they are read from the bundles once they are fetched.
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
    let inputs = directory("inputs")?;
    let derivations = Catalog::open_or_rebuild(&options.output.join("catalog.redb"), &store)
        .map_err(|e| e.to_string())?
        .into_derivations();
    let catalog = derivations.catalog();

    let previous = previous_identity(catalog, region_name).map_err(|e| e.to_string())?;
    let selection = !options.only.is_empty();
    if selection {
        let roots = [&options.tree, &options.storage];
        let roots = roots.into_iter().flatten().map(PathBuf::as_path);
        if let Some(conflict) = selection_conflict(previous.is_some(), roots) {
            return Err(conflict.into());
        }
    }
    let identity = current_identity(
        region_name,
        source.as_ref(),
        tables.as_ref(),
        previous.as_ref(),
        monitor,
    )?;
    let name = identity.name();
    if !(options.force || options.verify) && previous.is_some_and(|p| p.same_content(&identity)) {
        println!("{region_name} {name} unchanged");
        return Ok(Outcome::Unchanged(identity));
    }

    let held = catalog.holdings();
    if options.verify
        && let Some(bucket) = bucket
    {
        let listed = bucket.holdings(monitor)?;
        held.replace(&listed).map_err(|e| e.to_string())?;
        println!("bucket holds {} objects", listed.len());
    }
    let objects = match bucket {
        Some(bucket) => Objects {
            local: &store,
            held: &held,
            remote: bucket.remote(),
        },
        None => Objects::local(&store),
    };

    let all = {
        let _span = monitor.span("manifest", "manifest", region_name);
        source.manifest(&identity).map_err(|e| e.to_string())?
    };
    let mut manifest: Vec<Entry> = all
        .iter()
        .map(|entry| Entry {
            name: entry.name.clone(),
            size: entry.size,
            content: declared_content(&entry.declared),
            dependencies: entry.dependencies.clone(),
        })
        .collect();
    let fetcher = Fetcher {
        source: source.as_ref(),
        identity: &identity,
        inputs: &inputs,
        catalog,
        earlier: &earlier,
        region: region_name,
        monitor,
        fetched: Mutex::default(),
    };
    if listed {
        manifest = select(manifest, &options.only);
        identify(&fetcher, &all, &mut manifest, concurrency)?;
    }
    let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
    let indexes =
        IndexCache::open(&options.output.join("indexes.redb")).map_err(|e| e.to_string())?;
    let fusion = crate::storage::Fusion::new(&store, &derivations, monitor);
    let converters = converters(hca_key);
    let stream = Stream {
        converters: &converters,
        naming: &InBundle,
        store: &objects,
        derivations: &derivations,
        indexes: &indexes,
        cache: &cache,
        classes: &NoClasses,
        recorder: monitor,
        fusion: options
            .storage
            .is_some()
            .then_some(&fusion as &dyn sekai_pipeline::Fusion),
        options: budget::options(options.threads, options.memory, options.plain),
        segment: Segment::default(),
        ledger: Some(IGNORED),
        limit: None,
    };
    let evaluating = if listed {
        let reuse = if options.force {
            Reuse::refresh(&manifest, &stream, catalog)
        } else {
            Reuse::new(&manifest, &stream, catalog)
        };
        let reuse = reuse.map_err(|e| e.to_string())?;
        println!("units {} recorded {}", manifest.len(), reuse.reused());
        Evaluating::Listed(reuse)
    } else {
        Evaluating::Read {
            manifest,
            only: &options.only,
        }
    };
    let entries = in_opening_order(&all, evaluating.entries());
    let (mut streamed, failed) =
        fetch_and_evaluate(&fetcher, &entries, concurrency, evaluating, &stream)?;
    if failed > 0 || report(&streamed) > 0 {
        return Ok(Outcome::Incomplete);
    }
    if let Some(tables) = &tables {
        let read = |name: &str| tables.read(name).map_err(io::Error::other);
        sekai_pipeline::tables::evaluate(
            &sekai_pipeline::tables::TableStage {
                converters: &table_converters(),
                read: &read,
                store: &objects,
                recorder: monitor,
                threads: options.threads,
            },
            &mut streamed.evaluation,
        )
        .map_err(|e| e.to_string())?;
    }
    let evaluation = &streamed.evaluation;
    let target = format!("{region_name}/canonical");
    let (pointer, plan) = publish(
        evaluation,
        &name,
        &target,
        &Destination {
            output: &options.output,
            tree: options.tree.as_deref(),
        },
        &objects,
        options.threads,
        monitor,
    )?;
    write_ledger(&streamed.ledgers, &options.output, monitor)?;
    let storage_target = format!("{region_name}/storage");
    let storage = crate::storage::publish(
        &plan,
        &storage_target,
        &Destination {
            output: &options.output,
            tree: options.storage.as_deref(),
        },
        &objects,
        &derivations,
        options.threads,
        monitor,
    )?;
    if let Some(bucket) = bucket {
        let storage = storage.as_ref().map(|plan| (storage_target.as_str(), plan));
        bucket.publish(
            &objects,
            (&target, &plan),
            storage,
            options.threads,
            monitor,
        )?;
    }

    let record = || -> io::Result<()> {
        let text = serde_json::to_string(&identity).map_err(io::Error::other)?;
        let mut batch = catalog.write()?;
        if !selection {
            batch.put_version(region_name, &name, &text)?;
            batch.set_pointer(region_name, &name)?;
            batch.put_publication(
                "local",
                &target,
                &PublicationRow {
                    version: name.clone(),
                    revision: pointer.revision.clone(),
                },
            )?;
        }
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
    record().map_err(|e| e.to_string())?;

    let errored = evaluation.outputs.iter().filter(|o| o.error.is_some());
    println!(
        "{region_name} {name} published revision {} assets {} errors {} unnamed {} converted {} reused {}",
        pointer.revision,
        evaluation.outputs.len(),
        errored.count(),
        evaluation.unnamed.len(),
        evaluation.converted,
        evaluation.reused
    );
    Ok(Outcome::Updated(identity))
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
        let identity = VersionIdentity {
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
        };
        let mut undeclared = entry("b/plain", &[]);
        undeclared.declared = String::new();
        let mut lost = entry("c/lost", &[]);
        lost.declared = String::new();
        let all = [entry("a/declared", &[]), undeclared, lost];
        let mut manifest: Vec<Entry> = all
            .iter()
            .map(|entry| Entry {
                name: entry.name.clone(),
                size: entry.size,
                content: declared_content(&entry.declared),
                dependencies: entry.dependencies.clone(),
            })
            .collect();

        Monitor::watch(&root.join("metrics.jsonl"), |monitor| {
            let fetcher = Fetcher {
                source: &source,
                identity: &identity,
                inputs: &inputs,
                catalog: &catalog,
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
    fn a_selection_keeps_away_from_complete_runs_and_published_files() {
        let root = std::env::temp_dir().join(format!("sekai-selection-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (empty, used) = (root.join("empty"), root.join("used"));
        std::fs::create_dir_all(&empty).unwrap();
        std::fs::create_dir_all(&used).unwrap();
        std::fs::write(used.join("file"), b"x").unwrap();
        let missing = root.join("missing");

        let roots = [empty.as_path(), missing.as_path()];
        assert_eq!(selection_conflict(false, roots.into_iter()), None);
        assert!(selection_conflict(true, roots.into_iter()).is_some());
        let roots = [empty.as_path(), used.as_path()];
        let conflict = selection_conflict(false, roots.into_iter()).unwrap();
        assert!(conflict.contains("used"), "{conflict}");
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
