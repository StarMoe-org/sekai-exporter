//! `sekai-exporter gc`: finds the files of a site that nothing it keeps refers to, and deletes
//! them with `--delete`.
//!
//! What is kept: every pointer under `published/`, every object a pointer refers to, the state of
//! each region with the documents of the assets it built from tables, and the records whose
//! objects are all kept. Files written within the grace period are kept too, with the objects the
//! records among them name. Everything else under `store/` and `state/` is garbage. In the output
//! directory, the bundles of the input store that no region fetched last, and the partial writes
//! that interrupted runs left in `incoming/`, are garbage too.
//!
//! Records lie in packs that are never changed, as the pages of a flash drive are never written
//! twice. A pack that holds a record no longer kept, or that holds few records, is rewritten: the
//! records kept of every such pack are written to new packs, and then the packs they came from
//! are deleted. Records kept one file each are moved into packs the same way.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sekai_catalog::db::Catalog;
use sekai_catalog::{Key, LocalStore, MemoryHoldings, Objects};
use sekai_format::publication::Pointer;
use sekai_pipeline::parallel;
use sekai_pipeline::records::{
    DERIVATION_RECORDS, Entry, PACK_BYTES, Pack, RECORD_PACKS, UNIT_RECORDS,
};
use sekai_pipeline::state::{CHUNKS, ROOTS, RegionState, root_region};
use sekai_publish::history::Versions;
use sekai_publish::{Listed, referred};

use crate::bucket::Bucket;
use crate::update::document_objects;
use crate::update::site::Site;

/// The grace period unless `--grace` gives one.
const DAY: Duration = Duration::from_secs(86_400);

struct Options {
    output: PathBuf,
    config: Option<PathBuf>,
    secrets: Option<PathBuf>,
    grace: Duration,
    threads: usize,
    delete: bool,
}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Options, lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let mut output = None;
    let mut config = None;
    let mut secrets = None;
    let mut grace = DAY;
    let mut threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut delete = false;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("config") => config = Some(PathBuf::from(parser.value()?)),
            Long("secrets") => secrets = Some(PathBuf::from(parser.value()?)),
            Long("grace") => grace = parse_duration(&parser.value()?.string()?)?,
            Long("threads") => threads = parser.value()?.parse()?,
            Long("delete") => delete = true,
            Value(v) if output.is_none() => output = Some(PathBuf::from(v)),
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(Options {
        output: output.ok_or("expected <output>")?,
        config,
        secrets,
        grace,
        threads: threads.max(1),
        delete,
    })
}

/// The duration `text`: a whole number of seconds (`s`), minutes (`m`), hours (`h`) or days
/// (`d`), such as `12h`.
fn parse_duration(text: &str) -> Result<Duration, String> {
    let invalid = || format!("{text:?} is not a duration such as 30m, 12h or 7d");
    let unit = text.chars().last().ok_or_else(invalid)?;
    let seconds = match unit {
        's' => 1,
        'm' => 60,
        'h' => 3600,
        'd' => 86_400,
        _ => return Err(invalid()),
    };
    let count: u64 = text[..text.len() - 1].parse().map_err(|_| invalid())?;
    count
        .checked_mul(seconds)
        .map(Duration::from_secs)
        .ok_or_else(invalid)
}

pub(crate) fn run(args: impl Iterator<Item = OsString>) -> Result<bool, lexopt::Error> {
    let options = parse(args)?;
    let bucket = match &options.config {
        Some(path) => {
            let config =
                std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
            Bucket::open(&config, options.secrets.as_deref())?
        }
        None => None,
    };
    let site = bucket
        .as_ref()
        .map_or_else(|| Site::local(&options.output), Site::Bucket);
    let store = LocalStore::open(options.output.join("store")).map_err(|e| e.to_string())?;
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: site.remote(),
    };
    let collector = Collector {
        site: &site,
        objects: &objects,
        written_before: SystemTime::now()
            .checked_sub(options.grace)
            .unwrap_or(UNIX_EPOCH),
        requests: site.requests(options.threads),
    };
    let Garbage { packs, kinds } = collector.garbage()?;
    let output = Site::local(&options.output);
    let leftovers = collector.leftovers(&output, fetched(&options.output, &store).as_ref())?;
    for kind in kinds.iter().chain(&leftovers) {
        kind.report();
    }
    let bytes: usize = packs.iter().map(|pack| pack.bytes.len()).sum();
    println!("{RECORD_PACKS}: new {} files, {bytes} bytes", packs.len());
    if options.delete {
        // The records kept are in their new packs before the packs they came from go.
        collector.write(&site, &packs)?;
        for kind in &kinds {
            collector.delete(&site, kind)?;
        }
        for kind in &leftovers {
            collector.delete(&output, kind)?;
        }
        println!("deleted");
    } else {
        println!("nothing deleted; --delete deletes the files above");
    }
    Ok(true)
}

