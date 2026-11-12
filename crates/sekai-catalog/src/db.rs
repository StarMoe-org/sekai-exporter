//! The catalog kept in a redb database.
//!
//! The catalog is an index. Derivations can be rebuilt from the asset documents in the store, so
//! a damaged file is set aside and rebuilt instead of stopping a run.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::path::Path;
use std::sync::Mutex;

use redb::{
    Database, DatabaseError, ReadableDatabase, StorageError, TableDefinition, WriteTransaction,
};
use sekai_format::AssetDocument;

use crate::derivation::{BundleIndexes, Derivations, Derived, UnitRecords};
use crate::holdings::Holdings;
use crate::store::{Key, LocalStore, parse_hex};

const DERIVATIONS: TableDefinition<'_, &[u8; 32], &[u8]> = TableDefinition::new("derivations");
const BUNDLE_INDEXES: TableDefinition<'_, &[u8; 32], &[u8]> =
    TableDefinition::new("bundle_indexes");
/// Form, fingerprint, document and error code.
type AssetValue<'a> = (&'a str, &'a str, &'a str, Option<&'a str>);
const ASSETS: TableDefinition<'_, (&str, &str), AssetValue<'_>> = TableDefinition::new("assets");
/// Wall time, CPU time, bytes read, bytes written, estimated and measured peak memory.
type CostValue = (u64, u64, u64, u64, u64, u64);
const COSTS: TableDefinition<'_, (&str, &str), CostValue> = TableDefinition::new("costs");
const BUNDLES: TableDefinition<'_, (&str, &str), (&str, &str)> = TableDefinition::new("bundles");
const VERSIONS: TableDefinition<'_, (&str, &str), &str> = TableDefinition::new("versions");
const POINTERS: TableDefinition<'_, &str, &str> = TableDefinition::new("pointers");
const PUBLICATIONS: TableDefinition<'_, (&str, &str), (&str, &str)> =
    TableDefinition::new("publications");
/// The recorded result of each unit of an evaluation, by unit key.
const UNITS: TableDefinition<'_, &[u8; 32], &[u8]> = TableDefinition::new("units");
/// The size of each object the bucket of the catalog holds, by object name.
const OBJECTS: TableDefinition<'_, &str, u64> = TableDefinition::new("objects");

/// How many outputs [`CatalogDerivations`] holds before it writes them.
const BATCH: usize = 1024;

fn other(e: impl std::error::Error + Send + Sync + 'static) -> io::Error {
    io::Error::other(e)
}

fn is_damaged(error: &DatabaseError) -> bool {
    match error {
        DatabaseError::UpgradeRequired(_) | DatabaseError::RepairAborted => true,
        DatabaseError::Storage(StorageError::Corrupted(_)) => true,
        DatabaseError::Storage(StorageError::Io(e)) => e.kind() == io::ErrorKind::InvalidData,
        _ => false,
    }
}

fn encode(derived: &Derived) -> Vec<u8> {
    match derived {
        Derived::Nothing => Vec::new(),
        Derived::Document(key) => {
            let mut bytes = key.sha256.to_vec();
            bytes.extend_from_slice(key.extension.as_bytes());
            bytes
        }
    }
}

fn decode(bytes: &[u8]) -> io::Result<Derived> {
    if bytes.is_empty() {
        return Ok(Derived::Nothing);
    }
    let (sha256, extension) = bytes
        .split_first_chunk::<32>()
        .ok_or_else(|| io::Error::other("a derivation record is too short"))?;
    Ok(Derived::Document(Key {
        sha256: *sha256,
        extension: std::str::from_utf8(extension).map_err(other)?.to_owned(),
    }))
}

/// One asset of a version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetRow {
    /// The name of the asset.
    pub name: String,
    /// The asset form.
    pub form: String,
    /// The fingerprint, as lowercase hexadecimal.
    pub fingerprint: String,
    /// The asset document.
    pub document: Key,
    /// The error code of an asset that could not be produced.
    pub error: Option<String>,
}

/// What the last evaluation of a derivation cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostRow {
    /// The kind of derivation: an asset form or a view.
    pub kind: String,
    /// The name of the derivation.
    pub subject: String,
    /// The wall-clock time in microseconds.
    pub wall_us: u64,
    /// The CPU time in microseconds.
    pub cpu_us: u64,
    /// The bytes read.
    pub read: u64,
    /// The bytes written.
    pub written: u64,
    /// The estimated peak memory in bytes.
    pub estimated_memory: u64,
    /// The decoded size of the members in bytes.
    pub decoded_memory: u64,
}

