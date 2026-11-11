//! `sekai-exporter update`: the current version of a region, from its source to a publication.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sekai_catalog::db::{BundleRow, Catalog, CostRow, PublicationRow};
use sekai_catalog::store::{ObjectWriter, parse_hex};
use sekai_catalog::{Key, LocalStore};
use sekai_convert::tables::TableConverter;
use sekai_format::identity::VersionIdentity;
use sekai_game::cdn::{self, BundleEntry, Kind, Region, Secrets, Source, Tables};
use sekai_game::obfuscation::Deobfuscating;
use sekai_game::sink::Sink;
use sekai_pipeline::metrics::Recorder;
use sekai_pipeline::stream::{self, Entry, InBundle, Inputs, Segment, Stream};
use sekai_pipeline::{BundleInput, parallel};
use sekai_publish::Plan;
use sekai_unity::{BoundedBlockCache, FileSource, NoClasses};

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
    })
}

pub(crate) fn run(args: impl Iterator<Item = OsString>) -> Result<bool, lexopt::Error> {
    let options = parse(args)?;
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

/// The bundles of the input store, by name.
struct Stored<'s> {
    inputs: &'s LocalStore,
    keys: HashMap<String, Key>,
}

impl Inputs for Stored<'_> {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        let key = self.keys.get(name).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("{name} was not fetched"))
        })?;
        let source = FileSource::open(self.inputs.path(key)).map_err(io::Error::other)?;
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

fn download(
    source: &dyn Source,
    identity: &VersionIdentity,
    entry: &BundleEntry,
    inputs: &LocalStore,
    monitor: &Monitor,
) -> Result<Key, String> {
    let mut span = monitor.span("fetch", "bundle", &entry.name);
    let writer = inputs
        .writer(INPUT_EXTENSION)
        .map_err(|e| format!("input store: {e}"))?;
    let mut sink = Deobfuscating::new(Incoming(writer));
    let read = source
        .fetch(identity, entry, &mut sink)
        .map_err(|e| e.to_string())?;
    let Incoming(writer) = sink.finish().map_err(|e| e.to_string())?;
    let (key, size) = writer.finish().map_err(|e| format!("input store: {e}"))?;
    span.bytes(read, size);
    Ok(key)
}

/// Makes every entry available in `inputs`, and returns the entries with their keys and the number
/// of bundles that could not be fetched.
#[allow(clippy::too_many_arguments)]
fn fetch_all(
    source: &dyn Source,
    identity: &VersionIdentity,
    entries: &[BundleEntry],
    inputs: &LocalStore,
    catalog: &Catalog,
    earlier: &[EarlierRun],
    region: &str,
    concurrency: usize,
    monitor: &Monitor,
) -> Result<(Vec<(String, Key)>, usize), lexopt::Error> {
    let mut stored = Vec::new();
    let mut failed = 0;
    let mut fetched = 0;
    for chunk in entries.chunks(RECORD_BATCH) {
        let results = parallel(chunk.len(), concurrency, |i| {
            match already_stored(catalog, region, &chunk[i], inputs) {
                Ok(Some(key)) => Ok((key, false)),
                Ok(None) => match from_earlier_runs(earlier, region, &chunk[i], inputs) {
                    Ok(Some(key)) => Ok((key, true)),
                    Ok(None) => download(source, identity, &chunk[i], inputs, monitor)
                        .map(|key| (key, true)),
                    Err(e) => Err(e.to_string()),
                },
                Err(e) => Err(e.to_string()),
            }
        });
        let mut batch = catalog.write().map_err(|e| e.to_string())?;
        for (entry, result) in chunk.iter().zip(results) {
            match result {
                Ok((key, new)) => {
                    if new {
                        fetched += 1;
                        let row = BundleRow {
                            declared: entry.declared.clone(),
                            sha256: key.to_string()[..64].to_owned(),
                        };
                        batch
                            .put_bundle(region, &entry.name, &row)
                            .map_err(|e| e.to_string())?;
                    }
                    stored.push((entry.name.clone(), key));
                }
                Err(e) => {
                    failed += 1;
                    eprintln!("{}: {e}", entry.name);
                }
            }
        }
        batch.commit().map_err(|e| e.to_string())?;
    }
    println!(
        "bundles {} fetched {fetched} reused {} failed {failed}",
        entries.len(),
        stored.len() - fetched
    );
    Ok((stored, failed))
}

