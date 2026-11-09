//! Converters whose assets are built from game data tables.
//!
//! Such an asset has no root object. It is built from table rows and from assets the version
//! already holds, which it reads through a [`TableContext`].

use sekai_format::AssetDocument;

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

    /// The bytes of the member `role` of the asset `name`.
    fn member(&self, name: &str, role: &str) -> Result<Vec<u8>, ConvertError>;
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

    /// Builds the asset with the name at `index` of [`TablePlan::names`].
    fn convert(&self, index: usize) -> Result<TableAsset, ConvertError>;
}

/// A converter whose assets are built from game data tables.
pub trait TableConverter: Send + Sync {
    /// The asset form.
    fn name(&self) -> &'static str;

    /// The version of the form.
    fn version(&self) -> u32;

    /// Reads the tables and lists the assets. An error fails the run.
    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError>;
}
