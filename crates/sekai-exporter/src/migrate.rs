//! `sekai-exporter migrate joint-stereo`: brings the MP3 files of a site to joint stereo mode in
//! every frame, as the encoder writes a stereo sound, without encoding them again.
//!
//! Every asset document that a record names is read. Its MP3 members with frames in stereo mode
//! are rewritten as [`mp3::joint_stereo`] does, which leaves the samples they decode to as they
//! are, and the document is written again with them: the document that a conversion gives, under
//! the same fingerprint. The new files are at the site before any record names them. The records
//! that name a rewritten document then go to new record packs that name the new one, and the
//! files they lay in are deleted; the catalog of the output directory is rewritten the same way.
//! The views name the documents they named until `update --republish` publishes them anew, and
//! `gc` then finds the files that nothing names any more.

use std::collections::{BTreeSet, HashSet};
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};

use sekai_catalog::db::Catalog;
use sekai_catalog::store::{hex, parse_hex};
use sekai_catalog::{Derived, Key, LocalStore, MemoryHoldings, Objects};
use sekai_encode::mp3;
use sekai_format::AssetDocument;
use sekai_format::asset::MemberFile;
use sekai_pipeline::parallel;
use sekai_pipeline::records::{Entry, Pack, RECORD_PACKS, Replacement, Replacements};
use sekai_pipeline::reuse::Record;
use sekai_publish::{Listed, Object};

use crate::bucket;
use crate::record_files::{RecordFile, RecordFiles, state_name};
use crate::update::site::Site;

/// The documents read and rewritten before their new files go to the site.
const BATCH: usize = 4096;

/// The extension of MP3 files.
const MP3: &str = "mp3";

struct Options {
    output: PathBuf,
    config: Option<PathBuf>,
    secrets: Option<PathBuf>,
    threads: usize,
}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Options, lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let mut migration = None;
    let mut output = None;
    let mut config = None;
    let mut secrets = None;
    let mut threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    while let Some(arg) = parser.next()? {
        match arg {
            Long("config") => config = Some(PathBuf::from(parser.value()?)),
            Long("secrets") => secrets = Some(PathBuf::from(parser.value()?)),
            Long("threads") => threads = parser.value()?.parse()?,
            Value(v) if migration.is_none() => migration = Some(v.string()?),
            Value(v) if output.is_none() => output = Some(PathBuf::from(v)),
            _ => return Err(arg.unexpected()),
        }
    }
    match migration.as_deref() {
        Some("joint-stereo") => {}
        Some(other) => return Err(format!("{other} is not a migration").into()),
        None => return Err("expected a migration: joint-stereo".into()),
    }
    Ok(Options {
        output: output.ok_or("expected <output>")?,
        config,
        secrets,
        threads: threads.max(1),
    })
}

pub(crate) fn run(args: impl Iterator<Item = OsString>) -> Result<bool, lexopt::Error> {
    let options = parse(args)?;
    let bucket = bucket::configured(options.config.as_deref(), options.secrets.as_deref())?;
    let site = bucket
        .as_ref()
        .map_or_else(|| Site::local(&options.output), Site::Bucket);
    let migrated = site.while_held("migrate joint-stereo", || joint_stereo(&options, &site))?;
    Ok(migrated?)
}

/// Rewrites the stereo MP3 files of `site`, the documents that name them and the records that
/// name those. Returns whether every document could be read and rewritten.
fn joint_stereo(options: &Options, site: &Site<'_>) -> Result<bool, String> {
    let store = LocalStore::open(options.output.join("store")).map_err(|e| e.to_string())?;
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: site.remote(),
    };
    let requests = site.requests(options.threads);
    let files = RecordFiles { site, requests };
    let mut record_files = files.packs()?;
    record_files.extend(files.units()?);
    record_files.extend(files.derivations()?);
    let documents = documents(&record_files);
    println!(
        "records: {} files name {} documents",
        record_files.len(),
        documents.len()
    );
    let rewriter = Rewriter {
        site,
        objects: &objects,
        requests,
    };
    let rewritten = rewriter.rewrite(&documents)?;
    let replaced = replaced(&record_files, &rewritten.replacements)?;
    // The records that name the new documents are in their packs before the files they came
    // from go.
    files.write(&replaced.packs)?;
    let deleted = parallel(replaced.files.len(), requests, |i| {
        site.delete(&replaced.files[i].name)
    });
    deleted.into_iter().collect::<Result<(), String>>()?;
    println!(
        "{RECORD_PACKS}: {} records rewritten, {} files replaced by {} packs",
        replaced.records,
        replaced.files.len(),
        replaced.packs.len()
    );
    let catalog = rewrite_catalog(&options.output, &store, &rewritten.replacements)?;
    println!("catalog: {catalog} records rewritten");
    Ok(rewritten.failed == 0)
}

