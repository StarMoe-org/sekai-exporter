//! `sekai-exporter update`: the current version of a region, from its source to a publication.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::SystemTime;

use sekai_catalog::db::{Catalog, CatalogDerivations, CostRow, IndexCache};
use sekai_catalog::store::parse_hex;
use sekai_catalog::{Key, LocalStore, MemoryHoldings, Objects, Store};
use sekai_convert::Converter;
use sekai_format::AssetDocument;
use sekai_format::identity::{RecordedVersion, VersionIdentity};
use sekai_format::publication;
use sekai_game::cdn::{self, BundleEntry, Kind, Region, Secrets, Source, Tables};
use sekai_pipeline::claims::Claims;
use sekai_pipeline::metrics::{NoRecorder, Recorder};
use sekai_pipeline::parallel;
use sekai_pipeline::records::{Earlier, Records, UNIT_RECORDS};
use sekai_pipeline::reuse::{self, Reuse, UnitResult};
use sekai_pipeline::state::{
    BundleState, MemoryFiles, RegionState, Root, StateFiles, TableOutput, Written,
};
use sekai_pipeline::stream::{self, Entry, InBundle, Segment, Stream};
use sekai_pipeline::tables::{Built, TableStage};
use sekai_publish::patch::{self, Change, Edition, Lookup, Previous};
use sekai_publish::{Binding, Layout, Object, Publication};
use sekai_unity::{BoundedBlockCache, ClassLayouts, NoClasses};

use crate::bucket::Bucket;
use crate::budget;
use crate::export::{BLOCK_CACHE_BYTES, IGNORED, converters, report, write_ledger};
use crate::monitor::{self, Monitor};

mod backed;
mod fetch;
mod files;
mod site;
mod tables;
mod views;

use backed::Backed;
use fetch::{Arriving, EarlierRun, Fetcher, declared_content, identify, in_opening_order};
use files::KnownIndexes;
use site::Site;
use tables::{build_tables, table_bindings, table_converters};
use views::Advance;

/// The size limit of the assets of a run: none.
const LIMIT: Option<u64> = None;

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
    /// Whether every asset built from tables is built again, whatever the state keeps of it.
    rebuild_tables: bool,
    /// Whether the version of the source is only compared with the last one recorded.
    check: bool,
    /// Whether the run publishes the client package of the region instead of its assets.
    client: bool,
}

impl Options {
    /// The name the state of the run is kept under: the region, or `<region>-client` for its
    /// client package, so that the two never share a state.
    fn state_key(&self) -> String {
        if self.client {
            format!("{}-client", self.region)
        } else {
            self.region.clone()
        }
    }

