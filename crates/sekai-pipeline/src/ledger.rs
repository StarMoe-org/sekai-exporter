//! An account of what became of every object and resource file of a bundle.

use std::collections::{BTreeMap, BTreeSet, HashSet};

/// What became of the contents of one bundle. Every object counts once, as the first of root,
/// read, ignored and unknown that applies to it. An object is read when an asset of any unit
/// whose window holds the bundle read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleLedger {
    /// The bundle name.
    pub bundle: String,
    /// The objects of the bundle.
    pub objects: u64,
    /// The objects a converter took as the root of an asset, whatever the outcome.
    pub roots: u64,
    /// The other objects a conversion read.
    pub read: u64,
    /// The objects of a class or script declared to carry no asset content.
    pub ignored: u64,
    /// The objects nothing accounts for.
    pub unknown: u64,
    /// The number of unknown objects of each class.
    pub unknown_classes: BTreeMap<String, u64>,
    /// The resource files of the bundle.
    pub resources: u64,
    /// The resource files a conversion read.
    pub resources_read: u64,
}

impl BundleLedger {
    /// Whether everything in the bundle is accounted for.
    pub fn complete(&self) -> bool {
        self.unknown == 0 && self.resources_read == self.resources
    }
}

/// The class a script of the form `"<assembly>/<namespace>.<class>"` defines.
fn script_class(script: &str) -> &str {
    script.rsplit(['/', '.']).next().unwrap_or(script)
}

/// The account of one bundle, kept while units can still read from it.
#[derive(Debug, Default)]
pub(crate) struct Tally {
    /// The counts, once the objects of the bundle are counted. Read, ignored and unknown objects
    /// are counted when the account is settled.
    ledger: Option<BundleLedger>,
    /// Each object that is not a root, as its path ID, its class and whether its class or script
    /// is ignored.
    rest: Vec<(i64, Box<str>, bool)>,
    /// The path IDs of the objects a conversion read.
    read: HashSet<i64>,
    /// The resource files a conversion read.
    resources_read: BTreeSet<String>,
}

impl Tally {
    /// Starts counting the objects of `bundle`, which has `resources` resource files.
    pub(crate) fn open(&mut self, bundle: &str, resources: u64) {
        self.ledger = Some(BundleLedger {
            bundle: bundle.to_owned(),
            objects: 0,
            roots: 0,
            read: 0,
            ignored: 0,
            unknown: 0,
            unknown_classes: BTreeMap::new(),
            resources,
            resources_read: 0,
        });
    }

    /// Counts an object of the bundle, of class `class` with the script `script`. `ignored` names
    /// the classes and script classes that carry no asset content.
    pub(crate) fn object(
        &mut self,
        path_id: i64,
        class: &str,
        script: Option<&str>,
        root: bool,
        ignored: &[&str],
    ) {
        let Some(ledger) = &mut self.ledger else {
            return;
        };
        ledger.objects += 1;
        if root {
            ledger.roots += 1;
        } else {
            let skipped = ignored.contains(&class)
                || script.is_some_and(|script| ignored.contains(&script_class(script)));
            self.rest.push((path_id, class.into(), skipped));
        }
    }

    /// Notes that a conversion read the object `path_id` of the bundle.
    pub(crate) fn read(&mut self, path_id: i64) {
        self.read.insert(path_id);
    }

    /// Notes that a conversion read the resource file `file` of the bundle.
    pub(crate) fn read_resource(&mut self, file: &str) {
        if !self.resources_read.contains(file) {
            self.resources_read.insert(file.to_owned());
        }
    }

    /// The ledger of the bundle, once no conversion can read from it any more. `None` if its
    /// objects were never counted.
    pub(crate) fn settle(self) -> Option<BundleLedger> {
        let mut ledger = self.ledger?;
        for (path_id, class, skipped) in self.rest {
            if self.read.contains(&path_id) {
                ledger.read += 1;
            } else if skipped {
                ledger.ignored += 1;
            } else {
                ledger.unknown += 1;
                *ledger.unknown_classes.entry(class.into()).or_default() += 1;
            }
        }
        ledger.resources_read = self.resources_read.len() as u64;
        Some(ledger)
    }
}

#[cfg(test)]
mod tests {
    use super::script_class;

    #[test]
    fn a_script_class_is_the_last_part_of_the_script() {
        assert_eq!(script_class("Assembly-CSharp/Sekai.Build.Data"), "Data");
        assert_eq!(script_class("Assembly-CSharp/.Data"), "Data");
        assert_eq!(script_class("Assembly-CSharp/Data"), "Data");
    }
}