/// The asset documents that the records of `files` name, in key order.
fn documents(files: &[RecordFile]) -> Vec<Key> {
    let named = files.iter().flat_map(|file| &file.records);
    let named = named.flat_map(|(_, objects)| objects);
    let documents: BTreeSet<&Key> = named.filter(|key| key.extension == "json").collect();
    documents.into_iter().cloned().collect()
}

/// Rewrites documents and their MP3 members, and puts the new files at the site.
struct Rewriter<'a> {
    site: &'a Site<'a>,
    /// Where documents and members are read from, and new files written to.
    objects: &'a Objects<'a>,
    /// How many documents are rewritten at once.
    requests: usize,
}

/// A document rewritten: the new document, and the MP3 files rewritten for it.
struct NewDocument {
    document: Object,
    mp3s: Vec<Object>,
}

/// What rewriting the documents gave.
#[derive(Default)]
struct Rewritten {
    replacements: Replacements,
    /// How many MP3 files were rewritten, and their bytes.
    mp3s: usize,
    bytes: u64,
    /// How many documents could not be read or rewritten.
    failed: usize,
}

impl Rewritten {
    /// Counts `new`, the document that replaces `key`, and returns the files it adds.
    fn add(&mut self, key: &Key, new: NewDocument) -> impl Iterator<Item = Object> {
        self.mp3s += new.mp3s.len();
        self.bytes += new.mp3s.iter().map(|mp3| mp3.size).sum::<u64>();
        let replacement = Replacement {
            key: new.document.key.clone(),
            size: new.document.size,
        };
        self.replacements.insert(key.clone(), replacement);
        new.mp3s.into_iter().chain([new.document])
    }

    /// Prints how far the rewrite got, after `read` of `documents` documents.
    fn report(&self, read: usize, documents: usize) {
        println!(
            "documents {read}/{documents}: {} rewritten, {} MP3 files, {} bytes, {} failed",
            self.replacements.len(),
            self.mp3s,
            self.bytes,
            self.failed
        );
    }
}

impl Rewriter<'_> {
    /// Rewrites `documents` a batch at a time, and prints how far it got after each.
    fn rewrite(&self, documents: &[Key]) -> Result<Rewritten, String> {
        let mut rewritten = Rewritten::default();
        let mut read = 0;
        for keys in documents.chunks(BATCH) {
            let results = parallel(keys.len(), self.requests, |i| self.document(&keys[i]));
            let mut added = Vec::new();
            for (key, result) in keys.iter().zip(results) {
                match result {
                    Ok(Some(new)) => added.extend(rewritten.add(key, new)),
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("{key}: {e}");
                        rewritten.failed += 1;
                    }
                }
            }
            self.store(added)?;
            read += keys.len();
            rewritten.report(read, documents.len());
        }
        Ok(rewritten)
    }

    /// The document that replaces `key`, with the MP3 files rewritten for it, which the local
    /// store holds; `None` when no MP3 member of it has a frame in stereo mode.
    fn document(&self, key: &Key) -> io::Result<Option<NewDocument>> {
        let mut document = AssetDocument::from_bytes(&self.read(key)?).map_err(io::Error::other)?;
        let mut mp3s = Vec::new();
        for member in &mut document.members {
            if member.extension != MP3 {
                continue;
            }
            let mut bytes = self.read(&member_key(member)?)?;
            if mp3::joint_stereo(&mut bytes).map_err(io::Error::other)? {
                let key = self.objects.local.put(&bytes, MP3)?;
                member.sha256 = hex(&key.sha256);
                member.size = bytes.len() as u64;
                mp3s.push(Object {
                    key,
                    size: member.size,
                    media_type: member.media_type.clone(),
                });
            }
        }
        if mp3s.is_empty() {
            return Ok(None);
        }
        let bytes = document.to_bytes().map_err(io::Error::other)?;
        let document = Object {
            key: self.objects.local.put(&bytes, "json")?,
            size: bytes.len() as u64,
            media_type: "application/json".to_owned(),
        };
        Ok(Some(NewDocument { document, mp3s }))
    }

    /// The bytes of `key`, which the local store keeps no copy of when it lacks them.
    fn read(&self, key: &Key) -> io::Result<Vec<u8>> {
        let missing = || io::Error::new(io::ErrorKind::NotFound, format!("{key} is missing"));
        self.objects.fetch(key)?.ok_or_else(missing)
    }

    /// Puts `added`, which the local store holds, at the site. With a bucket, the local copies
    /// are removed after, so that the local store does not grow with the site.
    fn store(&self, added: Vec<Object>) -> Result<(), String> {
        let keys: Vec<Key> = added.iter().map(|object| object.key.clone()).collect();
        self.site.store(self.objects, added)?;
        if let Site::Bucket(_) = self.site {
            for key in &keys {
                let removed = self.objects.local.remove(key);
                removed.map_err(|e| format!("{key}: {e}"))?;
            }
        }
        Ok(())
    }
}