/// The last fetch of a bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleRow {
    /// The hash the manifest declared.
    pub declared: String,
    /// The SHA-256 of the bytes, as lowercase hexadecimal.
    pub sha256: String,
}

/// A publication at a destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationRow {
    /// The published version.
    pub version: String,
    /// The revision of the publication.
    pub revision: String,
}

/// The catalog.
pub struct Catalog {
    db: Database,
}

impl Catalog {
    fn init(db: Database) -> io::Result<Self> {
        let tx = db.begin_write().map_err(other)?;
        tx.open_table(DERIVATIONS).map_err(other)?;
        tx.open_table(BUNDLE_INDEXES).map_err(other)?;
        tx.open_table(ASSETS).map_err(other)?;
        tx.open_table(COSTS).map_err(other)?;
        tx.open_table(BUNDLES).map_err(other)?;
        tx.open_table(VERSIONS).map_err(other)?;
        tx.open_table(POINTERS).map_err(other)?;
        tx.open_table(PUBLICATIONS).map_err(other)?;
        tx.open_table(OBJECTS).map_err(other)?;
        tx.open_table(UNITS).map_err(other)?;
        tx.commit().map_err(other)?;
        Ok(Self { db })
    }

    /// Opens the catalog at `path`, creating it if it does not exist. A damaged file is renamed
    /// to `<path>.corrupt` and its derivations are rebuilt from `store`.
    pub fn open_or_rebuild(path: &Path, store: &LocalStore) -> io::Result<Self> {
        match Database::create(path) {
            Ok(db) => Self::init(db),
            Err(e) if is_damaged(&e) => {
                let mut aside = path.as_os_str().to_owned();
                aside.push(".corrupt");
                std::fs::rename(path, aside)?;
                let catalog = Self::init(Database::create(path).map_err(other)?)?;
                catalog.rebuild(store)?;
                Ok(catalog)
            }
            Err(e) => Err(other(e)),
        }
    }

    /// Records the output of every successful asset document in `store`.
    pub fn rebuild(&self, store: &LocalStore) -> io::Result<()> {
        let mut batch = self.write()?;
        for key in store.keys("json")? {
            let Ok(document) = AssetDocument::from_bytes(&store.read(&key)?) else {
                continue;
            };
            if document.format != sekai_format::asset::FORMAT || document.error.is_some() {
                continue;
            }
            if let Some(fingerprint) = parse_hex(&document.fingerprint) {
                batch.put_derivation(&fingerprint, &Derived::Document(key))?;
            }
        }
        batch.commit()
    }

    /// Starts a batch of writes. Readers see none of it until it is committed.
    pub fn write(&self) -> io::Result<Batch> {
        Ok(Batch(self.db.begin_write().map_err(other)?))
    }

    /// The output of `fingerprint`.
    pub fn derivation(&self, fingerprint: &[u8; 32]) -> io::Result<Option<Derived>> {
        let tx = self.db.begin_read().map_err(other)?;
        let table = tx.open_table(DERIVATIONS).map_err(other)?;
        let value = table.get(fingerprint).map_err(other)?;
        value.map(|v| decode(v.value())).transpose()
    }

    /// The assets of a version, in name order.
    pub fn assets(&self, version: &str) -> io::Result<Vec<AssetRow>> {
        let tx = self.db.begin_read().map_err(other)?;
        let table = tx.open_table(ASSETS).map_err(other)?;
        let mut rows = Vec::new();
        for entry in table.range((version, "")..).map_err(other)? {
            let (key, value) = entry.map_err(other)?;
            let (row_version, name) = key.value();
            if row_version != version {
                break;
            }
            let (form, fingerprint, document, error) = value.value();
            rows.push(AssetRow {
                name: name.to_owned(),
                form: form.to_owned(),
                fingerprint: fingerprint.to_owned(),
                document: document.parse().map_err(io::Error::other)?,
                error: error.map(str::to_owned),
            });
        }
        Ok(rows)
    }

