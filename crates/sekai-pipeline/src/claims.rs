//! Which unit owns each asset name.
//!
//! Every asset a unit gives claims its name. A name that exactly one asset claims is owned by the
//! unit of that asset; a name that several assets claim is owned by none, and those assets are
//! left unnamed (see [`crate::stream`]). Ownership therefore depends on every unit, not only on the
//! one that gave an asset.
//!
//! A run that keeps what some units gave changes only the claims of the other units. The owner of
//! a name can change only if one of those claimed it before or claims it now, so [`advance`] rules
//! on those names alone, and the claims of every other name stay as they were.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// An asset that claims a name: the unit that gave it, its form, and whether it failed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Claimant {
    /// The unit that gave the asset.
    pub unit: String,
    /// The asset form.
    pub form: String,
    /// Whether the asset could not be produced.
    pub failed: bool,
}

/// The assets that claim one name, in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// The name.
    pub name: String,
    /// The assets that claim it, in order. There is at least one.
    pub claimants: Vec<Claimant>,
}

impl Claims {
    /// The asset that owns the name: the only one that claims it.
    pub fn owner(&self) -> Option<&Claimant> {
        match self.claimants.as_slice() {
            [owner] => Some(owner),
            _ => None,
        }
    }
}

/// How the owner of a name may have changed: the asset that owned it before a run and the asset
/// that owns it after. A ruling is given for each name that a unit not kept claimed before or
/// claims now, also when the owner stays the same, since the asset it gave may have changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ruling {
    /// The name.
    pub name: String,
    /// The owner before the run.
    pub before: Option<Claimant>,
    /// The owner after the run.
    pub after: Option<Claimant>,
}

/// The claims after a run that keeps the claims in `old` of the units `kept`, and in place of
/// those of every other unit has the claims `new`, with a ruling for each name the owner of which
/// may have changed. `old` is in name order with no name twice, and so are the claims returned. A
/// name that no unit claims at all is neither among the claims nor ruled on.
pub fn advance(
    old: Vec<Claims>,
    kept: &BTreeSet<&str>,
    new: impl IntoIterator<Item = (String, Claimant)>,
) -> (Vec<Claims>, Vec<Ruling>) {
    let mut added: BTreeMap<String, Vec<Claimant>> = BTreeMap::new();
    for (name, claimant) in new {
        added.entry(name).or_default().push(claimant);
    }
    let mut added = added.into_iter().peekable();
    let mut advanced = Advanced::default();
    for old in old {
        while let Some((name, claimants)) = added.next_if(|(name, _)| *name < old.name) {
            advanced.settle(name, None, claimants);
        }
        let adds = added.next_if(|(name, _)| *name == old.name).map(|(_, c)| c);
        let kept_all = old
            .claimants
            .iter()
            .all(|claimant| kept.contains(claimant.unit.as_str()));
        if kept_all && adds.is_none() {
            advanced.claims.push(old);
            continue;
        }
        let before = old.owner().cloned();
        let Claims { name, claimants } = old;
        let mut claimants: Vec<Claimant> = claimants
            .into_iter()
            .filter(|claimant| kept.contains(claimant.unit.as_str()))
            .collect();
        claimants.extend(adds.into_iter().flatten());
        advanced.settle(name, before, claimants);
    }
    for (name, claimants) in added {
        advanced.settle(name, None, claimants);
    }
    (advanced.claims, advanced.rulings)
}

/// The claims and rulings of [`advance`], as it goes through the names in order.
#[derive(Default)]
struct Advanced {
    claims: Vec<Claims>,
    rulings: Vec<Ruling>,
}

