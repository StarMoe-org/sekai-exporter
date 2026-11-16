//! Results that later runs of any region may take, kept as state files: the record of a unit
//! under its key, and the output of a derivation under its fingerprint.
//!
//! A run reads a record only when it needs it, and writes the records of what it derived once
//! the objects they name are in the destination. A derivation whose object the destination does
//! not hold is not recorded. Every object a record names is therefore there, and a run knows the
//! size of each object a derivation record names without asking.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use sekai_catalog::store::parse_hex;
use sekai_catalog::{Derivations, Derived, Holdings, Key, MemoryHoldings};
use serde::{Deserialize, Serialize};

use crate::eval::{hex, parallel};
use crate::reuse::{Found, Kept, Record, Recorded, UnitKey, UnitResult};
use crate::state::{RegionState, StateFiles, UnitState};

/// The directory of the unit records among the state files.
pub const UNIT_RECORDS: &str = "records/units";

fn unit_file(key: &UnitKey) -> String {
    let key = key.to_string();
    format!("{UNIT_RECORDS}/{}/{key}.json", &key[..2])
}

/// The key of the unit record named `file`, when it names one.
fn unit_key(file: &str) -> Option<UnitKey> {
    let name = file.strip_prefix(UNIT_RECORDS)?.rsplit('/').next()?;
    let key = name.strip_suffix(".json")?;
    parse_hex(key).map(UnitKey)
}

/// The directory of the derivation records among the state files.
pub const DERIVATION_RECORDS: &str = "records/derivations";

fn derivation_file(fingerprint: &[u8; 32]) -> String {
    let fingerprint = hex(fingerprint);
    format!(
        "{DERIVATION_RECORDS}/{}/{fingerprint}.json",
        &fingerprint[..2]
    )
}

/// The document that the derivation record `bytes`, the state file `name`, names, with its size,
/// or `None` for a root that holds no asset of its form.
fn recorded_document(name: &str, bytes: &[u8]) -> io::Result<Option<(Key, u64)>> {
    let recorded = serde_json::from_slice(bytes).map_err(|e| invalid(name, e))?;
    match recorded {
        DerivationRecord::Document { key, size } => {
            let key: Key = key.parse().map_err(|e| invalid(name, e))?;
            Ok(Some((key, size)))
        }
        DerivationRecord::Nothing => Ok(None),
    }
}

/// The objects that the record `bytes`, the state file `file`, names: the asset documents of a
/// unit record, or the document of a derivation record. A unit record that cannot be read names
/// nothing, since a run evaluates its unit again.
pub fn record_objects(file: &str, bytes: &[u8]) -> io::Result<Vec<Key>> {
    if file.starts_with(UNIT_RECORDS) {
        return Record::read(bytes).map_or(Ok(Vec::new()), |record| record.documents());
    }
    if file.starts_with(DERIVATION_RECORDS) {
        let document = recorded_document(file, bytes)?;
        return Ok(document.into_iter().map(|(key, _size)| key).collect());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("{file} is not a record"),
    ))
}

/// What a derivation record holds.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DerivationRecord {
    /// The asset document, and its size.
    Document { key: String, size: u64 },
    /// The root holds no asset of its form.
    Nothing,
}

/// The records of earlier runs, and those of this run until they are written.
pub struct Records<'a> {
    files: &'a dyn StateFiles,
    /// The objects that the destination holds, with their sizes: those the records read name,
    /// and those the run puts there.
    held: &'a MemoryHoldings,
    /// The derivations read or made, by fingerprint.
    derivations: Mutex<HashMap<[u8; 32], Option<Derived>>>,
    /// The derivations made by this run, to write.
    made: Mutex<Vec<([u8; 32], Derived)>>,
    /// The keys of the unit records read, which need not be written again.
    read: Mutex<HashSet<UnitKey>>,
    /// The keys of every unit record the files keep, when a listing of them tells; a unit whose
    /// key is not among them is not looked for.
    listed: Option<HashSet<UnitKey>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn invalid(name: &str, e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{name}: {e}"))
}

impl<'a> Records<'a> {
    /// The records `files` keep, for a destination that holds `held`. The objects the derivation
    /// records name are added to `held` as they are read.
    pub fn new(files: &'a dyn StateFiles, held: &'a MemoryHoldings) -> Self {
        Self {
            files,
            held,
            derivations: Mutex::default(),
            made: Mutex::default(),
            read: Mutex::default(),
            listed: None,
        }
    }