    /// The cost of every derivation that has been measured, in (kind, subject) order.
    pub fn costs(&self) -> io::Result<Vec<CostRow>> {
        let tx = self.db.begin_read().map_err(other)?;
        let table = tx.open_table(COSTS).map_err(other)?;
        let mut rows = Vec::new();
        for entry in table.range::<(&str, &str)>(..).map_err(other)? {
            let (key, value) = entry.map_err(other)?;
            let (kind, subject) = key.value();
            let (wall_us, cpu_us, read, written, estimated_memory, decoded_memory) = value.value();
            rows.push(CostRow {
                kind: kind.to_owned(),
                subject: subject.to_owned(),
                wall_us,
                cpu_us,
                read,
                written,
                estimated_memory,
                decoded_memory,
            });
        }
        Ok(rows)
    }

    /// The last fetch of a bundle in a region.
    pub fn bundle(&self, region: &str, name: &str) -> io::Result<Option<BundleRow>> {
        let tx = self.db.begin_read().map_err(other)?;
        let table = tx.open_table(BUNDLES).map_err(other)?;
        let value = table.get((region, name)).map_err(other)?;
        Ok(value.map(|v| {
            let (declared, sha256) = v.value();
            BundleRow {
                declared: declared.to_owned(),
                sha256: sha256.to_owned(),
            }
        }))
    }

    /// The identity document of a version.
    pub fn version(&self, region: &str, version: &str) -> io::Result<Option<String>> {
        let tx = self.db.begin_read().map_err(other)?;
        let table = tx.open_table(VERSIONS).map_err(other)?;
        let value = table.get((region, version)).map_err(other)?;
        Ok(value.map(|v| v.value().to_owned()))
    }

    /// The last committed version of a region.
    pub fn pointer(&self, region: &str) -> io::Result<Option<String>> {
        let tx = self.db.begin_read().map_err(other)?;
        let table = tx.open_table(POINTERS).map_err(other)?;
        let value = table.get(region).map_err(other)?;
        Ok(value.map(|v| v.value().to_owned()))
    }

    /// The publication of `target` at `destination`.
    pub fn publication(
        &self,
        destination: &str,
        target: &str,
    ) -> io::Result<Option<PublicationRow>> {
        let tx = self.db.begin_read().map_err(other)?;
        let table = tx.open_table(PUBLICATIONS).map_err(other)?;
        let value = table.get((destination, target)).map_err(other)?;
        Ok(value.map(|v| {
            let (version, revision) = v.value();
            PublicationRow {
                version: version.to_owned(),
                revision: revision.to_owned(),
            }
        }))
    }

    /// What the bucket of the catalog holds, as recorded.
    pub fn holdings(&self) -> CatalogHoldings<'_> {
        CatalogHoldings { catalog: self }
    }

    /// The catalog as the [`BundleIndexes`] of opening a version.
    pub fn bundle_indexes(&self) -> CatalogBundleIndexes<'_> {
        CatalogBundleIndexes {
            catalog: self,
            pending: Mutex::new(BTreeMap::new()),
        }
    }

    /// The catalog as the [`Derivations`] of an evaluation.
    pub fn into_derivations(self) -> CatalogDerivations {
        CatalogDerivations {
            catalog: self,
            pending: Mutex::new(BTreeMap::new()),
        }
    }
}

/// A batch of writes to the catalog.
pub struct Batch(WriteTransaction);

impl Batch {
    /// Records the output of `fingerprint`.
    pub fn put_derivation(&mut self, fingerprint: &[u8; 32], derived: &Derived) -> io::Result<()> {
        let mut table = self.0.open_table(DERIVATIONS).map_err(other)?;
        table
            .insert(fingerprint, encode(derived).as_slice())
            .map_err(other)?;
        Ok(())
    }

    /// Records the result of the unit `key`.
    pub fn put_unit(&mut self, key: &[u8; 32], result: &[u8]) -> io::Result<()> {
        let mut table = self.0.open_table(UNITS).map_err(other)?;
        table.insert(key, result).map_err(other)?;
        Ok(())
    }

    /// Stores the index of a bundle.
    pub fn put_bundle_index(&mut self, bundle: &[u8; 32], index: &[u8]) -> io::Result<()> {
        let mut table = self.0.open_table(BUNDLE_INDEXES).map_err(other)?;
        table.insert(bundle, index).map_err(other)?;
        Ok(())
    }

