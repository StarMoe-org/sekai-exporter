//! `sekai-exporter update`: the current version of a region, from its source to a publication.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sekai_catalog::db::{BundleRow, Catalog, CostRow, PublicationRow};
use sekai_catalog::store::{ObjectWriter, parse_hex};
use sekai_catalog::{Key, LocalStore};
use sekai_format::identity::VersionIdentity;
use sekai_game::cdn::{self, BundleEntry, Region, Secrets, Source};
use sekai_game::obfuscation::Deobfuscating;
use sekai_game::sink::Sink;
use sekai_pipeline::metrics::Recorder;
use sekai_pipeline::{BundleInput, Version, evaluate, parallel};
use sekai_publish::Plan;
use sekai_unity::{BoundedBlockCache, FileSource};

use crate::export::{BLOCK_CACHE_BYTES, converters, publish};
use crate::monitor::Monitor;

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
    force: bool,
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
    let mut force = false;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("config") => config = Some(PathBuf::from(parser.value()?)),
            Long("secrets") => secrets = Some(PathBuf::from(parser.value()?)),
            Long("tree") => tree = Some(PathBuf::from(parser.value()?)),
            Long("storage") => storage = Some(PathBuf::from(parser.value()?)),
            Long("only") => only.push(parser.value()?.string()?),
            Long("threads") => threads = parser.value()?.parse()?,
            Long("force") => force = true,
            Value(v) => positional.push(v),
            _ => return Err(arg.unexpected()),
        }
    }
    let [region, output]: [OsString; 2] = positional
        .try_into()
        .map_err(|_| lexopt::Error::from("expected <region> and <output>"))?;
    Ok(Options {
        region: region.string()?,
        output: PathBuf::from(output),
        config: config.ok_or("--config is required")?,
        secrets,
        tree,
        storage,
        only,
        threads: threads.max(1),
        force,
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
fn select(entries: Vec<BundleEntry>, only: &[String]) -> Vec<BundleEntry> {
    if only.is_empty() {
        return entries;
    }
    let by_name: BTreeMap<&str, &BundleEntry> =
        entries.iter().map(|e| (e.name.as_str(), e)).collect();
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
        let mut writer = inputs.writer(INPUT_EXTENSION)?;
        io::copy(&mut File::open(run.inputs.path(&key))?, &mut writer)?;
        return writer.finish().map(|(key, _)| Some(key));
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
    let identity = {
        let _span = monitor.span("plan", "identity", region_name);
        source
            .identity(previous.as_ref())
            .map_err(|e| e.to_string())?
    };
    let name = identity.asset.name();
    if !options.force && previous.is_some_and(|p| p.asset == identity.asset) {
        println!("{region_name} {name} unchanged");
        return Ok(true);
    }

    let entries = {
        let _span = monitor.span("manifest", "manifest", region_name);
        select(
            source.manifest(&identity).map_err(|e| e.to_string())?,
            &options.only,
        )
    };
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

    let bundles = stored
        .into_iter()
        .map(|(name, key)| {
            let source = FileSource::open(inputs.path(&key)).map_err(|e| e.to_string())?;
            Ok(BundleInput {
                name,
                source: Arc::new(source),
                sha256: key.sha256,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
    let (version, errors) = {
        let _span = monitor.span("open", "version", region_name);
        Version::open_indexed(bundles, &cache, &catalog.bundle_indexes(), options.threads)
            .map_err(|e| e.to_string())?
    };
    for e in &errors {
        eprintln!("{}: {}: {}", e.name, e.code, e.message);
    }
    if !errors.is_empty() {
        return Ok(false);
    }

    let evaluation = evaluate(
        &version,
        &converters(hca_key),
        &store,
        &derivations,
        monitor,
        options.threads,
    )
    .map_err(|e| e.to_string())?;
    let target = format!("{region_name}/canonical");
    let (pointer, plan) = publish(
        &evaluation,
        &name,
        &target,
        &options.output,
        options.tree.as_deref(),
        &store,
        monitor,
    )?;
    publish_bucket(
        &config,
        options.secrets.as_deref(),
        &store,
        &target,
        &plan,
        monitor,
    )?;

    if let Some(root) = &options.storage {
        crate::storage::publish(
            &evaluation,
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
        for (subject, cost) in monitor.costs() {
            batch.put_cost(&CostRow {
                kind: cost.kind,
                subject,
                wall_us: cost.wall_us,
                cpu_us: cost.cpu_us,
                read: cost.read,
                written: cost.written,
                estimated_memory: cost.estimated_memory,
                actual_memory: cost.actual_memory,
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

/// Publishes `plan` to the bucket that the `publish.s3` section of the configuration describes.
#[cfg(feature = "s3")]
fn publish_bucket(
    config: &str,
    secrets: Option<&Path>,
    store: &LocalStore,
    target: &str,
    plan: &Plan,
    monitor: &Monitor,
) -> Result<(), lexopt::Error> {
    use sekai_publish::s3::{self, Bucket, Credentials, S3};

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
    let s3 = S3::new(bucket, credentials).map_err(|e| e.to_string())?;
    let _span = monitor.span("publish", "s3", target);
    let (_, report) = s3::publish(store, &s3, target, plan).map_err(|e| e.to_string())?;
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

/// Refuses a configuration that names a bucket, which this build cannot publish to.
#[cfg(not(feature = "s3"))]
fn publish_bucket(
    config: &str,
    _secrets: Option<&Path>,
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

    fn names(entries: &[BundleEntry]) -> Vec<&str> {
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
    fn a_selection_brings_its_dependencies() {
        let entries = vec![
            entry("a/one", &["c/shared"]),
            entry("b/two", &[]),
            entry("c/shared", &["d/deep", "missing"]),
            entry("d/deep", &[]),
        ];
        let selected = select(entries.clone(), &["a/".to_owned()]);
        assert_eq!(names(&selected), ["a/one", "c/shared", "d/deep"]);
        assert_eq!(select(entries, &[]).len(), 4);
    }
}