    /// The targets the run publishes, the one in the canonical layout first.
    fn targets(&self) -> Vec<String> {
        let region = &self.region;
        if self.client {
            vec![client_target(region)]
        } else {
            vec![canonical_target(region), storage_target(region)]
        }
    }
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
    let mut rebuild_tables = false;
    let mut check = false;
    let mut client = false;
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
            Long("rebuild-tables") => rebuild_tables = true,
            Long("check") => check = true,
            Long("client") => client = true,
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
        ("--rebuild-tables", rebuild_tables),
        ("--only", !only.is_empty()),
    ];
    if let Some((flag, _)) = acts.into_iter().find(|(_, given)| check && *given) {
        return Err(format!("--check only compares the version, and {flag} acts on it").into());
    }
    if client && rebuild_tables {
        return Err("--rebuild-tables builds tables, and a client package has none".into());
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
        rebuild_tables,
        check,
        client,
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

/// The tables of `region`, when its configuration names a source for them and the run publishes
/// its assets.
fn open_tables(options: &Options, region: &Region) -> Result<Option<Tables>, String> {
    let source = region.tables.as_ref().filter(|_| !options.client);
    source
        .map(|source| Tables::open(source, &region.network))
        .transpose()
        .map_err(|e| e.to_string())
}

/// What a run reads its bundles from, with the layouts they are read with.
pub(crate) struct Supply {
    pub(crate) source: Box<dyn Source>,
    /// The layouts of the engine classes and scripts that the bundles store without them.
    pub(crate) classes: Box<dyn ClassLayouts>,
}

/// The supply of the run: the client package of `region` with `--client`, and otherwise its
/// distribution source, whose bundles carry their layouts.
fn open_source(options: &Options, region: Region, secrets: &Secrets) -> Result<Supply, String> {
    if options.client {
        let package = region
            .package
            .ok_or_else(|| format!("the region {} has no package", options.region))?;
        return crate::client::update_source(&package);
    }
    let source = cdn::open(region, secrets).map_err(|e| e.to_string())?;
    Ok(Supply {
        source,
        classes: Box::new(NoClasses),
    })
}

/// The converters of a run on `region`, with the secrets of `secrets`.
fn converters_of(region: &Region, secrets: &Secrets) -> Result<Vec<Box<dyn Converter>>, String> {
    let hca_key = secrets
        .hca_key(&region.secrets)
        .map_err(|e| e.to_string())?;
    Ok(converters(hca_key))
}

/// What the assets of a run with `converters` and the class layouts `classes` follow from
/// besides its inputs, by part: the identity of the evaluation of its units and, when it builds
/// assets from tables, the form and version of each table converter.
fn evaluation_identity(
    converters: &[Box<dyn Converter>],
    classes: &dyn ClassLayouts,
    tables: bool,
) -> BTreeMap<String, String> {
    let mut identity = reuse::evaluation_identity(converters, classes, LIMIT);
    if tables {
        identity.extend(sekai_pipeline::tables::evaluation_identity(
            &table_converters(),
        ));
    }
    identity
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
    let key = options.state_key();
    let name = key.as_str();
    let (config, region) = configuration(options)?;
    let bucket = Bucket::open(&config, options.secrets.as_deref())?;
    let site = bucket
        .as_ref()
        .map_or_else(|| Site::local(&options.output), Site::Bucket);
    let previous = site::last_version(site.files(), name)?;
    let tables = open_tables(options, &region)?;
    let secrets = Secrets::new(options.secrets.clone());
    let converters = converters_of(&region, &secrets)?;
    let Supply { source, classes } = open_source(options, region, &secrets)?;
    let recorded = RecordedVersion {
        identity: current_identity(
            name,
            source.as_ref(),
            tables.as_ref(),
            previous.as_ref().map(|recorded| &recorded.identity),
            &NoRecorder,
        )?,
        evaluation: evaluation_identity(&converters, classes.as_ref(), tables.is_some()),
    };
    let change = change(&site, &options.targets(), previous.as_ref(), &recorded)?;
    let described = change.as_deref().unwrap_or("unchanged");
    println!("{name} {} {described}", recorded.identity.name());
    Ok(change.is_none())
}

/// What a run that would record `current` would change at `site` after the run that recorded
/// `previous`, or `None` when the version has the same content, the evaluation the same identity
/// and every target of the run is published in the current publication format.
fn change(
    site: &Site<'_>,
    targets: &[String],
    previous: Option<&RecordedVersion>,
    current: &RecordedVersion,
) -> Result<Option<String>, String> {
    let Some(previous) = previous else {
        return Ok(Some("changed".to_owned()));
    };
    if !previous.identity.same_content(&current.identity) {
        return Ok(Some(format!("changed from {}", previous.identity.name())));
    }
    if previous.evaluation.is_empty() {
        return Ok(Some(
            "changed: the last run recorded no evaluation".to_owned(),
        ));
    }
    let parts = previous.changed_parts(&current.evaluation);
    if !parts.is_empty() {
        let parts = parts.join(", ");
        return Ok(Some(format!("changed: the evaluation of {parts} differs")));
    }
    for target in targets {
        let current = site.current(target)?;
        if current.is_none_or(|pointer| pointer.version != publication::POINTER_VERSION) {
            return Ok(Some(format!("changed: {target} is in an earlier format")));
        }
    }
    Ok(None)
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
    let key = options.state_key();
    let opened = Opened::open(options, region)?;
    // A selection is not a version: it keeps no state and records nothing.
    let unkept = MemoryFiles::default();
    let selection = !options.only.is_empty();
    let files: &dyn StateFiles = if selection { &unkept } else { site.files() };
    let previous = site::last_version(site.files(), &key)?;
    if selection && previous.is_some() {
        return Err("--only needs an <output> that holds no complete run".into());
    }
    let recorded = RecordedVersion {
        identity: current_identity(
            &key,
            opened.source.as_ref(),
            opened.tables.as_ref(),
            previous.as_ref().map(|recorded| &recorded.identity),
            monitor,
        )?,
        evaluation: opened.evaluation_identity(),
    };
    if !(options.force || options.verify)
        && change(site, &options.targets(), previous.as_ref(), &recorded)?.is_none()
    {
        println!("{key} {} unchanged", recorded.identity.name());
        return Ok(Outcome::Unchanged(recorded.identity));
    }

    let Listed { held, unit_records } = listed(options, site, monitor)?;
    let objects = Objects {
        local: &opened.store,
        held: &held,
        remote: site.remote(),
    };
    let records = match &unit_records {
        Some(unit_records) => Records::listed(files, &held, unit_records),
        None => Records::new(files, &held),
    };
    let requests = site.requests(options.threads);
    let backed = Backed {
        records: &records,
        catalog: &opened.catalog,
        requests,
    };
    let baseline = {
        let _span = monitor.span("plan", "baseline", &key);
        Baseline::read(files, site, &key, &options.targets(), &objects, requests)?
    };
    let Baseline {
        mut state,
        root,
        publications,
    } = baseline;
    let evaluation = opened.evaluate(
        options,
        &recorded.identity,
        &objects,
        &backed,
        &state,
        monitor,
    )?;
    let Some(Evaluation { outcome, bundles }) = evaluation else {
        return Ok(Outcome::Incomplete);
    };
    let rulings = state.advance(bundles, &outcome);

    let context = Context {
        options,
        site,
        files,
        store: &opened.store,
        objects: &objects,
        backed: &backed,
        monitor,
        published_at: SystemTime::now(),
    };
    let advance = Advance {
        rulings: &rulings,
        state: &state,
        results: &outcome.results,
        records: &backed,
    };
    let name = recorded.identity.name();
    let views = context.publish(&name, &publications, &advance, &opened)?;
    let committed = context.commit(&outcome.results, state, root.as_ref(), &views, &recorded)?;
    record_costs(opened.catalog.catalog(), monitor)?;
    print_summary(&key, &committed, &views, &outcome);
    Ok(Outcome::Updated(recorded.identity))
}

/// What a run reads from: the source of the region and its tables, the directories of the output
/// and the converters.
struct Opened {
    source: Box<dyn Source>,
    /// The layouts of the engine classes and scripts of the bundles of the source.
    classes: Box<dyn ClassLayouts>,
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
        let converters = converters_of(&region, &secrets)?;
        let tables = open_tables(options, &region)?;
        let Supply { source, classes } = open_source(options, region, &secrets)?;
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
            classes,
            tables,
            inputs: directory("inputs")?,
            store,
            catalog,
            earlier,
            converters,
            listed,
            concurrency,
        })
    }

    /// The identity of the evaluation of a run, as [`evaluation_identity`] gives it.
    fn evaluation_identity(&self) -> BTreeMap<String, String> {
        evaluation_identity(
            &self.converters,
            self.classes.as_ref(),
            self.tables.is_some(),
        )
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
        let key = options.state_key();
        let all = {
            let _span = monitor.span("manifest", "manifest", &key);
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
            region: &key,
            monitor,
            fetched: Mutex::default(),
        };
        let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
        let indexes =
            IndexCache::open(&options.output.join("indexes.redb")).map_err(|e| e.to_string())?;
        // A manifest without dependencies has its bundles read as they are evaluated, so the
        // state learns nothing of their files.
        let bundles = if self.listed {
            let selected = selected(&manifest, &options.only);
            let fetchable = |name: &str| selected.as_ref().is_none_or(|names| names.contains(name));
            let concurrency = self.concurrency;
            let bundles = fetcher.add_file_dependencies(
                &all,
                &mut manifest,
                fetchable,
                &cache,
                &indexes,
                concurrency,
            )?;
            manifest = select(manifest, &options.only);
            identify(&fetcher, &all, &mut manifest, concurrency)?;
            bundles
        } else {
            Vec::new()
        };
        let storage = crate::storage::Fusion::new(&self.store, backed, monitor);
        // Only the assets of a region have a storage view, whose files fusion derives.
        let fusion: Option<&dyn sekai_pipeline::Fusion> = (!options.client).then_some(&storage);
        let stream = Stream {
            converters: &self.converters,
            naming: &InBundle,
            store: objects,
            derivations: backed,
            indexes: &indexes,
            cache: &cache,
            classes: self.classes.as_ref(),
            recorder: monitor,
            fusion,
            options: budget::options(options.threads, options.memory, options.plain),
            segment: Segment::default(),
            ledger: Some(IGNORED),
            limit: LIMIT,
        };
        let earlier = Earlier::new(state, backed);
        let known = KnownIndexes {
            fetcher: &fetcher,
            indexes: &indexes,
        };
        let evaluating = if self.listed {
            let reuse = if options.force {
                Reuse::refresh(&manifest, &stream)
            } else {
                Reuse::new(&manifest, &stream, &earlier, backed.requests, &known)
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
    /// What the bundles of the version hold and name, for those whose files the run knows.
    bundles: Vec<BundleState>,
}

/// What `site` is known to hold before the run.
struct Listed {
    /// The objects of the store: with `--verify`, those a listing of the bucket finds, and else
    /// none until the run reads records or puts objects there.
    held: MemoryHoldings,
    /// With `--verify`, the unit records a listing of the bucket finds; else each record the run
    /// needs is looked up.
    unit_records: Option<Vec<String>>,
}

/// What a listing of `site` finds before the run, with `--verify`.
fn listed(options: &Options, site: &Site<'_>, monitor: &Monitor) -> Result<Listed, lexopt::Error> {
    match site {
        Site::Bucket(bucket) if options.verify => {
            let held = bucket.holdings(monitor)?;
            let unit_records = bucket.state_names(UNIT_RECORDS, monitor)?;
            println!(
                "bucket holds {} objects and {} unit records",
                held.len(),
                unit_records.len()
            );
            Ok(Listed {
                held: MemoryHoldings::listed(held),
                unit_records: Some(unit_records),
            })
        }
        _ => Ok(Listed {
            held: MemoryHoldings::default(),
            unit_records: None,
        }),
    }
}

/// What a run starts from: the state that the last run of the region left, and the previous
/// publications of its views. When the state does not name the revisions those publications have,
/// the run changes no publication and keeps only what the state knows of bundles; the version
/// histories of the views go on from those publications.
struct Baseline {
    state: RegionState,
    /// The root of the state, whose chunks a write keeps.
    root: Option<Root>,
    publications: Publications,
}

/// The publications of the targets of a run that it patches, by target. A target without one is
/// published anew.
#[derive(Default)]
struct Publications(BTreeMap<String, Previous>);

impl Publications {
    /// The publication of `target`, or none.
    fn of(&self, target: &str) -> &Previous {
        static NONE: Previous = Previous {
            index: None,
            paths: None,
            tree: None,
            last: None,
        };
        self.0.get(target).unwrap_or(&NONE)
    }
}

impl Baseline {
    /// The baseline of the state `key` and of `targets`, from the state in `files` and the
    /// publications of `site`, whose documents are read from `store`, with up to `requests` reads
    /// at once.
    fn read(
        files: &dyn StateFiles,
        site: &Site<'_>,
        key: &str,
        targets: &[String],
        store: &dyn Store,
        requests: usize,
    ) -> Result<Self, String> {
        let read = RegionState::read(files, key, requests);
        let read = read.map_err(|e| format!("the state of {key}: {e}"))?;
        let (state, root) = read.map_or_else(Default::default, |(state, root)| (state, Some(root)));
        let pointers = targets
            .iter()
            .map(|target| Ok((target, site.current(target)?)))
            .collect::<Result<Vec<_>, String>>()?;
        let agrees = pointers.iter().all(|(target, pointer)| {
            pointer.as_ref().map(|pointer| &pointer.revision) == state.published.get(*target)
        });
        let state = if agrees {
            state
        } else {
            println!("{key}: the state names other publications, so the views are published anew");
            RegionState {
                bundles: state.bundles,
                ..RegionState::default()
            }
        };
        // A view published anew changes no publication, but its version history goes on.
        let previous_of = if agrees {
            Previous::read
        } else {
            Previous::after
        };
        let mut publications = BTreeMap::new();
        for (target, pointer) in pointers {
            if let Some(pointer) = pointer {
                let previous = previous_of(store, &pointer).map_err(|e| e.to_string())?;
                publications.insert(target.clone(), previous);
            }
        }
        Ok(Self {
            state,
            root,
            publications: Publications(publications),
        })
    }
}

fn canonical_target(region: &str) -> String {
    format!("{region}/canonical")
}

fn storage_target(region: &str) -> String {
    format!("{region}/storage")
}

fn client_target(region: &str) -> String {
    format!("{region}/client")
}

/// The publications of a version and the assets it built from tables.
struct Views {
    /// The publication of each target, in the order of [`Options::targets`], so the one in the
    /// canonical layout comes first.
    published: Vec<(String, Publication)>,
    tables: Vec<Built>,
}

impl Views {
    /// The publication in the canonical layout.
    fn canonical(&self) -> &Publication {
        &self.published[0].1
    }

    /// The objects that the publications refer to.
    fn objects(&self) -> impl Iterator<Item = Object> + '_ {
        let publications = self.published.iter().map(|(_, publication)| publication);
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
    /// When the run publishes its views, which their version histories record.
    published_at: SystemTime,
}

/// What a commit wrote.
struct Committed {
    /// The state of the region after the run.
    state: RegionState,
    /// How many records.
    records: usize,
    /// The chunks of the state.
    chunks: Written,
}

impl Context<'_> {
    /// How many reads and writes of the site run at once.
    fn requests(&self) -> usize {
        self.site.requests(self.options.threads)
    }

    /// The publications of the version `name` that `advance` reaches, patched from the previous
    /// `publications`, with the converters of `opened`: the view of a client package, or the
    /// canonical and the storage view of the assets of a region with those built from tables.
    fn publish(
        &self,
        name: &str,
        publications: &Publications,
        advance: &Advance<'_>,
        opened: &Opened,
    ) -> Result<Views, String> {
        let region = self.options.region.as_str();
        let owners = advance.owners()?;
        if self.options.client {
            let target = client_target(region);
            let previous = publications.of(&target);
            let client = self.canonical("client", name, previous, advance, &owners, &[])?;
            return Ok(Views {
                published: vec![(target, client)],
                tables: Vec::new(),
            });
        }
        let target = canonical_target(region);
        let previous = publications.of(&target);
        let lookup = previous.lookup(self.objects);
        let tables = self.build_tables(opened, advance.state, &owners, &lookup)?;
        let canonical = self.canonical("canonical", name, previous, advance, &owners, &tables)?;

        let (units, assets) = advance.storage_assets(&lookup, &owners)?;
        let storage = storage_target(region);
        let previous = publications.of(&storage);
        let change = crate::storage::change(
            &units,
            &assets,
            &previous.lookup(self.objects),
            self.objects,
            self.backed,
            self.requests(),
            self.monitor,
        )?;
        let storage_publication = self.apply("storage", name, Layout::Paths, previous, change)?;
        Ok(Views {
            published: vec![(target, canonical), (storage, storage_publication)],
            tables,
        })
    }

    /// The publication of `view` in the canonical layout as the version `name`: the assets that
    /// `advance` reaches, owned as `owners` say, with `tables`, patched from `previous`.
    fn canonical(
        &self,
        view: &str,
        name: &str,
        previous: &Previous,
        advance: &Advance<'_>,
        owners: &BTreeMap<String, Key>,
        tables: &[Built],
    ) -> Result<Publication, String> {
        let lookup = previous.lookup(self.objects);
        let (mut old, mut new) = advance.canonical_bindings(&lookup, owners)?;
        old.extend(table_bindings(&advance.state.tables)?);
        new.extend(tables.iter().map(|built| Binding {
            name: built.output.name.clone(),
            document: built.output.document.clone(),
        }));
        let change = {
            let _span = self.monitor.span("publish", "canonical change", "");
            views::canonical_change(old, new, self.objects, self.requests())?
        };
        self.apply(view, name, self.site.layout(), previous, change)
    }

    /// The assets that the converters of tables build, or keep from those the state lists unless
    /// `--rebuild-tables` is given, with the assets that the claims of `state` own as those of the
    /// version: their documents are those of `owners`, or else those of the previous publication
    /// `canonical`.
    fn build_tables(
        &self,
        opened: &Opened,
        state: &RegionState,
        owners: &BTreeMap<String, Key>,
        canonical: &Lookup<'_>,
    ) -> Result<Vec<Built>, String> {
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
            threads: self.requests(),
            previous: if self.options.rebuild_tables {
                &[]
            } else {
                &state.tables
            },
        };
        let forms: Vec<&'static str> = opened.converters.iter().map(|c| c.name()).collect();
        build_tables(&stage, &state.claims, &forms, &document)
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
        let edition = Edition {
            view,
            version: name,
            layout,
            published_at: Some(self.published_at),
        };
        let target = format!("{}/{view}", self.options.region);
        let versions = self.site.versions(&target);
        let applied = patch::apply(
            self.objects,
            self.store,
            &edition,
            previous,
            change,
            &versions,
        );
        applied.map_err(|e| e.to_string())
    }

    /// Commits the version `identity` that `views` publish, in this order: puts at the site every
    /// object of the views and of the records of `results`, then writes the records, the state
    /// that `advanced` becomes with the views (the chunks that `root` does not name, and the
    /// root), the pointers of the views and `recorded`, as the last version.
    fn commit(
        &self,
        results: &[UnitResult],
        advanced: RegionState,
        root: Option<&Root>,
        views: &Views,
        recorded: &RecordedVersion,
    ) -> Result<Committed, String> {
        let requests = self.requests();
        let key = self.options.state_key();
        let records = self.backed.records;
        let results: Vec<&UnitResult> = results.iter().collect();
        {
            let _span = self.monitor.span("commit", "objects", "");
            let mut referred = record_objects(records, &results, self.objects, requests)?;
            // The publications come last, so the media types they give are the ones stored.
            referred.extend(views.objects());
            self.site.store(self.objects, referred)?;
        }
        let written = {
            let _span = self.monitor.span("commit", "records", "");
            records.write(&results, requests)
        };
        let written = written.map_err(|e| format!("records: {e}"))?;

        let revisions = views
            .published
            .iter()
            .map(|(target, publication)| (target.clone(), publication.pointer.revision.clone()));
        let state = RegionState {
            version: recorded.identity.name(),
            published: revisions.collect(),
            tables: views.tables.iter().map(TableOutput::of).collect(),
            ..advanced
        };
        let chunks = {
            let _span = self.monitor.span("commit", "state", "");
            state.write(self.files, &key, root, requests)
        };
        let chunks = chunks.map_err(|e| format!("the state of {key}: {e}"))?;
        for (target, publication) in &views.published {
            self.site.make_current(target, &publication.pointer)?;
        }
        site::set_last_version(self.files, &key, recorded)?;
        Ok(Committed {
            state,
            records: written,
            chunks,
        })
    }
}

