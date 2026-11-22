//! Results that later runs of any region may take, kept in record packs among the state files:
//! the record of a unit under its key, and the output of a derivation under its fingerprint.
//!
//! A run reads every pack the first time it looks for a record, and keeps the records in memory.
//! It records a derivation as soon as the objects it names are in the destination, and writes the
//! records waiting in a pack once they fill one or a while after its last pack. The records of its
//! units go into its last pack, once the objects they name are there. A derivation whose object
//! the destination does not hold is not recorded. Every object a record names is therefore there,
//! and a run knows the size of each object a derivation record names without asking.

mod pack;

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io;
use std::sync::{Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use sekai_catalog::{Derivations, Derived, Holdings, Key, MemoryHoldings};

use self::pack::DerivationRecord;
pub use self::pack::{
    DERIVATION_RECORDS, Entry, PACK_BYTES, Pack, RECORD_PACKS, Subject, UNIT_RECORDS, is_pack,
};
use crate::eval::parallel;
use crate::reuse::{Found, Kept, Record, Recorded, UnitKey, UnitResult};
use crate::state::{RegionState, StateFiles, UnitState};

/// A document that replaces another, with its size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replacement {
    /// The document.
    pub key: Key,
    /// Its size in bytes.
    pub size: u64,
}

/// The documents that replace others, by the document each replaces.
pub type Replacements = HashMap<Key, Replacement>;

/// How long after its last pack a run writes the records waiting, though they fill no pack.
const PACK_INTERVAL: Duration = Duration::from_secs(300);

/// The records of earlier runs, and the derivations of this run.
pub struct Records<'a> {
    files: &'a dyn StateFiles,
    /// The objects that the destination holds, with their sizes: those the records taken name,
    /// and those the run puts there.
    held: &'a MemoryHoldings,
    /// How many packs are read or written at once.
    requests: usize,
    /// The records of the packs, read the first time a record is looked for.
    packed: OnceLock<Packed>,
    /// Held while the packs are read, so that they are read once.
    reading: Mutex<()>,
    /// The derivations this run made, by fingerprint.
    made: Mutex<HashMap<[u8; 32], Derived>>,
    /// The keys of the unit records read, which need not be written again.
    read: Mutex<HashSet<UnitKey>>,
    /// The records not yet in a pack.
    waiting: Mutex<Waiting>,
}

/// The records of a run not yet in a pack.
struct Waiting {
    entries: Vec<Entry>,
    /// The bytes of their lines, which [`PACK_BYTES`] counts.
    bytes: usize,
    /// When the run started or last wrote a pack.
    since: Instant,
}

impl Waiting {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            bytes: 0,
            since: Instant::now(),
        }
    }

    /// Adds `entry`, and returns every record waiting once they fill a pack or have waited long
    /// enough.
    fn add(&mut self, entry: Entry) -> Option<Vec<Entry>> {
        self.bytes += entry.size();
        self.entries.push(entry);
        let due = self.bytes >= PACK_BYTES || self.since.elapsed() >= PACK_INTERVAL;
        due.then(|| self.take())
    }

    fn take(&mut self) -> Vec<Entry> {
        std::mem::replace(self, Self::new()).entries
    }
}

/// The records of the packs, as a run keeps them.
#[derive(Default)]
struct Packed {
    /// The JSON of each unit record.
    units: HashMap<UnitKey, Box<str>>,
    /// The document of each derivation record, or `None` for one that gave nothing.
    derivations: HashMap<[u8; 32], Option<Document>>,
    /// The extensions of the documents, which [`Document::extension`] indexes.
    extensions: Vec<String>,
}

/// A document that a derivation record names.
struct Document {
    sha256: [u8; 32],
    extension: usize,
    size: u64,
}

impl Packed {
    /// The records of the packs that `files` keep, reading up to `requests` packs at once. Of
    /// the records of one subject, the one in the pack whose name comes first is taken.
    fn read(files: &dyn StateFiles, requests: usize) -> io::Result<Self> {
        let mut names = files.list(RECORD_PACKS)?;
        names.retain(|name| is_pack(name));
        names.sort();
        let mut packed = Self::default();
        for batch in names.chunks(requests.max(1)) {
            let read = parallel(batch.len(), requests, |i| {
                let name = &batch[i];
                // A pack gone since the listing holds nothing the run can take.
                let bytes = files.read(name)?.unwrap_or_default();
                if bytes.is_empty() {
                    return Ok(Vec::new());
                }
                Pack::read(name, &bytes)
            });
            for entries in read {
                for entry in entries? {
                    packed.add(entry)?;
                }
            }
        }
        Ok(packed)
    }