    /// Records one asset of a version.
    pub fn put_asset(&mut self, version: &str, row: &AssetRow) -> io::Result<()> {
        let mut table = self.0.open_table(ASSETS).map_err(other)?;
        let document = row.document.to_string();
        table
            .insert(
                (version, row.name.as_str()),
                (
                    row.form.as_str(),
                    row.fingerprint.as_str(),
                    document.as_str(),
                    row.error.as_deref(),
                ),
            )
            .map_err(other)?;
        Ok(())
    }

    /// Records what a derivation cost, replacing any earlier measurement.
    pub fn put_cost(&mut self, row: &CostRow) -> io::Result<()> {
        let mut table = self.0.open_table(COSTS).map_err(other)?;
        table
            .insert(
                (row.kind.as_str(), row.subject.as_str()),
                (
                    row.wall_us,
                    row.cpu_us,
                    row.read,
                    row.written,
                    row.estimated_memory,
                    row.decoded_memory,
                ),
            )
            .map_err(other)?;
        Ok(())
    }

    /// Records the last fetch of a bundle in a region.
    pub fn put_bundle(&mut self, region: &str, name: &str, row: &BundleRow) -> io::Result<()> {
        let mut table = self.0.open_table(BUNDLES).map_err(other)?;
        table
            .insert((region, name), (row.declared.as_str(), row.sha256.as_str()))
            .map_err(other)?;
        Ok(())
    }

    /// Records the identity document of a version.
    pub fn put_version(&mut self, region: &str, version: &str, identity: &str) -> io::Result<()> {
        let mut table = self.0.open_table(VERSIONS).map_err(other)?;
        table.insert((region, version), identity).map_err(other)?;
        Ok(())
    }

    /// Makes `version` the last committed version of a region.
    pub fn set_pointer(&mut self, region: &str, version: &str) -> io::Result<()> {
        let mut table = self.0.open_table(POINTERS).map_err(other)?;
        table.insert(region, version).map_err(other)?;
        Ok(())
    }

    /// Records the publication of `target` at `destination`.
    pub fn put_publication(
        &mut self,
        destination: &str,
        target: &str,
        row: &PublicationRow,
    ) -> io::Result<()> {
        let mut table = self.0.open_table(PUBLICATIONS).map_err(other)?;
        table
            .insert(
                (destination, target),
                (row.version.as_str(), row.revision.as_str()),
            )
            .map_err(other)?;
        Ok(())
    }

    /// Records that the bucket holds `key` with `size` bytes.
    pub fn hold(&mut self, key: &Key, size: u64) -> io::Result<()> {
        let mut table = self.0.open_table(OBJECTS).map_err(other)?;
        table
            .insert(key.to_string().as_str(), size)
            .map_err(other)?;
        Ok(())
    }

    /// Forgets every object recorded as held.
    pub fn release_all(&mut self) -> io::Result<()> {
        let mut table = self.0.open_table(OBJECTS).map_err(other)?;
        table.retain(|_, _| false).map_err(other)
    }

    /// Makes the batch visible and durable.
    pub fn commit(self) -> io::Result<()> {
        self.0.commit().map_err(other)
    }
}

/// [`BundleIndexes`] kept in the catalog and written in batches, in key order.
pub struct CatalogBundleIndexes<'c> {
    catalog: &'c Catalog,
    pending: Mutex<BTreeMap<[u8; 32], Vec<u8>>>,
}

impl CatalogBundleIndexes<'_> {
    fn write(&self, pending: &mut BTreeMap<[u8; 32], Vec<u8>>) -> io::Result<()> {
        let mut batch = self.catalog.write()?;
        for (bundle, index) in pending.iter() {
            batch.put_bundle_index(bundle, index)?;
        }
        batch.commit()?;
        pending.clear();
        Ok(())
    }
}

impl UnitRecords for Catalog {
    fn get(&self, key: &[u8; 32]) -> io::Result<Option<Vec<u8>>> {
        let tx = self.db.begin_read().map_err(other)?;
        let table = tx.open_table(UNITS).map_err(other)?;
        let value = table.get(key).map_err(other)?;
        Ok(value.map(|v| v.value().to_vec()))
    }

