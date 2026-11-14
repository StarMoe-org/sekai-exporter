//! Converters whose assets are built from game data tables.
//!
//! Such an asset has no root object. It is built from table rows and from assets the version
//! already holds, which it reads through a [`TableContext`].
//!
//! A converter plans the assets of a version from the tables, and then builds each of them on its
//! own. An asset reads the version only through the context it is built with, and the state of
//! its plan only through [`Shared`] maps, so that the reads of each asset are known: an asset of a
//! [reusable](TablePlan::reusable) plan whose reads give what they gave before keeps what it built
//! then.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::str::FromStr;

use sekai_format::AssetDocument;
use serde::Serialize;

use crate::{Asset, ConvertError};

/// The error code of a failure to read the store, which fails the run and not the asset.
pub const STORE_FAILED: &str = "store_failed";

/// An asset of the version, as table converters find it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetEntry {
    /// The name of the asset.
    pub name: String,
    /// The asset form.
    pub form: &'static str,
}

/// What a table converter reads through.
pub trait TableContext: Sync {
    /// The bytes of the table `name`.
    fn table(&self, name: &str) -> Result<Vec<u8>, ConvertError>;

    /// The bundles that hold assets of the version, in name order.
    fn bundles(&self) -> Vec<&str>;

    /// The assets made from the objects of `bundle`, in name order. Assets that could not be
    /// produced are left out.
    fn assets(&self, bundle: &str) -> &[AssetEntry];

    /// The document of the asset `name`.
    fn document(&self, name: &str) -> Result<AssetDocument, ConvertError>;

    /// A text that names the document of the asset `name` within the version: two assets with
    /// the same identity have the same document. It counts as a read of the document, so a
    /// converter keeps what it read of an asset under its identity, and not under its name. This
    /// context gives the name itself.
    fn identity(&self, name: &str) -> Result<String, ConvertError> {
        Ok(name.to_owned())
    }

    /// The bytes of the member `role` of the asset `name`.
    fn member(&self, name: &str, role: &str) -> Result<Vec<u8>, ConvertError>;

    /// The bytes of the member `role` of each asset of `names`, in that order. A context may read
    /// them at once; this one reads one after another.
    fn members(&self, names: &[&str], role: &str) -> Vec<Result<Vec<u8>, ConvertError>> {
        names.iter().map(|name| self.member(name, role)).collect()
    }

    /// Notes that the asset being built read the entry `key` of the [`Shared`] map `map` of its
    /// plan. A context that does not follow the reads of an asset ignores it.
    fn used(&self, _map: &str, _key: &str) {}
}

/// A map that a plan derives from the tables and shares among its assets, which read it by key
/// through the context they are built with.
#[derive(Debug, Clone)]
pub struct Shared<K, V> {
    name: &'static str,
    entries: BTreeMap<K, V>,
}

impl<K: Ord + Display, V> Shared<K, V> {
    /// The map `name` of a plan, with `entries`.
    pub fn new(name: &'static str, entries: BTreeMap<K, V>) -> Self {
        Self { name, entries }
    }

    /// The entry `key`, read by the asset that `cx` builds.
    pub fn get(&self, key: &K, cx: &dyn TableContext) -> Option<&V> {
        cx.used(self.name, &key.to_string());
        self.entries.get(key)
    }
}

/// A [`Shared`] map, as the table stage reads it to check the reads of an asset again.
pub trait SharedEntries: Sync {
    /// The name of the map.
    fn name(&self) -> &'static str;

    /// The entry with the key written as `key`, as JSON. An error means that the entry cannot
    /// be compared with what it was.
    fn entry(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
}

impl<K, V> SharedEntries for Shared<K, V>
where
    K: Ord + Display + FromStr + Sync,
    V: Serialize + Sync,
{
    fn name(&self) -> &'static str {
        self.name
    }

    fn entry(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let key = key
            .parse()
            .map_err(|_| format!("{}: the key {key} does not read back", self.name))?;
        let value = self.entries.get(&key);
        let json = value.map(serde_json::to_vec).transpose();
        json.map_err(|e| format!("{}: {e}", self.name))
    }
}

/// An asset built from tables.
#[derive(Debug, Clone)]
pub struct TableAsset {
    /// The members of the asset.
    pub asset: Asset,
    /// The names of the assets it refers to, in any order.
    pub references: Vec<String>,
}

/// The assets one table converter builds for a version.
pub trait TablePlan: Sync {
    /// The names of the assets.
    fn names(&self) -> &[String];

    /// Builds the asset with the name at `index` of [`TablePlan::names`], reading the version
    /// through `cx`.
    fn convert(&self, index: usize, cx: &dyn TableContext) -> Result<TableAsset, ConvertError>;

    /// Whether [`TablePlan::convert`] reads the state of the plan only through the maps of
    /// [`TablePlan::shared`], so that an asset whose reads give what they gave before may keep
    /// what it built then.
    fn reusable(&self) -> bool {
        false
    }

    /// The [`Shared`] maps of the plan.
    fn shared(&self) -> Vec<&dyn SharedEntries> {
        Vec::new()
    }
}

/// A converter whose assets are built from game data tables.
pub trait TableConverter: Send + Sync {
    /// The asset form.
    fn name(&self) -> &'static str;

    /// The version of the form.
    fn version(&self) -> u32;

    /// Reads the tables and lists the assets. An error fails the run.
    fn plan(&self, cx: &dyn TableContext) -> Result<Box<dyn TablePlan>, ConvertError>;
}