    fn add(&mut self, entry: Entry) -> io::Result<()> {
        match entry.subject {
            Subject::Unit(key) => {
                self.units
                    .entry(key)
                    .or_insert_with(|| entry.into_json().into_boxed_str());
            }
            Subject::Derivation(fingerprint) if !self.derivations.contains_key(&fingerprint) => {
                let document = entry.derivation_record()?.document()?;
                let document = document.map(|(key, size)| self.document(key, size));
                self.derivations.insert(fingerprint, document);
            }
            Subject::Derivation(_) => {}
        }
        Ok(())
    }

    fn document(&mut self, key: Key, size: u64) -> Document {
        let known = self.extensions.iter().position(|e| *e == key.extension);
        let extension = known.unwrap_or_else(|| {
            self.extensions.push(key.extension);
            self.extensions.len() - 1
        });
        Document {
            sha256: key.sha256,
            extension,
            size,
        }
    }

    /// The derivation recorded under `fingerprint`: the document it gave with its size, or
    /// `None` when it gave nothing.
    fn derivation(&self, fingerprint: &[u8; 32]) -> Option<Option<(Key, u64)>> {
        let document = self.derivations.get(fingerprint)?.as_ref();
        Some(document.map(|document| {
            let key = Key {
                sha256: document.sha256,
                extension: self.extensions[document.extension].clone(),
            };
            (key, document.size)
        }))
    }
}

/// The derivation that a record of a document with its size, or of nothing, names.
fn derived_of(document: Option<(Key, u64)>) -> Derived {
    document.map_or(Derived::Nothing, |(key, _size)| Derived::Document(key))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<'a> Records<'a> {
    /// The records `files` keep, for a destination that holds `held`, reading and writing up to
    /// `requests` packs at once. The objects the derivation records name are added to `held` as
    /// they are taken.
    pub fn new(files: &'a dyn StateFiles, held: &'a MemoryHoldings, requests: usize) -> Self {
        Self {
            files,
            held,
            requests,
            packed: OnceLock::new(),
            reading: Mutex::new(()),
            made: Mutex::default(),
            read: Mutex::default(),
            waiting: Mutex::new(Waiting::new()),
        }
    }

    /// The records of the packs, read once.
    fn packed(&self) -> io::Result<&Packed> {
        if let Some(packed) = self.packed.get() {
            return Ok(packed);
        }
        let _reading = lock(&self.reading);
        if let Some(packed) = self.packed.get() {
            return Ok(packed);
        }
        let packed = Packed::read(self.files, self.requests)?;
        Ok(self.packed.get_or_init(|| packed))
    }

    /// The asset documents that the unit records of [`Records::write`] of `units` name.
    pub fn documents(&self, units: &[&UnitResult]) -> io::Result<Vec<Key>> {
        let mut documents = Vec::new();
        for (_, record) in self.to_write(units) {
            documents.extend(record.documents()?);
        }
        documents.sort();
        documents.dedup();
        Ok(documents)
    }

    /// The units of `units` whose records a write writes, with their keys: those a later run may
    /// take, unless their records were read from the packs.
    fn to_write<'u>(&self, units: &[&'u UnitResult]) -> Vec<(UnitKey, &'u Record)> {
        let read = lock(&self.read);
        let written = units.iter().filter_map(|unit| {
            let key = unit.reusable_key().filter(|key| !read.contains(key))?;
            Some((*key, &unit.record))
        });
        written.collect()
    }

    /// Writes the record of each of `units` that a later run may take and that was not read from
    /// the packs, with every derivation record still waiting. Call it once the objects the records
    /// name are in the destination. Returns how many unit records were written.
    pub fn write(&self, units: &[&UnitResult]) -> io::Result<usize> {
        let units = self.to_write(units);
        let mut entries = lock(&self.waiting).take();
        for (key, record) in &units {
            entries.push(Entry::unit(*key, record)?);
        }
        self.write_packs(entries)?;
        Ok(units.len())
    }

    /// Remembers that the derivation `fingerprint` gave `derived` in this run. Returns whether
    /// that is new: a derivation made twice, or one the packs hold, is recorded once.
    pub fn remember(&self, fingerprint: [u8; 32], derived: &Derived) -> bool {
        let packed = self
            .packed
            .get()
            .and_then(|packed| packed.derivation(&fingerprint));
        if packed.map(derived_of).as_ref() == Some(derived) {
            return false;
        }
        let earlier = lock(&self.made).insert(fingerprint, derived.clone());
        earlier.as_ref() != Some(derived)
    }

    /// Records the derivation `fingerprint` that gave `derived`. Call it once the objects it names
    /// are in the destination. Returns whether it was recorded: a derivation whose object the
    /// destination does not hold is not.
    pub fn record_derivation(&self, fingerprint: &[u8; 32], derived: &Derived) -> io::Result<bool> {
        let Some(recorded) = self.recorded(derived)? else {
            return Ok(false);
        };
        let entry = Entry::derivation(*fingerprint, &recorded)?;
        let due = lock(&self.waiting).add(entry);
        if let Some(entries) = due {
            self.write_packs(entries)?;
        }
        Ok(true)
    }

    /// The record of `derived`, or `None` when the destination does not hold its object.
    fn recorded(&self, derived: &Derived) -> io::Result<Option<DerivationRecord>> {
        Ok(match derived {
            Derived::Document(key) => {
                let size = self.held.size(key)?;
                size.map(|size| DerivationRecord::Document {
                    key: key.to_string(),
                    size,
                })
            }
            Derived::Nothing => Some(DerivationRecord::Nothing),
        })
    }

    /// Writes `entries` in packs.
    fn write_packs(&self, entries: Vec<Entry>) -> io::Result<()> {
        let packs = Pack::of(entries)?;
        let written = parallel(packs.len(), self.requests, |i| {
            self.files.write(&packs[i].name, &packs[i].bytes)
        });
        written.into_iter().collect()
    }
}