    fn put(&self, results: &[([u8; 32], Vec<u8>)]) -> io::Result<()> {
        let mut batch = self.write()?;
        for (key, result) in results {
            batch.put_unit(key, result)?;
        }
        batch.commit()
    }
}

impl BundleIndexes for CatalogBundleIndexes<'_> {
    fn get(&self, bundle: &[u8; 32]) -> io::Result<Option<Vec<u8>>> {
        if let Some(index) = self.pending.lock().unwrap().get(bundle) {
            return Ok(Some(index.clone()));
        }
        let tx = self.catalog.db.begin_read().map_err(other)?;
        let table = tx.open_table(BUNDLE_INDEXES).map_err(other)?;
        let value = table.get(bundle).map_err(other)?;
        Ok(value.map(|v| v.value().to_vec()))
    }

    fn put(&self, bundle: [u8; 32], index: Vec<u8>) -> io::Result<()> {
        let mut pending = self.pending.lock().unwrap();
        pending.insert(bundle, index);
        if pending.len() >= BATCH {
            self.write(&mut pending)?;
        }
        Ok(())
    }

    fn flush(&self) -> io::Result<()> {
        self.write(&mut self.pending.lock().unwrap())
    }
}

/// [`Derivations`] kept in the catalog and written in batches, in key order.
pub struct CatalogDerivations {
    catalog: Catalog,
    pending: Mutex<BTreeMap<[u8; 32], Derived>>,
}

impl CatalogDerivations {
    /// The catalog.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    fn write(&self, pending: &mut BTreeMap<[u8; 32], Derived>) -> io::Result<()> {
        let mut batch = self.catalog.write()?;
        for (fingerprint, derived) in pending.iter() {
            batch.put_derivation(fingerprint, derived)?;
        }
        batch.commit()?;
        pending.clear();
        Ok(())
    }
}

impl Derivations for CatalogDerivations {
    fn get(&self, fingerprint: &[u8; 32]) -> io::Result<Option<Derived>> {
        if let Some(derived) = self.pending.lock().unwrap().get(fingerprint) {
            return Ok(Some(derived.clone()));
        }
        self.catalog.derivation(fingerprint)
    }

    fn put(&self, fingerprint: [u8; 32], derived: Derived) -> io::Result<()> {
        let mut pending = self.pending.lock().unwrap();
        pending.insert(fingerprint, derived);
        if pending.len() >= BATCH {
            self.write(&mut pending)?;
        }
        Ok(())
    }

    fn flush(&self) -> io::Result<()> {
        self.write(&mut self.pending.lock().unwrap())
    }
}

/// What the bucket of the catalog holds, kept in the catalog.
pub struct CatalogHoldings<'c> {
    catalog: &'c Catalog,
}

impl CatalogHoldings<'_> {
    /// Records that the bucket holds exactly `objects`, such as a listing of it, in place of what
    /// was recorded.
    pub fn replace(&self, objects: &BTreeMap<Key, u64>) -> io::Result<()> {
        let mut batch = self.catalog.write()?;
        batch.release_all()?;
        for (key, &size) in objects {
            batch.hold(key, size)?;
        }
        batch.commit()
    }
}

impl Holdings for CatalogHoldings<'_> {
    fn size(&self, key: &Key) -> io::Result<Option<u64>> {
        let tx = self.catalog.db.begin_read().map_err(other)?;
        let table = tx.open_table(OBJECTS).map_err(other)?;
        let value = table.get(key.to_string().as_str()).map_err(other)?;
        Ok(value.map(|v| v.value()))
    }

    fn sizes(&self, prefix: u8) -> io::Result<HashMap<Key, u64>> {
        let tx = self.catalog.db.begin_read().map_err(other)?;
        let table = tx.open_table(OBJECTS).map_err(other)?;
        let start = format!("{prefix:02x}");
        let mut sizes = HashMap::new();
        for entry in table.range(start.as_str()..).map_err(other)? {
            let (name, size) = entry.map_err(other)?;
            let name = name.value();
            if !name.starts_with(&start) {
                break;
            }
            sizes.insert(name.parse().map_err(other)?, size.value());
        }
        Ok(sizes)
    }

    fn hold(&self, objects: &[(Key, u64)]) -> io::Result<()> {
        let mut batch = self.catalog.write()?;
        for (key, size) in objects {
            batch.hold(key, *size)?;
        }
        batch.commit()
    }
}