    /// [`Records::new`], for files that keep the unit records `unit_files` in [`UNIT_RECORDS`]
    /// and no others.
    pub fn listed(
        files: &'a dyn StateFiles,
        held: &'a MemoryHoldings,
        unit_files: &[String],
    ) -> Self {
        let listed = unit_files.iter().filter_map(|file| unit_key(file));
        Self {
            listed: Some(listed.collect()),
            ..Self::new(files, held)
        }
    }

    /// The asset documents that the unit records of [`Records::write`] of `units` name.
    pub fn documents(&self, units: &[&UnitResult]) -> io::Result<Vec<Key>> {
        let mut documents = Vec::new();
        for unit in self.to_write(units) {
            documents.extend(unit.record.documents()?);
        }
        documents.sort();
        documents.dedup();
        Ok(documents)
    }

    /// The units of `units` whose records a write writes: those a later run may take, unless
    /// their records were read from the files.
    fn to_write<'u>(&self, units: &[&'u UnitResult]) -> Vec<&'u UnitResult> {
        let read = lock(&self.read);
        let written = units.iter().copied();
        written
            .filter(|unit| unit.reusable_key().is_some_and(|key| !read.contains(key)))
            .collect()
    }

    /// Writes the derivations this run made whose objects the destination holds, and the record
    /// of each of `units` that a later run may take and that was not read from the records, on up
    /// to `threads` threads. Call it once the objects the unit records name are in the
    /// destination. Returns how many records were written.
    pub fn write(&self, units: &[&UnitResult], threads: usize) -> io::Result<usize> {
        let units = self.to_write(units);
        let made = std::mem::take(&mut *lock(&self.made));
        let mut held = Vec::with_capacity(made.len());
        for (fingerprint, derived) in made {
            if let Some(recorded) = self.recorded(derived)? {
                held.push((fingerprint, recorded));
            }
        }
        let derivations = parallel(held.len(), threads, |i| {
            let (fingerprint, recorded) = &held[i];
            self.write_derivation(fingerprint, recorded)
        });
        let records = parallel(units.len(), threads, |i| self.write_unit(units[i]));
        derivations
            .into_iter()
            .chain(records)
            .collect::<io::Result<Vec<()>>>()
            .map(|written| written.len())
    }

    /// The record of `derived`, or `None` when the destination does not hold its object.
    fn recorded(&self, derived: Derived) -> io::Result<Option<DerivationRecord>> {
        Ok(match derived {
            Derived::Document(key) => {
                let size = self.held.size(&key)?;
                size.map(|size| DerivationRecord::Document {
                    key: key.to_string(),
                    size,
                })
            }
            Derived::Nothing => Some(DerivationRecord::Nothing),
        })
    }

    fn write_derivation(
        &self,
        fingerprint: &[u8; 32],
        recorded: &DerivationRecord,
    ) -> io::Result<()> {
        let bytes = serde_json::to_vec(recorded).map_err(io::Error::other)?;
        self.files.write(&derivation_file(fingerprint), &bytes)
    }

    fn write_unit(&self, unit: &UnitResult) -> io::Result<()> {
        let Some(key) = unit.reusable_key() else {
            return Ok(());
        };
        self.files.write(&unit_file(key), &unit.record.bytes()?)
    }

    /// The derivation recorded under `fingerprint`, read from the files.
    fn read_derivation(&self, fingerprint: &[u8; 32]) -> io::Result<Option<Derived>> {
        let name = derivation_file(fingerprint);
        let Some(bytes) = self.files.read(&name)? else {
            return Ok(None);
        };
        Ok(Some(match recorded_document(&name, &bytes)? {
            Some((key, size)) => {
                self.held.hold(&[(key.clone(), size)])?;
                Derived::Document(key)
            }
            None => Derived::Nothing,
        }))
    }
}

impl Derivations for Records<'_> {
    fn get(&self, fingerprint: &[u8; 32]) -> io::Result<Option<Derived>> {
        if let Some(derived) = lock(&self.derivations).get(fingerprint) {
            return Ok(derived.clone());
        }
        let derived = self.read_derivation(fingerprint)?;
        lock(&self.derivations).insert(*fingerprint, derived.clone());
        Ok(derived)
    }

    fn put(&self, fingerprint: [u8; 32], derived: Derived) -> io::Result<()> {
        lock(&self.derivations).insert(fingerprint, Some(derived.clone()));
        lock(&self.made).push((fingerprint, derived));
        Ok(())
    }
}

