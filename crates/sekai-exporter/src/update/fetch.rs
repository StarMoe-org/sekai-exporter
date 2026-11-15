//! Where the bundles of a run come from: the input store, earlier runs or the source.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use sekai_catalog::db::{BundleRow, Catalog};
use sekai_catalog::store::{ObjectWriter, parse_hex};
use sekai_catalog::{Key, LocalStore};
use sekai_format::identity::VersionIdentity;
use sekai_game::cdn::{BundleEntry, Source};
use sekai_game::obfuscation::Deobfuscating;
use sekai_game::sink::Sink;
use sekai_pipeline::metrics::Recorder;
use sekai_pipeline::state::BundleState;
use sekai_pipeline::stream::{self, Content, Entry, Inputs};
use sekai_pipeline::{BundleInput, parallel};
use sekai_unity::FileSource;

use crate::monitor::Monitor;

/// The extension of bundles in the input store.
pub(super) const INPUT_EXTENSION: &str = "bundle";
/// How many fetched bundles are recorded together.
const RECORD_BATCH: usize = 1024;

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

/// The bundles of a run in the input store, as they arrive. Opening one waits until it has been
/// fetched, so evaluation starts with the first bundles while later ones are still on their way.
pub(super) struct Arriving<'s> {
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
    pub(super) fn new(inputs: &'s LocalStore) -> Self {
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
    pub(super) fn ending(&self) -> Ending<'_, 's> {
        Ending(self)
    }

    fn wait_until(&self, done: impl Fn(&Arrivals) -> bool) -> MutexGuard<'_, Arrivals> {
        self.changed
            .wait_while(self.lock(), |arrivals| !done(arrivals))
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Ends the fetching of an [`Arriving`] when dropped.
pub(super) struct Ending<'a, 's>(&'a Arriving<'s>);

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
pub(super) fn open_input(inputs: &LocalStore, name: &str, key: &Key) -> io::Result<BundleInput> {
    let source = FileSource::open(inputs.path(key)).map_err(io::Error::other)?;
    Ok(BundleInput {
        name: name.to_owned(),
        source: Arc::new(source),
        sha256: key.sha256,
    })
}

/// The bundles that another output directory holds.
pub(super) struct EarlierRun {
    catalog: Catalog,
    inputs: LocalStore,
}

impl EarlierRun {
    pub(super) fn open(output: &Path) -> Result<Self, String> {
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
pub(super) struct Fetcher<'r> {
    pub(super) source: &'r dyn Source,
    pub(super) identity: &'r VersionIdentity,
    pub(super) inputs: &'r LocalStore,
    pub(super) catalog: &'r Catalog,
    /// The bundles of the last version of the region, in name order.
    pub(super) bundles: &'r [BundleState],
    pub(super) earlier: &'r [EarlierRun],
    pub(super) region: &'r str,
    pub(super) monitor: &'r Monitor,
    /// The key of each bundle this run has fetched, by name, so that none is fetched twice.
    pub(super) fetched: Mutex<HashMap<String, Key>>,
}

/// A bundle in the input store, and whether this run placed it there.
type Fetched = (Key, bool);

impl Fetcher<'_> {
    /// Fetches `entries` into the input store in their order, `concurrency` at a time, and tells
    /// `arriving` of each as it arrives. Returns how many bundles could not be fetched.
    pub(super) fn fetch_all(
        &self,
        entries: &[BundleEntry],
        concurrency: usize,
        arriving: &Arriving<'_>,
    ) -> Result<usize, String> {
        let arrived = |name: &str, fetched: &Result<Fetched, String>| {
            arriving.arrive(name, fetched.clone().map(|(key, _)| key));
        };
        let results = self.fetch_each(entries, concurrency, &arrived)?;
        let mut new = 0;
        let mut failed = 0;
        for (entry, result) in entries.iter().zip(results) {
            match result {
                Ok((_, placed)) => new += usize::from(placed),
                Err(e) => {
                    failed += 1;
                    eprintln!("{}: {e}", entry.name);
                }
            }
        }
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
    pub(super) fn fetch_keys(
        &self,
        entries: &[BundleEntry],
        concurrency: usize,
    ) -> Result<HashMap<String, Key>, String> {
        let results = self.fetch_each(entries, concurrency, &|_, _| {})?;
        let fetched = entries.iter().zip(results);
        let keys = fetched.filter_map(|(entry, result)| Some((entry.name.clone(), result.ok()?.0)));
        Ok(keys.collect())
    }

    /// Fetches `entries` into the input store in their order, `concurrency` at a time, and tells
    /// `arrived` of each as it arrives. The bundles this run places are recorded in the catalog in
    /// batches. Returns what became of each entry; only a failure of the catalog fails the call.
    fn fetch_each(
        &self,
        entries: &[BundleEntry],
        concurrency: usize,
        arrived: &(dyn Fn(&str, &Result<Fetched, String>) + Sync),
    ) -> Result<Vec<Result<Fetched, String>>, String> {
        let rows = Mutex::new(Vec::new());
        let results = parallel(entries.len(), concurrency, |i| {
            let entry = &entries[i];
            let fetched = self.fetch(entry);
            arrived(&entry.name, &fetched);
            if let Ok((key, true)) = &fetched {
                self.place(&rows, entry, key)?;
            }
            Ok(fetched)
        });
        self.record(&rows.into_inner().unwrap_or_else(PoisonError::into_inner))?;
        results.into_iter().collect()
    }

    /// The key of `entry` in the input store, and whether this call placed it there: fetched
    /// earlier in this run, found in the catalog, linked from an earlier run, or downloaded.
    pub(super) fn fetch(&self, entry: &BundleEntry) -> Result<Fetched, String> {
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
    fn locate(&self, entry: &BundleEntry) -> Result<Fetched, String> {
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
pub(super) fn in_opening_order(all: &[BundleEntry], manifest: &[Entry]) -> Vec<BundleEntry> {
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
/// that hash, unless it is empty.
pub(super) fn declared_content(declared: &str) -> Option<Content> {
    (!declared.is_empty()).then(|| Content::Declared(declared.to_owned()))
}

/// Identifies the content of each entry of `manifest` that declares no hash by the bytes of its
/// bundle, which are fetched for that from the entries of `all`. An entry whose bundle cannot be
/// fetched stays unidentified.
pub(super) fn identify(
    fetcher: &Fetcher<'_>,
    all: &[BundleEntry],
    manifest: &mut [Entry],
    concurrency: usize,
) -> Result<(), String> {
    let undeclared: HashSet<&str> = manifest
        .iter()
        .filter(|entry| entry.content.is_none())
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
            entry.content = Some(Content::Bytes(key.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use sekai_game::cdn;
    use sekai_pipeline::NoBundleIndexes;
    use sekai_unity::BoundedBlockCache;

    use super::*;
    use crate::export::BLOCK_CACHE_BYTES;

    fn entry(name: &str, dependencies: &[&str]) -> BundleEntry {
        BundleEntry {
            name: name.to_owned(),
            declared: "h".to_owned(),
            download_path: None,
            size: 1,
            dependencies: dependencies.iter().map(|d| (*d).to_owned()).collect(),
        }
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
            assert_eq!(manifest[0].content, declared_content("h"));
            assert_eq!(manifest[1].content, Some(Content::Bytes(key.clone())));
            assert_eq!(manifest[2].content, None);
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
}