/// The SHA-256 of the bundle each region fetched last, as the catalog of the output directory
/// `output` records them. `None` when it records no fetch or cannot be read, which leaves the
/// input store alone.
fn fetched(output: &Path, store: &LocalStore) -> Option<HashSet<String>> {
    let database = output.join("catalog.redb");
    if !database.is_file() {
        return None;
    }
    let rows = Catalog::open_or_rebuild(&database, store).and_then(|catalog| catalog.bundles());
    match rows {
        Ok(rows) if !rows.is_empty() => Some(rows.into_iter().map(|row| row.sha256).collect()),
        Ok(_) => None,
        Err(e) => {
            eprintln!("inputs: the catalog cannot be read, so no bundle is garbage: {e}");
            None
        }
    }
}

/// The SHA-256 of the bundle that the file `file` of the input store holds, from its name.
fn input_hash(file: &Listed) -> Option<&str> {
    let (hash, _) = file.name.rsplit('/').next()?.split_once('.')?;
    Some(hash)
}

/// Finds the garbage of a site.
struct Collector<'a> {
    site: &'a Site<'a>,
    /// Where the documents of the publications are read from: the local store, and else the site.
    objects: &'a Objects<'a>,
    /// Files written at or after this time are within the grace period.
    written_before: SystemTime,
    /// How many reads and deletions run at once.
    requests: usize,
}

/// A file of records: a pack, or a record kept in a file of its own.
struct RecordFile {
    file: Listed,
    /// Its records, each with the objects it names.
    records: Vec<(Entry, Vec<Key>)>,
}

impl RecordFile {
    /// The bytes of its records, which [`PACK_BYTES`] counts.
    fn size(&self) -> usize {
        self.records.iter().map(|(entry, _)| entry.size()).sum()
    }

    /// The objects its records name.
    fn named(&self) -> impl Iterator<Item = &Key> {
        self.records.iter().flat_map(|(_, objects)| objects)
    }

    /// Its records that name only objects in `live`, which are kept.
    fn kept<'r>(&'r self, live: &'r HashSet<Key>) -> impl Iterator<Item = &'r Entry> {
        let kept = self
            .records
            .iter()
            .filter(|(_, objects)| named_live(objects, live));
        kept.map(|(entry, _)| entry)
    }

    /// Whether it holds a record that names an object outside `live`, which is not kept.
    fn drops(&self, live: &HashSet<Key>) -> bool {
        let kept = |(_, objects): &(Entry, Vec<Key>)| named_live(objects, live);
        !self.records.iter().all(kept)
    }
}

/// Whether every one of `objects` is in `live`.
fn named_live(objects: &[Key], live: &HashSet<Key>) -> bool {
    objects.iter().all(|object| live.contains(object))
}

/// The garbage of a site, and the record packs a collection writes before it deletes any.
struct Garbage {
    packs: Vec<Pack>,
    /// The files of each kind, in the order they are deleted.
    kinds: Vec<Kind>,
}

/// The files of one kind that a site keeps, and those of them that are garbage.
struct Kind {
    name: &'static str,
    files: usize,
    bytes: u64,
    garbage: Vec<Listed>,
}

impl Kind {
    /// The files named `name` of `judged`, each with whether it is garbage.
    fn of(name: &'static str, judged: impl IntoIterator<Item = (Listed, bool)>) -> Self {
        let mut kind = Self {
            name,
            files: 0,
            bytes: 0,
            garbage: Vec::new(),
        };
        for (file, garbage) in judged {
            kind.files += 1;
            kind.bytes += file.size;
            if garbage {
                kind.garbage.push(file);
            }
        }
        kind
    }

    /// Prints how many of the files are garbage, and for objects, of which extensions.
    fn report(&self) {
        let bytes: u64 = self.garbage.iter().map(|file| file.size).sum();
        println!(
            "{}: {} files, {} bytes; garbage {} files, {bytes} bytes",
            self.name,
            self.files,
            self.bytes,
            self.garbage.len()
        );
        let mut extensions: BTreeMap<String, (usize, u64)> = BTreeMap::new();
        for file in &self.garbage {
            if let Some(key) = object_key(file) {
                let (count, bytes) = extensions.entry(key.extension).or_default();
                *count += 1;
                *bytes += file.size;
            }
        }
        for (extension, (count, bytes)) in extensions {
            println!("  .{extension}: {count} files, {bytes} bytes");
        }
    }
}

/// The object of the store that the file `file` is, when it is one.
fn object_key(file: &Listed) -> Option<Key> {
    file.name
        .strip_prefix("store/")?
        .rsplit('/')
        .next()?
        .parse()
        .ok()
}

/// The name of the state file `file` among the state files.
fn state_name(file: &Listed) -> &str {
    file.name.strip_prefix("state/").unwrap_or(&file.name)
}

