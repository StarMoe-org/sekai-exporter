//! What a run changes in the views of its region.
//!
//! A run evaluates only the units it does not keep, and the rulings on the names their assets
//! claim ([`sekai_pipeline::claims`]) say which assets the views gain, lose or replace. The
//! canonical view changes at those assets and at the assets built from tables. The storage view
//! changes in the directories of the units the rulings name (see [`crate::storage`]).
//!
//! Each change also says what it follows from ([`Causes`]): the sources, when a bundle of the
//! window of a unit it touches holds other content than in the last run, or a table changed; and
//! only the conversion, when the sources are the same.

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap};

use sekai_catalog::{Key, Objects};
use sekai_format::version_history::Cause;
use sekai_pipeline::claims::Ruling;
use sekai_pipeline::reuse::{Found, Record, Recorded, UnitKey, UnitResult};
use sekai_pipeline::state::{BundleState, RegionState};
use sekai_pipeline::stream::Content;
use sekai_publish::Binding;
use sekai_publish::patch::{Change, Lookup};

use crate::storage::Directories;

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
    /// What the changes of the run follow from.
    pub(super) causes: &'a Causes,
}

/// What the changes of a run follow from.
pub(super) struct Causes {
    /// What the change of each unit the run did not keep follows from, by unit.
    units: HashMap<String, Option<Cause>>,
    /// What the change of every asset built from tables follows from.
    tables: Cause,
}

impl Causes {
    /// The causes of a run that gave `results`, when the bundles of the last run were `before`
    /// and those of this one are `after`. A unit follows from the contents of the bundles of its
    /// window: its sources changed when one of them holds other content, and only the conversion
    /// when every one holds the same. Its cause is not known when the content of one is not. The
    /// assets built from tables follow only from the conversion when `same_content`, the version
    /// has the bundles and tables of the last one.
    pub(super) fn new(
        before: &[BundleState],
        after: &[BundleState],
        results: &[UnitResult],
        same_content: bool,
    ) -> Self {
        let (before, after) = (contents(before), contents(after));
        let bundle = |name: &String| {
            let content = after.get(name.as_str())?;
            Some(cause(before.get(name.as_str()) == Some(content)))
        };
        let units = results.iter().map(|result| {
            let cause = result.window.iter().map(bundle).reduce(Cause::of_both);
            (result.unit.clone(), cause.flatten())
        });
        Self {
            units: units.collect(),
            tables: cause(same_content),
        }
    }
}

/// What a change follows from when its sources are `same` as before.
fn cause(same: bool) -> Cause {
    if same {
        Cause::Conversion
    } else {
        Cause::Source
    }
}

/// The content of each of `bundles`, by name.
fn contents(bundles: &[BundleState]) -> HashMap<&str, &Content> {
    let pairs = bundles
        .iter()
        .map(|bundle| (bundle.name.as_str(), &bundle.content));
    pairs.collect()
}

impl Advance<'_> {
    /// What the change of `unit` follows from: as the run found for a unit it did not keep, only
    /// the conversion for one it kept, and the sources for one the version no longer has.
    pub(super) fn unit_cause(&self, unit: &str) -> Option<Cause> {
        match self.causes.units.get(unit) {
            Some(cause) => *cause,
            None if self.key(unit).is_ok() => Some(Cause::Conversion),
            None => Some(Cause::Source),
        }
    }

    /// What the change of each asset of the canonical view follows from, by name: the changes
    /// of the units that own its name before and after the run, or, for an asset that no ruling
    /// names, one built from tables, the change of those.
    pub(super) fn asset_cause(&self) -> impl Fn(&str) -> Option<Cause> + '_ {
        let ruled: HashMap<&str, Option<Cause>> = self
            .rulings
            .iter()
            .map(|ruling| {
                let owners = [&ruling.before, &ruling.after].into_iter().flatten();
                let causes = owners.map(|owner| self.unit_cause(&owner.unit));
                (
                    ruling.name.as_str(),
                    causes.reduce(Cause::of_both).flatten(),
                )
            })
            .collect();
        move |name| match ruled.get(name) {
            Some(cause) => *cause,
            None => Some(self.causes.tables),
        }
    }

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

    /// The units whose directories of the storage view the run changes, each with what its
    /// change follows from, and every produced asset they own after it. The document of an owner
    /// that no ruling names is the one the previous publication of the canonical view,
    /// `previous`, has.
    pub(super) fn storage_assets(
        &self,
        previous: &Lookup<'_>,
        owners: &BTreeMap<String, Key>,
    ) -> Result<(Directories<'_>, Vec<Binding>), String> {
        let ruled = self.rulings.iter();
        let claimants = ruled.flat_map(|ruling| [&ruling.before, &ruling.after]);
        let units: Directories<'_> = claimants
            .flatten()
            .map(|claimant| (claimant.unit.as_str(), self.unit_cause(&claimant.unit)))
            .collect();
        let mut assets = Vec::new();
        for claims in &self.state.claims {
            let Some(owner) = claims.owner() else {
                continue;
            };
            if owner.failed || !units.contains_key(owner.unit.as_str()) {
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
/// where `cause` tells what the change of each asset follows from, with the asset documents read
/// from `objects` on up to `threads` threads.
pub(super) fn canonical_change(
    mut old: Vec<Binding>,
    mut new: Vec<Binding>,
    cause: impl Fn(&str) -> Option<Cause>,
    objects: &Objects<'_>,
    threads: usize,
) -> Result<Change, String> {
    old.sort_by(|a, b| a.name.cmp(&b.name));
    new.sort_by(|a, b| a.name.cmp(&b.name));
    let change = sekai_publish::plan::change(&old, &new, cause, objects, threads);
    change.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use sekai_pipeline::reuse::Record;

    use super::*;

    fn bundle(name: &str, hash: &str) -> BundleState {
        BundleState {
            name: name.to_owned(),
            content: Content::Declared(hash.to_owned()),
            files: Vec::new(),
        }
    }

    fn result(unit: &str, window: &[&str]) -> UnitResult {
        UnitResult {
            unit: unit.to_owned(),
            key: None,
            window: window.iter().map(|&bundle| bundle.to_owned()).collect(),
            reusable: false,
            record: Record::default(),
        }
    }

    #[test]
    fn a_unit_follows_from_the_contents_of_the_bundles_of_its_window() {
        let before = [bundle("a", "1"), bundle("b", "2"), bundle("c", "3")];
        let after = [bundle("a", "1"), bundle("b", "2"), bundle("c", "4")];
        let results = [
            result("a", &["a", "b"]),
            result("c", &["c", "a"]),
            result("d", &["d"]),
            result("e", &["a", "e"]),
        ];
        let causes = Causes::new(&before, &after, &results, false);
        assert_eq!(causes.units["a"], Some(Cause::Conversion));
        assert_eq!(causes.units["c"], Some(Cause::Source));
        // The content of `d` and `e` is not known: one bundle of `e` is the same, so its sources
        // may or may not have changed.
        assert_eq!(causes.units["d"], None);
        assert_eq!(causes.units["e"], None);
        assert_eq!(causes.tables, Cause::Source);

        let with_new = [bundle("a", "1"), bundle("f", "5")];
        let causes = Causes::new(&before, &with_new, &[result("f", &["f", "a"])], true);
        assert_eq!(causes.units["f"], Some(Cause::Source));
        assert_eq!(causes.tables, Cause::Conversion);
    }
}
