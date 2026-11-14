//! Results that later runs of any region may take, kept as state files: the record of a unit
//! under its key, and the output of a derivation under its fingerprint.
//!
//! A run reads a record only when it needs it, and writes the records of what it derived once
//! the objects they name are in the destination. Every object a record names is therefore there,
//! and a run knows the size of each object a derivation record names without asking.

use std::collections::HashMap;
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use sekai_catalog::{Derivations, Derived, Holdings, Key, MemoryHoldings, Store};
use serde::{Deserialize, Serialize};

use crate::eval::{hex, parallel};
use crate::reuse::{Record, Recorded, UnitKey, UnitResult};
use crate::state::StateFiles;

fn unit_file(key: &UnitKey) -> String {
    let key = key.to_string();
    format!("records/units/{}/{key}.json", &key[..2])
}

fn derivation_file(fingerprint: &[u8; 32]) -> String {
    let fingerprint = hex(fingerprint);
    format!(
        "records/derivations/{}/{fingerprint}.json",
        &fingerprint[..2]
    )
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
    /// The objects that the records read name, with their sizes.
    held: &'a MemoryHoldings,
    /// Where the objects of this run's derivations are, to size them.
    store: &'a dyn Store,
    /// The derivations read or made, by fingerprint.
    derivations: Mutex<HashMap<[u8; 32], Option<Derived>>>,
    /// The derivations made by this run, to write.
    made: Mutex<Vec<([u8; 32], Derived)>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn invalid(name: &str, e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{name}: {e}"))
}

impl<'a> Records<'a> {
    /// The records `files` keep. The objects the derivation records name are added to `held` as
    /// they are read, and the objects of new derivations are sized from `store`.
    pub fn new(files: &'a dyn StateFiles, held: &'a MemoryHoldings, store: &'a dyn Store) -> Self {
        Self {
            files,
            held,
            store,
            derivations: Mutex::default(),
            made: Mutex::default(),
        }
    }

    /// Writes the derivations this run made and the record of each of `units` that a later run
    /// may take, on up to `threads` threads. Call it once the objects they name are in the
    /// destination. Returns how many records were written.
    pub fn write(&self, units: &[&UnitResult], threads: usize) -> io::Result<usize> {
        let made = std::mem::take(&mut *lock(&self.made));
        let units: Vec<&UnitResult> = units.iter().copied().filter(|u| u.reusable).collect();
        let derivations = parallel(made.len(), threads, |i| {
            let (fingerprint, derived) = &made[i];
            self.write_derivation(fingerprint, derived)
        });
        let records = parallel(units.len(), threads, |i| self.write_unit(units[i]));
        derivations
            .into_iter()
            .chain(records)
            .collect::<io::Result<Vec<()>>>()
            .map(|written| written.len())
    }

    fn write_derivation(&self, fingerprint: &[u8; 32], derived: &Derived) -> io::Result<()> {
        let recorded = match derived {
            Derived::Document(key) => {
                let size = self.store.size(key)?.ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("{key} is not in the store"),
                    )
                })?;
                DerivationRecord::Document {
                    key: key.to_string(),
                    size,
                }
            }
            Derived::Nothing => DerivationRecord::Nothing,
        };
        let bytes = serde_json::to_vec(&recorded).map_err(io::Error::other)?;
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
        let recorded = serde_json::from_slice(&bytes).map_err(|e| invalid(&name, e))?;
        Ok(Some(match recorded {
            DerivationRecord::Document { key, size } => {
                let key: Key = key.parse().map_err(|e| invalid(&name, e))?;
                self.held.hold(&[(key.clone(), size)])?;
                Derived::Document(key)
            }
            DerivationRecord::Nothing => Derived::Nothing,
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
    fn record(&self, _unit: &str, key: &UnitKey) -> io::Result<Option<Record>> {
        let bytes = self.files.read(&unit_file(key))?;
        Ok(bytes.and_then(|bytes| Record::read(&bytes)))
    }
}

/// The results a run of a region takes: those of the last run of the region, and else the
/// records of every run.
pub struct Earlier<'a> {
    /// The result of each unit of the last version of the region, by name.
    units: HashMap<&'a str, &'a UnitResult>,
    records: &'a dyn Recorded,
}

impl<'a> Earlier<'a> {
    /// The results of `units`, the last run of the region, and of `records`.
    pub fn new(units: &'a [UnitResult], records: &'a dyn Recorded) -> Self {
        let units = units
            .iter()
            .map(|unit| (unit.unit.as_str(), unit))
            .collect();
        Self { units, records }
    }

    /// Whether the last run of the region gave the result of `unit`, under the same key.
    pub fn gave(&self, unit: &UnitResult) -> bool {
        let earlier = self.units.get(unit.unit.as_str());
        earlier.is_some_and(|earlier| earlier.reusable_key() == unit.reusable_key())
    }
}

impl Recorded for Earlier<'_> {
    fn record(&self, unit: &str, key: &UnitKey) -> io::Result<Option<Record>> {
        match self.units.get(unit) {
            Some(earlier) if earlier.reusable_key() == Some(key) => {
                Ok(Some(earlier.record.clone()))
            }
            _ => self.records.record(unit, key),
        }
    }
}