impl Collector<'_> {
    /// The garbage of the site, in the order it is deleted: the records, which name objects, before
    /// the state chunks and the objects of the store.
    fn garbage(&self) -> Result<Garbage, String> {
        let mut live = self.published()?;
        let chunks = self.states(&mut live)?;
        let (packs, records) = self.records(&mut live)?;
        let chunk_files = self.site.list_sharded(&format!("state/{CHUNKS}"))?;
        let state_chunks = chunk_files.into_iter().map(|file| {
            let garbage = self.old(&file) && !chunks.contains(state_name(&file));
            (file, garbage)
        });
        let objects = self.site.list_sharded("store")?.into_iter().map(|file| {
            let unreferred = object_key(&file).is_some_and(|key| !live.contains(&key));
            let garbage = self.old(&file) && unreferred;
            (file, garbage)
        });
        let others = [Kind::of(CHUNKS, state_chunks), Kind::of("store", objects)];
        let kinds = records.into_iter().chain(others).collect();
        Ok(Garbage { packs, kinds })
    }

    /// The files that runs leave in the output directory `output`: the bundles of the input store
    /// that `fetched` does not hold, with no bundle garbage without it, and the partial writes of
    /// interrupted runs.
    fn leftovers(
        &self,
        output: &Site<'_>,
        fetched: Option<&HashSet<String>>,
    ) -> Result<[Kind; 2], String> {
        let inputs = output.list_sharded("inputs")?.into_iter().map(|file| {
            let unfetched = fetched
                .zip(input_hash(&file))
                .is_some_and(|(fetched, hash)| !fetched.contains(hash));
            let garbage = self.old(&file) && unfetched;
            (file, garbage)
        });
        let mut partial = output.list("inputs/incoming")?;
        partial.extend(output.list("store/incoming")?);
        let partial = partial.into_iter().map(|file| {
            let garbage = self.old(&file);
            (file, garbage)
        });
        Ok([Kind::of("inputs", inputs), Kind::of("incoming", partial)])
    }

    /// Whether `file` was written before the grace period.
    fn old(&self, file: &Listed) -> bool {
        file.modified < self.written_before
    }

    /// The objects that the pointers of the site refer to.
    fn published(&self) -> Result<HashSet<Key>, String> {
        let mut pointers: Vec<(String, Pointer)> = Vec::new();
        for file in self.site.list("published")? {
            if let Some(pointer) = self.pointer(&file.name)?
                && pointers.iter().all(|(_, known)| *known != pointer)
            {
                pointers.push((file.name, pointer));
            }
        }
        let mut live = HashSet::new();
        for (name, pointer) in pointers {
            let referred = referred(self.objects, &pointer, self.requests);
            live.extend(referred.map_err(|e| format!("{name}: {e}"))?);
        }
        Ok(live)
    }

    /// The pointer that the file `name` holds, when it is the current pointer of a view or the
    /// pointer of one of its versions.
    fn pointer(&self, name: &str) -> Result<Option<Pointer>, String> {
        let Some(path) = name.strip_prefix("published/") else {
            return Ok(None);
        };
        if let Some(target) = path.strip_suffix("/current.json") {
            return self.site.current(target);
        }
        let Some((target, file)) = path.rsplit_once("/versions/") else {
            return Ok(None);
        };
        let Some(version) = file.strip_suffix(".json") else {
            return Ok(None);
        };
        let pointer = self.site.versions(target).pointer(version);
        pointer.map_err(|e| format!("{name}: {e}"))
    }

    /// The state chunks that the root of each region names. The documents of the assets each
    /// built from tables, with their members, are added to `live`.
    fn states(&self, live: &mut HashSet<Key>) -> Result<HashSet<String>, String> {
        let mut chunks = HashSet::new();
        for file in self.site.list(&format!("state/{ROOTS}"))? {
            let Some(region) = root_region(state_name(&file)) else {
                continue;
            };
            let read = RegionState::read(self.site.files(), region, self.requests);
            let failed = |e: String| format!("the state of {region}: {e}");
            let Some((state, root)) = read.map_err(|e| failed(e.to_string()))? else {
                continue;
            };
            chunks.extend(root.chunk_files());
            for table in &state.tables {
                let document = table.document.parse::<Key>();
                let document = document.map_err(|e| failed(e.to_string()))?;
                let objects = document_objects(self.objects, &document);
                let objects = objects.map_err(|e| failed(format!("{document}: {e}")))?;
                live.extend(objects.into_iter().map(|object| object.key));
            }
        }
        Ok(chunks)
    }

    /// The record packs that hold the records kept of the record files rewritten, and the record
    /// files of each kind with those to delete. The objects that the records written within the
    /// grace period name are added to `live` first; a record written before it is kept when every
    /// object it names is live.
    ///
    /// A pack written before the grace period is rewritten when it holds a record that is not
    /// kept or when it is small, and so is every file of a single record written before it. A pack
    /// that the new packs give again as it is stays, and is not written again.
    fn records(&self, live: &mut HashSet<Key>) -> Result<(Vec<Pack>, [Kind; 3]), String> {
        let packs = self.read_packs()?;
        let units = self.read_singles(UNIT_RECORDS)?;
        let derivations = self.read_singles(DERIVATION_RECORDS)?;
        let singles = || units.iter().chain(&derivations);
        for file in packs.iter().chain(singles()) {
            if !self.old(&file.file) {
                live.extend(file.named().cloned());
            }
        }
        let live = &*live;
        let rewritten = |pack: &RecordFile| {
            self.old(&pack.file) && (pack.drops(live) || pack.size() < PACK_BYTES / 4)
        };
        let moved = packs.iter().filter(|pack| rewritten(pack));
        let moved = moved.chain(singles().filter(|file| self.old(&file.file)));
        let kept = moved.flat_map(|file| file.kept(live)).cloned().collect();
        let new = Pack::of(kept).map_err(|e| format!("{RECORD_PACKS}: {e}"))?;
        let names: HashSet<&str> = new.iter().map(|pack| pack.name.as_str()).collect();
        let judged = packs.iter().map(|pack| {
            let garbage = rewritten(pack) && !names.contains(state_name(&pack.file));
            (pack.file.clone(), garbage)
        });
        let kinds = [
            Kind::of(RECORD_PACKS, judged),
            Kind::of(UNIT_RECORDS, self.moved(&units)),
            Kind::of(DERIVATION_RECORDS, self.moved(&derivations)),
        ];
        let existing: HashSet<&str> = packs.iter().map(|pack| state_name(&pack.file)).collect();
        let written = new
            .into_iter()
            .filter(|pack| !existing.contains(pack.name.as_str()));
        Ok((written.collect(), kinds))
    }

    /// The files of single records `files`, each with whether it is moved into a pack: when it was
    /// written before the grace period.
    fn moved<'f>(&'f self, files: &'f [RecordFile]) -> impl Iterator<Item = (Listed, bool)> + 'f {
        files
            .iter()
            .map(|file| (file.file.clone(), self.old(&file.file)))
    }

    /// The record packs of the site, with their records.
    fn read_packs(&self) -> Result<Vec<RecordFile>, String> {
        let files = self.site.list(&format!("state/{RECORD_PACKS}"))?;
        self.read_records(files, Pack::read)
    }

    /// The files of single records in the state directory `directory`, with their records; a
    /// file that holds no record that can be read holds none.
    fn read_singles(&self, directory: &str) -> Result<Vec<RecordFile>, String> {
        let files = self.site.list_sharded(&format!("state/{directory}"))?;
        self.read_records(files, |name, bytes| {
            Ok(Entry::of_file(name, bytes).into_iter().collect())
        })
    }

    /// The record files `files`, with the records that `read` finds in each and the objects each
    /// names. A file gone since the listing holds none.
    fn read_records(
        &self,
        files: Vec<Listed>,
        read: impl Fn(&str, &[u8]) -> io::Result<Vec<Entry>> + Sync,
    ) -> Result<Vec<RecordFile>, String> {
        let records = parallel(files.len(), self.requests, |i| {
            let name = state_name(&files[i]);
            let named = || -> io::Result<Vec<(Entry, Vec<Key>)>> {
                let Some(bytes) = self.site.files().read(name)? else {
                    return Ok(Vec::new());
                };
                let entries = read(name, &bytes)?;
                let objects = entries.iter().map(Entry::objects);
                let objects = objects.collect::<io::Result<Vec<_>>>()?;
                Ok(entries.into_iter().zip(objects).collect())
            };
            named().map_err(|e| format!("{name}: {e}"))
        });
        let files = files.into_iter().zip(records);
        let files = files.map(|(file, records)| {
            Ok(RecordFile {
                file,
                records: records?,
            })
        });
        files.collect()
    }

    /// Writes `packs` at `site`.
    fn write(&self, site: &Site<'_>, packs: &[Pack]) -> Result<(), String> {
        let written = parallel(packs.len(), self.requests, |i| {
            let pack = &packs[i];
            let written = site.files().write(&pack.name, &pack.bytes);
            written.map_err(|e| format!("{}: {e}", pack.name))
        });
        written.into_iter().collect()
    }

    /// Deletes the garbage of `kind`, which lies at `site`.
    fn delete(&self, site: &Site<'_>, kind: &Kind) -> Result<(), String> {
        let garbage = &kind.garbage;
        let deleted = parallel(garbage.len(), self.requests, |i| {
            site.delete(&garbage[i].name)
        });
        deleted.into_iter().collect()
    }
}