impl Advanced {
    /// Settles the name `name`, which `before` owned and `claimants` now claim.
    fn settle(&mut self, name: String, before: Option<Claimant>, mut claimants: Vec<Claimant>) {
        claimants.sort();
        let after = Claims { name, claimants };
        let owner = after.owner().cloned();
        if before.is_some() || owner.is_some() {
            self.rulings.push(Ruling {
                name: after.name.clone(),
                before,
                after: owner,
            });
        }
        if !after.claimants.is_empty() {
            self.claims.push(after);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claimant(unit: &str) -> Claimant {
        Claimant {
            unit: unit.to_owned(),
            form: "texture".to_owned(),
            failed: false,
        }
    }

    /// The claims that `assets`, pairs of a name and a unit, make.
    fn claims_of(assets: &[(String, String)]) -> Vec<Claims> {
        let mut by_name: BTreeMap<String, Vec<Claimant>> = BTreeMap::new();
        for (name, unit) in assets {
            by_name
                .entry(name.clone())
                .or_default()
                .push(claimant(unit));
        }
        by_name
            .into_iter()
            .map(|(name, mut claimants)| {
                claimants.sort();
                Claims { name, claimants }
            })
            .collect()
    }

    fn owners(claims: &[Claims]) -> BTreeMap<&str, &str> {
        claims
            .iter()
            .filter_map(|c| Some((c.name.as_str(), c.owner()?.unit.as_str())))
            .collect()
    }

    #[test]
    fn a_name_two_units_claim_has_no_owner_until_one_lets_go() {
        let old = claims_of(&[("a".into(), "u".into()), ("a".into(), "v".into())]);
        assert_eq!(owners(&old), BTreeMap::new());
        let kept = BTreeSet::from(["u"]);
        let (claims, rulings) = advance(old, &kept, [("b".to_owned(), claimant("v"))]);
        assert_eq!(owners(&claims), BTreeMap::from([("a", "u"), ("b", "v")]));
        let ruled: Vec<_> = rulings
            .iter()
            .map(|r| (r.name.as_str(), r.before.clone(), r.after.clone()))
            .collect();
        assert_eq!(
            ruled,
            [
                ("a", None, Some(claimant("u"))),
                ("b", None, Some(claimant("v")))
            ]
        );
    }

    /// A small pseudo-random sequence, so that the test is the same on every run.
    struct Random(u64);

    impl Random {
        fn below(&mut self, n: u64) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (self.0 >> 33) % n
        }

        fn assets(&mut self, units: &[&str], count: u64) -> Vec<(String, String)> {
            (0..count)
                .map(|_| {
                    let name = format!("n{}", self.below(12));
                    let unit = units[self.below(units.len() as u64) as usize].to_owned();
                    (name, unit)
                })
                .collect()
        }
    }

    #[test]
    fn advancing_gives_the_claims_and_owners_of_the_whole_run() {
        let units = ["a", "b", "c", "d", "e"];
        let mut random = Random(7);
        for _ in 0..500 {
            let count = random.below(16);
            let before = random.assets(&units, count);
            let kept: BTreeSet<&str> = units
                .iter()
                .copied()
                .filter(|_| random.below(2) == 0)
                .collect();
            let count = random.below(8);
            let new: Vec<(String, String)> = random
                .assets(&units, count)
                .into_iter()
                .filter(|(_, unit)| !kept.contains(unit.as_str()))
                .collect();
            let mut after = before.clone();
            after.retain(|(_, unit)| kept.contains(unit.as_str()));
            after.extend(new.iter().cloned());

            let old = claims_of(&before);
            let whole = claims_of(&after);
            let pairs = new
                .iter()
                .map(|(name, unit)| (name.clone(), claimant(unit)));
            let (claims, rulings) = advance(old.clone(), &kept, pairs);
            assert_eq!(claims, whole);

            // Every name whose owner changed, or whose owner is not kept, is ruled on, with the
            // owners before and after.
            let (was, is) = (owners(&old), owners(&whole));
            let unit = |c: &Option<Claimant>| c.as_ref().map(|c| c.unit.clone());
            for ruled in &rulings {
                let name = ruled.name.as_str();
                assert_eq!(unit(&ruled.before).as_deref(), was.get(name).copied());
                assert_eq!(unit(&ruled.after).as_deref(), is.get(name).copied());
            }
            let names: BTreeSet<&str> = was.keys().chain(is.keys()).copied().collect();
            for name in names {
                let (before, after) = (was.get(name), is.get(name));
                let renewed_owner = [before, after]
                    .into_iter()
                    .flatten()
                    .any(|unit| !kept.contains(unit));
                if before != after || renewed_owner {
                    assert!(
                        rulings.iter().any(|r| r.name == name),
                        "{name} is not ruled on"
                    );
                }
            }
        }
    }
}