impl Recorded for Records<'_> {
    fn record(&self, _unit: &str, key: &UnitKey) -> io::Result<Option<Found>> {
        if self
            .listed
            .as_ref()
            .is_some_and(|listed| !listed.contains(key))
        {
            return Ok(None);
        }
        let bytes = self.files.read(&unit_file(key))?;
        let Some(record) = bytes.and_then(|bytes| Record::read(&bytes)) else {
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
            reusable,
            record: Record::default(),
        }
    }

    #[test]
    fn derivations_are_written_with_the_sizes_of_their_held_objects() {
        let files = MemoryFiles::default();
        let document = Key::of(b"{\"a\":1}", "json");
        let unheld = Key::of(b"unheld", "webp");
        let held = MemoryHoldings::default();
        let records = Records::new(&files, &held);
        records
            .put([1; 32], Derived::Document(document.clone()))
            .unwrap();
        records.put([2; 32], Derived::Nothing).unwrap();
        records.put([4; 32], Derived::Document(unheld)).unwrap();
        assert_eq!(
            records.get(&[1; 32]).unwrap(),
            Some(Derived::Document(document.clone()))
        );
        assert!(files.names().is_empty());
        held.hold(&[(document.clone(), 7)]).unwrap();
        assert_eq!(records.write(&[], 2).unwrap(), 2);

        let later_held = MemoryHoldings::default();
        let later = Records::new(&files, &later_held);
        assert_eq!(
            later.get(&[1; 32]).unwrap(),
            Some(Derived::Document(document.clone()))
        );
        assert_eq!(later.get(&[2; 32]).unwrap(), Some(Derived::Nothing));
        assert_eq!(later.get(&[3; 32]).unwrap(), None);
        assert_eq!(later.get(&[4; 32]).unwrap(), None);
        assert_eq!(later_held.size(&document).unwrap(), Some(7));
    }

    #[test]
    fn only_reusable_units_are_recorded() {
        let files = MemoryFiles::default();
        let held = MemoryHoldings::default();
        let records = Records::new(&files, &held);
        let (kept, broken) = (unit("a", 1, true), unit("b", 2, false));
        assert_eq!(records.write(&[&kept, &broken], 2).unwrap(), 1);
        let key = |unit: &UnitResult| unit.key.unwrap();
        assert_eq!(
            records.record("a", &key(&kept)).unwrap(),
            Some(Found::Recorded(Record::default()))
        );
        assert_eq!(records.record("b", &key(&broken)).unwrap(), None);
    }

    #[test]
    fn listed_records_look_only_for_the_units_the_listing_names() {
        let files = MemoryFiles::default();
        let held = MemoryHoldings::default();
        let (a, b) = (unit("a", 1, true), unit("b", 2, true));
        Records::new(&files, &held).write(&[&a, &b], 2).unwrap();

        let listed = Records::listed(&files, &held, &[unit_file(&UnitKey([1; 32]))]);
        assert!(listed.record("a", &UnitKey([1; 32])).unwrap().is_some());
        assert_eq!(listed.record("b", &UnitKey([2; 32])).unwrap(), None);
    }

    #[test]
    fn a_record_read_is_not_written_again() {
        let files = MemoryFiles::default();
        let held = MemoryHoldings::default();
        let first = Records::new(&files, &held);
        let shared = unit("shared", 3, true);
        assert_eq!(first.write(&[&shared], 1).unwrap(), 1);

        let later = Records::new(&files, &held);
        assert!(later.record("shared", &UnitKey([3; 32])).unwrap().is_some());
        let new = unit("new", 4, true);
        assert_eq!(later.write(&[&shared, &new], 1).unwrap(), 1);
    }

    #[test]
    fn the_last_run_of_the_region_comes_first() {
        let files = MemoryFiles::default();
        let held = MemoryHoldings::default();
        let records = Records::new(&files, &held);
        let elsewhere = unit("shared", 3, true);
        records.write(&[&elsewhere], 1).unwrap();

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