/// Prints what the run of `region` published and committed.
fn print_summary(region: &str, committed: &Committed, views: &Views, outcome: &reuse::Outcome) {
    let state = &committed.state;
    let owned = state.claims.iter().filter_map(Claims::owner);
    let errors = owned.clone().filter(|owner| owner.failed).count();
    let unnamed = state.claims.iter().filter(|c| c.owner().is_none()).count();
    let evaluation = &outcome.streamed.evaluation;
    println!(
        "{region} {} published revision {} assets {} errors {errors} unnamed {unnamed} \
         evaluated {} kept {} converted {} reused {} records {} state chunks {} bytes {}",
        state.version,
        views.canonical().pointer.revision,
        owned.count() + views.tables.len(),
        outcome.results.len(),
        outcome.kept.len(),
        evaluation.converted,
        evaluation.reused,
        committed.records,
        committed.chunks.chunks,
        committed.chunks.bytes,
    );
}

/// The objects that the records of `units` name, read from `objects` with up to `requests` reads
/// at once: the asset documents and their members.
fn record_objects(
    records: &Records<'_>,
    units: &[&UnitResult],
    objects: &Objects<'_>,
    requests: usize,
) -> Result<Vec<Object>, String> {
    let documents = records
        .documents(units)
        .map_err(|e| format!("records: {e}"))?;
    let named = parallel(documents.len(), requests, |i| {
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
            content: declared_content(&entry.declared),
            dependencies: entry.dependencies,
        })
        .collect();
        let selected = select(entries.clone(), &["a/".to_owned()]);
        assert_eq!(names(&selected), ["a/one", "c/shared", "d/deep"]);
        assert_eq!(select(entries, &[]).len(), 4);
    }

    fn options(args: &[&str]) -> Result<Options, lexopt::Error> {
        let args = ["jp", "out", "--config", "regions.json"].iter().chain(args);
        parse(args.map(OsString::from))
    }

    #[test]
    fn a_client_run_keeps_its_own_state_and_publishes_one_target() {
        let assets = options(&[]).unwrap();
        assert_eq!(assets.state_key(), "jp");
        assert_eq!(assets.targets(), ["jp/canonical", "jp/storage"]);
        let client = options(&["--client"]).unwrap();
        assert_eq!(client.state_key(), "jp-client");
        assert_eq!(client.targets(), ["jp/client"]);
        assert!(options(&["--client", "--check"]).is_ok());
        assert!(options(&["--client", "--rebuild-tables"]).is_err());
    }
}