#[cfg(test)]
mod tests {
    use sekai_catalog::MemoryStore;

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
    fn derivations_are_written_with_the_sizes_of_their_objects() {
        let files = MemoryFiles::default();
        let store = MemoryStore::default();
        let document = store.put(b"{\"a\":1}", "json").unwrap();
        let held = MemoryHoldings::default();
        let records = Records::new(&files, &held, &store);
        records
            .put([1; 32], Derived::Document(document.clone()))
            .unwrap();
        records.put([2; 32], Derived::Nothing).unwrap();
        assert_eq!(
            records.get(&[1; 32]).unwrap(),
            Some(Derived::Document(document.clone()))
        );
        assert!(files.names().is_empty());
        assert_eq!(records.write(&[], 2).unwrap(), 2);

        let later_held = MemoryHoldings::default();
        let later = Records::new(&files, &later_held, &store);
        assert_eq!(
            later.get(&[1; 32]).unwrap(),
            Some(Derived::Document(document.clone()))
        );
        assert_eq!(later.get(&[2; 32]).unwrap(), Some(Derived::Nothing));
        assert_eq!(later.get(&[3; 32]).unwrap(), None);
        assert_eq!(later_held.size(&document).unwrap(), Some(7));
    }

    #[test]
    fn only_reusable_units_are_recorded() {
        let files = MemoryFiles::default();
        let store = MemoryStore::default();
        let held = MemoryHoldings::default();
        let records = Records::new(&files, &held, &store);
        let (kept, broken) = (unit("a", 1, true), unit("b", 2, false));
        assert_eq!(records.write(&[&kept, &broken], 2).unwrap(), 1);
        let key = |unit: &UnitResult| unit.key.unwrap();
        assert_eq!(
            records.record("a", &key(&kept)).unwrap(),
            Some(Record::default())
        );
        assert_eq!(records.record("b", &key(&broken)).unwrap(), None);
    }

    #[test]
    fn the_last_run_of_the_region_comes_first() {
        let files = MemoryFiles::default();
        let store = MemoryStore::default();
        let held = MemoryHoldings::default();
        let records = Records::new(&files, &held, &store);
        let elsewhere = unit("shared", 3, true);
        records.write(&[&elsewhere], 1).unwrap();

        let last = [unit("a", 1, true), unit("b", 2, false)];
        let earlier = Earlier::new(&last, &records);
        assert!(earlier.record("a", &UnitKey([1; 32])).unwrap().is_some());
        // A result that may not be taken, or one under another key, is not.
        assert!(earlier.record("b", &UnitKey([2; 32])).unwrap().is_none());
        assert!(earlier.record("a", &UnitKey([9; 32])).unwrap().is_none());
        // Another region recorded this one.
        assert!(
            earlier
                .record("shared", &UnitKey([3; 32]))
                .unwrap()
                .is_some()
        );
        assert!(earlier.gave(&last[0]));
        assert!(!earlier.gave(&unit("a", 9, true)));
    }
}
