//! What a run changes in the views of its region.
//!
//! A run evaluates only the units it does not keep, and the rulings on the names their assets
//! claim ([`sekai_pipeline::claims`]) say which assets the views gain, lose or replace. The
//! canonical view changes at those assets and at the assets built from tables. The storage view
//! changes in the directories of the units the rulings name (see [`crate::storage`]).

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use sekai_catalog::{Key, Objects};
use sekai_pipeline::claims::Ruling;
use sekai_pipeline::reuse::{Found, Record, Recorded, UnitKey, UnitResult};
use sekai_pipeline::state::RegionState;
use sekai_publish::Binding;
use sekai_publish::patch::{Change, Lookup};

/// What a run gave, as its views need it.
pub(super) struct Advance<'a> {
    /// A ruling on each name whose owner may have changed.
    pub(super) rulings: &'a [Ruling],
    /// The state of the region after the run.
    pub(super) state: &'a RegionState,
    /// The result of each unit the run did not keep.
    pub(super) results: &'a [UnitResult],
    /// Where the records of the units the run kept are found.
    pub(super) records: &'a dyn Recorded,
}

impl Advance<'_> {
    /// The asset document of the owner of each ruled name that has an owner after the run, by
    /// name. An owner from a unit the run kept is found in the record of the unit.
    pub(super) fn owners(&self) -> Result<BTreeMap<String, Key>, String> {
        let evaluated: HashMap<&str, &Record> = self
            .results
            .iter()
            .map(|result| (result.unit.as_str(), &result.record))
            .collect();
        let mut kept: HashMap<&str, Record> = HashMap::new();
        let mut owners = BTreeMap::new();
        for ruling in self.rulings {
            let Some(owner) = &ruling.after else {
                continue;
            };
            let unit = owner.unit.as_str();
            let record = match evaluated.get(unit) {
                Some(record) => *record,
                None => match kept.entry(unit) {
                    Entry::Occupied(entry) => &*entry.into_mut(),
                    Entry::Vacant(entry) => &*entry.insert(self.kept_record(unit)?),
                },
            };
            let document = record.document(&ruling.name).map_err(|e| e.to_string())?;
            let document =
                document.ok_or_else(|| format!("unit {unit} gave no asset {}", ruling.name))?;
            owners.insert(ruling.name.clone(), document);
        }
        Ok(owners)
    }

    /// The record of `unit`, a unit the run kept.
    fn kept_record(&self, unit: &str) -> Result<Record, String> {
        let key = self.key(unit)?;
        match self.records.record(unit, key).map_err(|e| e.to_string())? {
            Some(Found::Recorded(record)) => Ok(record),
            _ => Err(format!("the record of unit {unit} under {key} is missing")),
        }
    }

    /// The key of `unit` in the state.
    fn key(&self, unit: &str) -> Result<&UnitKey, String> {
        let units = &self.state.units;
        let found = units.binary_search_by(|state| state.unit.as_str().cmp(unit));
        found
            .map(|index| &units[index].key)
            .map_err(|_| format!("the state holds no unit {unit}"))
    }

    /// The bindings of the canonical view at the ruled names: before the run, as `previous`, the
    /// previous publication of the view, has them, and after it, as `owners` has them.
    pub(super) fn canonical_bindings(
        &self,
        previous: &Lookup<'_>,
        owners: &BTreeMap<String, Key>,
    ) -> Result<(Vec<Binding>, Vec<Binding>), String> {
        let before = self.rulings.iter().filter(|ruling| ruling.before.is_some());
        let old = previous
            .bindings(before.map(|ruling| ruling.name.as_str()))
            .map_err(|e| e.to_string())?;
        let new = owners
            .iter()
            .map(|(name, document)| Binding {
                name: name.clone(),
                document: document.clone(),
            })
            .collect();
        Ok((old, new))
    }

    /// The units whose directories of the storage view the run changes, and every produced
    /// asset they own after it. The document of an owner that no ruling names is the one the
    /// previous publication of the canonical view, `previous`, has.
    pub(super) fn storage_assets(
        &self,
        previous: &Lookup<'_>,
        owners: &BTreeMap<String, Key>,
    ) -> Result<(BTreeSet<&str>, Vec<Binding>), String> {
        let ruled = self.rulings.iter();
        let claimants = ruled.flat_map(|ruling| [&ruling.before, &ruling.after]);
        let units: BTreeSet<&str> = claimants
            .flatten()
            .map(|claimant| claimant.unit.as_str())
            .collect();
        let mut assets = Vec::new();
        for claims in &self.state.claims {
            let Some(owner) = claims.owner() else {
                continue;
            };
            if owner.failed || !units.contains(owner.unit.as_str()) {
                continue;
            }
            let document = match owners.get(&claims.name) {
                Some(document) => Some(document.clone()),
                None => previous.document(&claims.name).map_err(|e| e.to_string())?,
            };
            let document = document
                .ok_or_else(|| format!("the canonical view has no document of {}", claims.name))?;
            assets.push(Binding {
                name: claims.name.clone(),
                document,
            });
        }
        Ok((units, assets))
    }
}

/// The change to the canonical view from the bindings `old` to `new`, both at the same names,
/// with the asset documents read from `objects` on up to `threads` threads.
pub(super) fn canonical_change(
    mut old: Vec<Binding>,
    mut new: Vec<Binding>,
    objects: &Objects<'_>,
    threads: usize,
) -> Result<Change, String> {
    old.sort_by(|a, b| a.name.cmp(&b.name));
    new.sort_by(|a, b| a.name.cmp(&b.name));
    sekai_publish::plan::change(&old, &new, objects, threads).map_err(|e| e.to_string())
}
