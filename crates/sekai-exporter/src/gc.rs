//! `sekai-exporter gc`: finds the files of a site that nothing it keeps refers to, and deletes
//! them with `--delete`.
//!
//! What is kept: every pointer under `published/`, every object a pointer refers to, the state of
//! each region with the documents of the assets it built from tables, and the records whose
//! objects are all kept. Files written within the grace period are kept too, with the objects the
//! records among them name. Everything else under `store/` and `state/` is garbage.

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use sekai_catalog::{Key, LocalStore, MemoryHoldings, Objects};
use sekai_format::publication::Pointer;
use sekai_pipeline::parallel;
use sekai_pipeline::records::{DERIVATION_RECORDS, UNIT_RECORDS, record_objects};
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
        written_before: SystemTime::now() - options.grace,
        requests: site.requests(options.threads),
    };
    let garbage = collector.garbage()?;
    for kind in &garbage {
        kind.report();
    }
    if options.delete {
        for kind in &garbage {
            collector.delete(kind)?;
        }
        println!("deleted");
    } else {
        println!("nothing deleted; --delete deletes the files above");
    }
    Ok(true)
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

/// A record, with the objects it names.
struct Recorded {
    file: Listed,
    objects: Vec<Key>,
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
    fn garbage(&self) -> Result<Vec<Kind>, String> {
        let mut live = self.published()?;
        let chunks = self.states(&mut live)?;
        let records = self.records(&mut live)?;
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
        Ok(records.into_iter().chain(others).collect())
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

    /// The unit and the derivation records. A record written before the grace period is garbage
    /// when it names an object outside `live`; the objects that the records written within it name
    /// are added to `live` first.
    fn records(&self, live: &mut HashSet<Key>) -> Result<[Kind; 2], String> {
        let units = self.read_records(UNIT_RECORDS)?;
        let derivations = self.read_records(DERIVATION_RECORDS)?;
        for record in units.iter().chain(&derivations) {
            if !self.old(&record.file) {
                live.extend(record.objects.iter().cloned());
            }
        }
        let kind = |name, records: Vec<Recorded>| {
            let judged = records.into_iter().map(|record| {
                let named_dead = record.objects.iter().any(|object| !live.contains(object));
                let garbage = self.old(&record.file) && named_dead;
                (record.file, garbage)
            });
            Kind::of(name, judged)
        };
        Ok([
            kind(UNIT_RECORDS, units),
            kind(DERIVATION_RECORDS, derivations),
        ])
    }

    /// The records in the state directory `directory`, with the objects each names.
    fn read_records(&self, directory: &str) -> Result<Vec<Recorded>, String> {
        let files = self.site.list_sharded(&format!("state/{directory}"))?;
        let named = parallel(files.len(), self.requests, |i| self.named(&files[i]));
        let records = files.into_iter().zip(named);
        records
            .map(|(file, objects)| {
                Ok(Recorded {
                    file,
                    objects: objects?,
                })
            })
            .collect()
    }

    /// The objects that the record `file` names; none when it is gone.
    fn named(&self, file: &Listed) -> Result<Vec<Key>, String> {
        let name = state_name(file);
        let failed = |e: std::io::Error| format!("{name}: {e}");
        let Some(bytes) = self.site.files().read(name).map_err(failed)? else {
            return Ok(Vec::new());
        };
        record_objects(name, &bytes).map_err(failed)
    }

    /// Deletes the garbage of `kind`.
    fn delete(&self, kind: &Kind) -> Result<(), String> {
        let garbage = &kind.garbage;
        let deleted = parallel(garbage.len(), self.requests, |i| {
            self.site.delete(&garbage[i].name)
        });
        deleted.into_iter().collect()
    }
}