impl Derivations for Records<'_> {
    fn get(&self, fingerprint: &[u8; 32]) -> io::Result<Option<Derived>> {
        if let Some(derived) = lock(&self.made).get(fingerprint) {
            return Ok(Some(derived.clone()));
        }
        let Some(document) = self.packed()?.derivation(fingerprint) else {
            return Ok(None);
        };
        if let Some(held) = &document {
            self.held.hold(std::slice::from_ref(held))?;
        }
        Ok(Some(derived_of(document)))
    }

    /// Remembers the derivation for this run; [`Records::record_derivation`] records it.
    fn put(&self, fingerprint: [u8; 32], derived: Derived) -> io::Result<()> {
        self.remember(fingerprint, &derived);
        Ok(())
    }
}

impl Recorded for Records<'_> {
    fn record(&self, _unit: &str, key: &UnitKey) -> io::Result<Option<Found>> {
        let units = &self.packed()?.units;
        let Some(record) = units
            .get(key)
            .and_then(|json| Record::read(json.as_bytes()))
        else {
            return Ok(None);
        };
        lock(&self.read).insert(*key);
        Ok(Some(Found::Recorded(record)))
    }
}

/// What a run of a region finds of earlier runs: the units that the last run of the region left
/// under the same key, which keep what they gave, and else the records of every run.
pub struct Earlier<'a> {
    /// The units of the last version of the region whose results a later run may keep, by name.
    units: HashMap<&'a str, &'a UnitState>,
    /// The forms whose roots the last run of the region looked for.
    scanned: &'a BTreeSet<String>,
    records: &'a dyn Recorded,
}

impl<'a> Earlier<'a> {
    /// The units that the last run of the region left, `state`, and the records `records`.
    pub fn new(state: &'a RegionState, records: &'a dyn Recorded) -> Self {
        let units = state.units.iter();
        let units = units.map(|unit| (unit.unit.as_str(), unit)).collect();
        Self {
            units,
            scanned: &state.scanned,
            records,
        }
    }
}