/// The key of the file of `member`.
fn member_key(member: &MemberFile) -> io::Result<Key> {
    let sha256 = parse_hex(&member.sha256)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "a member has a bad hash"))?;
    Ok(Key {
        sha256,
        extension: member.extension.clone(),
    })
}

/// The record packs that replace the files of records that name a replaced document, and those
/// of the files that the packs do not give again.
struct Replaced {
    packs: Vec<Pack>,
    files: Vec<Listed>,
    /// How many records name a new document.
    records: usize,
}

/// The files of `record_files` that hold a record naming a document of `replacements`, and the
/// packs of their records with the new documents named.
fn replaced(record_files: &[RecordFile], replacements: &Replacements) -> Result<Replaced, String> {
    let mut entries = Vec::new();
    let mut files = Vec::new();
    let mut records = 0;
    for file in record_files {
        let replacing = file
            .records
            .iter()
            .map(|(entry, _)| entry.replacing(replacements));
        let replacing = replacing.collect::<io::Result<Vec<Option<Entry>>>>();
        let replacing = replacing.map_err(|e| format!("{}: {e}", file.file.name))?;
        if replacing.iter().all(Option::is_none) {
            continue;
        }
        records += replacing.iter().flatten().count();
        let kept = file.records.iter().map(|(entry, _)| entry);
        let rewritten = replacing.into_iter().zip(kept);
        entries.extend(rewritten.map(|(new, old)| new.unwrap_or_else(|| old.clone())));
        files.push(file.file.clone());
    }
    let packs = Pack::of(entries).map_err(|e| format!("{RECORD_PACKS}: {e}"))?;
    let given: HashSet<&str> = packs.iter().map(|pack| pack.name.as_str()).collect();
    files.retain(|file| !given.contains(state_name(file)));
    Ok(Replaced {
        packs,
        files,
        records,
    })
}

/// Rewrites the records of units and derivations that the catalog of the output directory
/// `output` keeps, as `replacements` says, and returns how many it rewrote.
fn rewrite_catalog(
    output: &Path,
    store: &LocalStore,
    replacements: &Replacements,
) -> Result<usize, String> {
    let database = output.join("catalog.redb");
    if !database.is_file() {
        return Ok(0);
    }
    let rewrite = || -> io::Result<usize> {
        let catalog = Catalog::open_or_rebuild(&database, store)?;
        let units = catalog.units()?.into_iter().filter_map(|(key, bytes)| {
            let record = Record::read(&bytes)?.replacing(replacements)?;
            Some((key, record))
        });
        let units: Vec<_> = units.collect();
        let derivations = catalog.derivations()?.into_iter();
        let derivations = derivations.filter_map(|(fingerprint, derived)| match derived {
            Derived::Document(key) => replacements.get(&key).map(|new| (fingerprint, new)),
            Derived::Nothing => None,
        });
        let derivations: Vec<_> = derivations.collect();
        let mut batch = catalog.write()?;
        for (key, record) in &units {
            batch.put_unit(key, &record.bytes()?)?;
        }
        for (fingerprint, new) in &derivations {
            batch.put_derivation(fingerprint, &Derived::Document(new.key.clone()))?;
        }
        batch.commit()?;
        Ok(units.len() + derivations.len())
    };
    rewrite().map_err(|e| format!("{}: {e}", database.display()))
}
