//! An account of what became of every object and resource file of a version.

use std::collections::{BTreeMap, BTreeSet};

use sekai_convert::Converter;
use sekai_format::AssetDocument;

use crate::version::Version;

/// What became of the contents of one bundle. Every object counts once, as the first of root,
/// read, ignored and unknown that applies to it.
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

/// The ledger of each bundle of `version`, in bundle order. `documents` are the assets evaluated
/// with `converters`, and `ignored` names the classes and script classes that carry no asset
/// content.
pub fn account(
    version: &Version<'_>,
    converters: &[Box<dyn Converter>],
    documents: &[AssetDocument],
    ignored: &[&str],
) -> Vec<BundleLedger> {
    let index = version.index();
    let read: BTreeSet<(&str, i64)> = documents
        .iter()
        .flat_map(|document| &document.sources)
        .map(|source| (source.bundle.as_str(), source.path_id))
        .collect();
    let resources: BTreeSet<(&str, &str)> = documents
        .iter()
        .flat_map(|document| &document.resources)
        .map(|resource| (resource.bundle.as_str(), resource.file.as_str()))
        .collect();
    let mut ledgers: Vec<BundleLedger> = index
        .bundles()
        .iter()
        .enumerate()
        .map(|(b, bundle)| BundleLedger {
            bundle: bundle.name.clone(),
            objects: 0,
            roots: 0,
            read: 0,
            ignored: 0,
            unknown: 0,
            unknown_classes: BTreeMap::new(),
            resources: version.resource_hashes(b).len() as u64,
            resources_read: resources
                .iter()
                .filter(|(name, _)| *name == bundle.name)
                .count() as u64,
        })
        .collect();
    for (id, object, paths) in version.objects() {
        let ledger = &mut ledgers[id.bundle];
        ledger.objects += 1;
        let script = index
            .position(id)
            .and_then(|position| index.script(id.bundle, id.file, position))
            .flatten();
        if converters
            .iter()
            .any(|converter| converter.is_root(object, script, paths))
        {
            ledger.roots += 1;
        } else if read.contains(&(ledger.bundle.as_str(), id.path_id)) {
            ledger.read += 1;
        } else if ignored.contains(&object.class.as_str())
            || script.is_some_and(|s| ignored.contains(&script_class(s)))
        {
            ledger.ignored += 1;
        } else {
            ledger.unknown += 1;
            *ledger
                .unknown_classes
                .entry(object.class.clone())
                .or_default() += 1;
        }
    }
    ledgers
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
