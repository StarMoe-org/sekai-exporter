//! The reads of an asset built from tables, noted while it is built and checked again in a later
//! run.

use std::collections::{BTreeSet, HashMap};
use std::io;
use std::sync::{Mutex, MutexGuard};

use sekai_catalog::Key;
use sekai_convert::ConvertError;
use sekai_convert::tables::{AssetEntry, SharedEntries, TableContext, TablePlan};
use sekai_format::AssetDocument;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::Assets;
use crate::eval::hex;

/// A read of an asset built from tables.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Read {
    /// The table of the name.
    Table(String),
    /// The list of the bundles of the version.
    Bundles,
    /// The assets of the bundle of the name.
    Assets(String),
    /// The document of the asset of the name, or a member of it.
    Document(String),
    /// The entry `key` of the shared map `map` of the plan.
    Shared {
        /// The name of the map.
        map: String,
        /// The key of the entry, as text.
        key: String,
    },
}

/// The reads of an asset built from tables, with a digest of what they gave.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trace {
    /// The reads, in order.
    pub reads: Vec<Read>,
    /// The digest of the form and of what the reads gave.
    pub digest: String,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The context an asset is built with: the version, with every read of the asset noted.
pub(super) struct Traced<'a> {
    version: &'a Assets<'a>,
    reads: Mutex<BTreeSet<Read>>,
}

impl<'a> Traced<'a> {
    pub(super) fn new(version: &'a Assets<'a>) -> Self {
        Self {
            version,
            reads: Mutex::default(),
        }
    }

    fn note(&self, read: Read) {
        lock(&self.reads).insert(read);
    }

    /// The reads noted, in order.
    pub(super) fn reads(self) -> Vec<Read> {
        let reads = self.reads.into_inner().unwrap_or_else(|e| e.into_inner());
        reads.into_iter().collect()
    }
}

impl TableContext for Traced<'_> {
    fn table(&self, name: &str) -> Result<Vec<u8>, ConvertError> {
        self.note(Read::Table(name.to_owned()));
        self.version.table(name)
    }

    fn bundles(&self) -> Vec<&str> {
        self.note(Read::Bundles);
        self.version.bundles()
    }

    fn assets(&self, bundle: &str) -> &[AssetEntry] {
        self.note(Read::Assets(bundle.to_owned()));
        self.version.assets(bundle)
    }

    fn document(&self, name: &str) -> Result<AssetDocument, ConvertError> {
        self.note(Read::Document(name.to_owned()));
        self.version.document(name)
    }

    fn identity(&self, name: &str) -> Result<String, ConvertError> {
        self.note(Read::Document(name.to_owned()));
        self.version.identity(name)
    }

    fn member(&self, name: &str, role: &str) -> Result<Vec<u8>, ConvertError> {
        self.note(Read::Document(name.to_owned()));
        self.version.member(name, role)
    }

    fn members(&self, names: &[&str], role: &str) -> Vec<Result<Vec<u8>, ConvertError>> {
        for name in names {
            self.note(Read::Document((*name).to_owned()));
        }
        self.version.members(names, role)
    }

    fn used(&self, map: &str, key: &str) {
        self.note(Read::Shared {
            map: map.to_owned(),
            key: key.to_owned(),
        });
    }
}

/// What reads give in this run, each table, listing of a bundle and document looked up once.
pub(super) struct Values<'a> {
    version: &'a Assets<'a>,
    tables: Mutex<HashMap<String, Option<String>>>,
    listings: Mutex<HashMap<String, String>>,
    documents: Mutex<HashMap<String, Option<Key>>>,
}

impl<'a> Values<'a> {
    pub(super) fn new(version: &'a Assets<'a>) -> Self {
        Self {
            version,
            tables: Mutex::default(),
            listings: Mutex::default(),
            documents: Mutex::default(),
        }
    }

    /// The digest of `converter`, the identity of the converter of an asset of `plan`, and of
    /// what `reads` give for the asset, or `None` when what one of them gives cannot be compared
    /// with what it gave. A failure of the store is an error.
    pub(super) fn digest(
        &self,
        converter: &str,
        reads: &[Read],
        plan: &dyn TablePlan,
    ) -> io::Result<Option<String>> {
        let shared = plan.shared();
        let mut hasher = Sha256::new();
        hasher.update(format!("table reads 1\nform {converter}\n"));
        for read in reads {
            let Some(value) = self.value(read, &shared)? else {
                return Ok(None);
            };
            let read = serde_json::to_string(read).map_err(io::Error::other)?;
            hasher.update(format!("{read} {value}\n"));
        }
        Ok(Some(hex(&hasher.finalize())))
    }

    /// What `read` gives, as text, or `None` when it cannot be compared.
    fn value(&self, read: &Read, shared: &[&dyn SharedEntries]) -> io::Result<Option<String>> {
        Ok(match read {
            Read::Table(name) => self.table(name),
            Read::Bundles => Some(digest_of(self.version.bundles().join("\n"))),
            Read::Assets(bundle) => Some(self.listing(bundle)),
            Read::Document(name) => {
                let key = self.document(name)?;
                Some(key.map_or_else(|| "none".to_owned(), |key| key.to_string()))
            }
            Read::Shared { map, key } => {
                let map = shared.iter().find(|shared| shared.name() == map);
                let entry = map.and_then(|map| map.entry(key).ok());
                entry.map(|entry| entry.map_or_else(|| "none".to_owned(), digest_of))
            }
        })
    }

    /// The digest of the table `name`, or `None` when it cannot be read.
    fn table(&self, name: &str) -> Option<String> {
        if let Some(found) = lock(&self.tables).get(name) {
            return found.clone();
        }
        let digest = (self.version.read)(name).ok().map(digest_of);
        lock(&self.tables).insert(name.to_owned(), digest.clone());
        digest
    }

    /// The key of the document of the asset `name`, if the version has one.
    fn document(&self, name: &str) -> io::Result<Option<Key>> {
        if let Some(found) = lock(&self.documents).get(name) {
            return Ok(found.clone());
        }
        let key = (self.version.version.document)(name)?;
        lock(&self.documents).insert(name.to_owned(), key.clone());
        Ok(key)
    }

    /// The digest of the assets of `bundle`.
    fn listing(&self, bundle: &str) -> String {
        if let Some(found) = lock(&self.listings).get(bundle) {
            return found.clone();
        }
        let entries = self.version.assets(bundle).iter();
        let lines: Vec<String> = entries
            .map(|entry| format!("{} {}", entry.name, entry.form))
            .collect();
        let digest = digest_of(lines.join("\n"));
        lock(&self.listings).insert(bundle.to_owned(), digest.clone());
        digest
    }
}

fn digest_of(bytes: impl AsRef<[u8]>) -> String {
    hex(&Sha256::digest(bytes.as_ref()))
}