fn update(options: &Options, monitor: &Monitor) -> Result<bool, lexopt::Error> {
    let region_name = options.region.as_str();
    let config = std::fs::read_to_string(&options.config)
        .map_err(|e| format!("{}: {e}", options.config.display()))?;
    let region = Region::load(region_name, &config).map_err(|e| e.to_string())?;
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
    let tables = region
        .tables
        .as_ref()
        .map(|source| Tables::open(source, &region.network))
        .transpose()
        .map_err(|e| e.to_string())?;
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
    let mut identity = {
        let _span = monitor.span("plan", "identity", region_name);
        source
            .identity(previous.as_ref())
            .map_err(|e| e.to_string())?
    };
    identity.tables = tables.as_ref().map(|tables| tables.revision().to_owned());
    let name = identity.name();
    if !(options.force || options.verify) && previous.is_some_and(|p| p.same_content(&identity)) {
        println!("{region_name} {name} unchanged");
        return Ok(true);
    }

    let all = {
        let _span = monitor.span("manifest", "manifest", region_name);
        source.manifest(&identity).map_err(|e| e.to_string())?
    };
    let mut manifest: Vec<Entry> = all
        .iter()
        .map(|entry| Entry {
            name: entry.name.clone(),
            size: entry.size,
            dependencies: entry.dependencies.clone(),
        })
        .collect();
    if listed {
        manifest = select(manifest, &options.only);
    }
    let wanted: BTreeSet<&str> = manifest.iter().map(|entry| entry.name.as_str()).collect();
    let entries: Vec<BundleEntry> = all
        .iter()
        .filter(|entry| wanted.contains(entry.name.as_str()))
        .cloned()
        .collect();
    let (stored, failed) = fetch_all(
        source.as_ref(),
        &identity,
        &entries,
        &inputs,
        catalog,
        &earlier,
        region_name,
        concurrency,
        monitor,
    )?;
    if failed > 0 {
        return Ok(false);
    }

    let stored = Stored {
        inputs: &inputs,
        keys: stored.into_iter().collect(),
    };
    let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
    let indexes = catalog.bundle_indexes();
    if !listed {
        let _span = monitor.span("manifest", "dependencies", region_name);
        stream::with_dependencies(&mut manifest, &stored, &cache, &indexes, options.threads)
            .map_err(|e| e.to_string())?;
        manifest = select(manifest, &options.only);
    }

    let fusion = crate::storage::Fusion::new(&store, &derivations, monitor);
    let converters = converters(hca_key);
    let mut streamed = stream::evaluate(
        &manifest,
        &stored,
        &Stream {
            converters: &converters,
            naming: &InBundle,
            store: &store,
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
        },
    )
    .map_err(|e| e.to_string())?;
    if report(&streamed) > 0 {
        return Ok(false);
    }
    if let Some(tables) = &tables {
        let read = |name: &str| tables.read(name).map_err(io::Error::other);
        sekai_pipeline::tables::evaluate(
            &sekai_pipeline::tables::TableStage {
                converters: &table_converters(),
                read: &read,
                store: &store,
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
        &store,
        options.threads,
        monitor,
    )?;
    write_ledger(&streamed.ledgers, &options.output, monitor)?;
    if !selection {
        publish_bucket(&config, options, &store, &target, &plan, monitor)?;
    }

    if let Some(root) = &options.storage {
        crate::storage::publish(
            &plan,
            &name,
            root,
            &store,
            &derivations,
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
    Ok(true)
}

/// The `publish.s3` section of the configuration, if there is one.
fn bucket_section(config: &str) -> Result<Option<serde_json::Value>, lexopt::Error> {
    let config: serde_json::Value = serde_json::from_str(config).map_err(|e| e.to_string())?;
    Ok(config.get("publish").and_then(|p| p.get("s3")).cloned())
}

/// The value of the secret `name`: the environment variable of that name, or else the file of that
/// name in `secrets`, without surrounding whitespace.
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

#[cfg(feature = "s3")]
fn secret(name: &str, secrets: Option<&Path>) -> Result<String, lexopt::Error> {
    if let Ok(value) = std::env::var(name) {
        return Ok(value.trim().to_owned());
    }
    secrets
        .and_then(|directory| std::fs::read_to_string(directory.join(name)).ok())
        .map(|value| value.trim().to_owned())
        .ok_or_else(|| format!("the secret {name} is not set").into())
}

/// Publishes `plan` to the bucket that the `publish.s3` section of the configuration describes,
/// with the secrets of `options`. With `--verify`, what the bucket holds is learned from a listing
/// of its whole store.
#[cfg(feature = "s3")]
fn publish_bucket(
    config: &str,
    options: &Options,
    store: &LocalStore,
    target: &str,
    plan: &Plan,
    monitor: &Monitor,
) -> Result<(), lexopt::Error> {
    use sekai_publish::s3::{self, Bucket, Check, Credentials, S3, S3Layout};

    let secrets = options.secrets.as_deref();

    let Some(section) = bucket_section(config)? else {
        return Ok(());
    };
    let text = |value: Option<&serde_json::Value>, key: &str| {
        value
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| lexopt::Error::from(format!("publish.s3.{key} is not a string")))
    };
    let names = section.get("secrets");
    let credentials = Credentials {
        access_key_id: secret(
            &text(
                names.and_then(|n| n.get("access_key_id")),
                "secrets.access_key_id",
            )?,
            secrets,
        )?,
        secret_access_key: secret(
            &text(
                names.and_then(|n| n.get("secret_access_key")),
                "secrets.secret_access_key",
            )?,
            secrets,
        )?,
    };
    let bucket = Bucket {
        endpoint: text(section.get("endpoint"), "endpoint")?,
        region: text(section.get("region"), "region")?,
        prefix: match section.get("prefix") {
            None => String::new(),
            value => text(value, "prefix")?,
        },
        connections: match section.get("connections") {
            None => 8,
            Some(value) => value
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or("publish.s3.connections is not a number")?,
        },
    };
    let layout = match section.get("layout") {
        None => S3Layout::Sqlite(parse_sqlite_options(&section)?),
        Some(value) => match text(Some(value), "layout")?.as_str() {
            "sqlite" => S3Layout::Sqlite(parse_sqlite_options(&section)?),
            "index" => S3Layout::Index,
            other => {
                return Err(
                    format!("publish.s3.layout {other:?} must be \"sqlite\" or \"index\"").into(),
                );
            }
        },
    };
    let s3 = S3::new(bucket, credentials).map_err(|e| e.to_string())?;
    let _span = monitor.span("publish", "s3", target);
    let check = if options.verify {
        Check::Listing
    } else {
        Check::Pointer
    };
    let (_, report) = s3::publish_with(store, &s3, target, plan, &layout, check, options.threads)
        .map_err(|e| e.to_string())?;
    println!(
        "bucket uploaded {} present {} pointer {}",
        report.uploaded,
        report.present,
        if report.pointer_written {
            "written"
        } else {
            "unchanged"
        }
    );
    Ok(())
}

#[cfg(feature = "s3")]
fn parse_sqlite_options(
    section: &serde_json::Value,
) -> Result<sekai_publish::sqlite::SqliteOptions, lexopt::Error> {
    use sekai_publish::sqlite::SqliteOptions;
    let Some(value) = section.get("pack_forms") else {
        return Ok(SqliteOptions::default());
    };
    let array = value
        .as_array()
        .ok_or("publish.s3.pack_forms must be an array of strings")?;
    let mut pack_forms = std::collections::BTreeSet::new();
    for item in array {
        let form = item
            .as_str()
            .ok_or("publish.s3.pack_forms entries must be strings")?;
        pack_forms.insert(form.to_owned());
    }
    Ok(SqliteOptions { pack_forms })
}

/// Refuses a configuration that names a bucket, which this build cannot publish to.
#[cfg(not(feature = "s3"))]
fn publish_bucket(
    config: &str,
    _options: &Options,
    _store: &LocalStore,
    _target: &str,
    _plan: &Plan,
    _monitor: &Monitor,
) -> Result<(), lexopt::Error> {
    match bucket_section(config)? {
        Some(_) => {
            Err("the configuration names an S3 bucket, but this build has no s3 feature".into())
        }
        None => Ok(()),
    }
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
            dependencies: entry.dependencies,
        })
        .collect();
        let selected = select(entries.clone(), &["a/".to_owned()]);
        assert_eq!(names(&selected), ["a/one", "c/shared", "d/deep"]);
        assert_eq!(select(entries, &[]).len(), 4);
    }
}