impl Recorded for Earlier<'_> {
    fn record(&self, unit: &str, key: &UnitKey) -> io::Result<Option<Found>> {
        match self.units.get(unit) {
            Some(earlier) if earlier.key == *key => Ok(Some(Found::Kept(Kept {
                forms: earlier.forms.clone(),
                scanned: self.scanned.clone(),
            }))),
            _ => self.records.record(unit, key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::MemoryFiles;

    fn unit(name: &str, key: u8, reusable: bool) -> UnitResult {
        UnitResult {
            unit: name.to_owned(),
            key: Some(UnitKey([key; 32])),
            window: vec![name.to_owned()],
            reusable,
            record: Record::default(),
        }
    }

    fn packs(files: &MemoryFiles) -> Vec<String> {
        files.list(RECORD_PACKS).unwrap()
    }

    #[test]
    fn derivations_are_recorded_with_the_sizes_of_their_held_objects() {
        let files = MemoryFiles::default();
        let document = Key::of(b"{\"a\":1}", "json");
        let unheld = Key::of(b"unheld", "webp");
        let held = MemoryHoldings::default();
        let records = Records::new(&files, &held, 2);
        records
            .put([1; 32], Derived::Document(document.clone()))
            .unwrap();
        records.put([2; 32], Derived::Nothing).unwrap();
        records
            .put([4; 32], Derived::Document(unheld.clone()))
            .unwrap();
        assert_eq!(
            records.get(&[1; 32]).unwrap(),
            Some(Derived::Document(document.clone()))
        );
        held.hold(&[(document.clone(), 7)]).unwrap();
        let recorded = [
            ([1; 32], Derived::Document(document.clone())),
            ([2; 32], Derived::Nothing),
            ([4; 32], Derived::Document(unheld)),
        ]
        .map(|(fingerprint, derived)| records.record_derivation(&fingerprint, &derived).unwrap());
        assert_eq!(recorded, [true, true, false]);
        // The records wait for the last pack of the run.
        assert!(packs(&files).is_empty());
        records.write(&[]).unwrap();
        assert_eq!(packs(&files).len(), 1);

        let later_held = MemoryHoldings::default();
        let later = Records::new(&files, &later_held, 2);
        assert_eq!(
            later.get(&[1; 32]).unwrap(),
            Some(Derived::Document(document.clone()))
        );
        assert_eq!(later.get(&[2; 32]).unwrap(), Some(Derived::Nothing));
        assert_eq!(later.get(&[3; 32]).unwrap(), None);
        assert_eq!(later.get(&[4; 32]).unwrap(), None);
        assert_eq!(later_held.size(&document).unwrap(), Some(7));
        // A derivation the packs hold, or one made twice, is recorded once.
        assert!(!later.remember([2; 32], &Derived::Nothing));
        assert!(later.remember([3; 32], &Derived::Nothing));
        assert!(!later.remember([3; 32], &Derived::Nothing));
    }

    #[test]
    fn records_that_fill_a_pack_are_written_before_the_run_ends() {
        let files = MemoryFiles::default();
        let held = MemoryHoldings::default();
        let records = Records::new(&files, &held, 2);
        let mut recorded: u64 = 0;
        while packs(&files).is_empty() {
            let mut fingerprint = [0; 32];
            fingerprint[..8].copy_from_slice(&recorded.to_be_bytes());
            records
                .record_derivation(&fingerprint, &Derived::Nothing)
                .unwrap();
            recorded += 1;
        }
        assert!(recorded > 1000, "{recorded}");
        let later = Records::new(&files, &held, 2);
        assert_eq!(later.get(&[0; 32]).unwrap(), Some(Derived::Nothing));
    }

    #[test]
    fn only_reusable_units_are_recorded() {
        let files = MemoryFiles::default();
        let held = MemoryHoldings::default();
        let (kept, broken) = (unit("a", 1, true), unit("b", 2, false));
        let records = Records::new(&files, &held, 2);
        assert_eq!(records.write(&[&kept, &broken]).unwrap(), 1);
        let later = Records::new(&files, &held, 2);
        let key = |unit: &UnitResult| unit.key.unwrap();
        assert_eq!(
            later.record("a", &key(&kept)).unwrap(),
            Some(Found::Recorded(Record::default()))
        );
        assert_eq!(later.record("b", &key(&broken)).unwrap(), None);
    }

    #[test]
    fn a_record_read_is_not_written_again() {
        let files = MemoryFiles::default();
        let held = MemoryHoldings::default();
        let shared = unit("shared", 3, true);
        let first = Records::new(&files, &held, 1);
        assert_eq!(first.write(&[&shared]).unwrap(), 1);

        let later = Records::new(&files, &held, 1);
        assert!(later.record("shared", &UnitKey([3; 32])).unwrap().is_some());
        let new = unit("new", 4, true);
        assert_eq!(later.write(&[&shared, &new]).unwrap(), 1);
    }

    #[test]
    fn the_last_run_of_the_region_comes_first() {
        let files = MemoryFiles::default();
        let held = MemoryHoldings::default();
        let elsewhere = unit("shared", 3, true);
        Records::new(&files, &held, 1).write(&[&elsewhere]).unwrap();

        let records = Records::new(&files, &held, 1);
        let last = RegionState {
            units: [unit("a", 1, true), unit("b", 2, false)]
                .iter()
                .filter_map(UnitState::of)
                .collect(),
            scanned: BTreeSet::from(["texture".to_owned()]),
            ..RegionState::default()
        };
        let earlier = Earlier::new(&last, &records);
        let kept = Kept {
            scanned: last.scanned.clone(),
            ..Kept::default()
        };
        assert_eq!(
            earlier.record("a", &UnitKey([1; 32])).unwrap(),
            Some(Found::Kept(kept))
        );
        // A result that may not be taken, or one under another key, is not.
        assert!(earlier.record("b", &UnitKey([2; 32])).unwrap().is_none());
        assert!(earlier.record("a", &UnitKey([9; 32])).unwrap().is_none());
        // Another region recorded this one.
        assert_eq!(
            earlier.record("shared", &UnitKey([3; 32])).unwrap(),
            Some(Found::Recorded(Record::default()))
        );
    }
}
