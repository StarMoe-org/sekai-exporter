//! The bundles of a client package as the source of an export or of an update.

use std::io;
use std::path::Path;
use std::sync::Arc;

use sekai_game::client::{PackageBundle, bundles_from};
use sekai_layouts::{PlayerLayouts, ScriptTable, TableError, VersionTable};
use sekai_pipeline::BundleInput;
use sekai_pipeline::stream::{Entry, Inputs};
use sha2::{Digest, Sha256};

use crate::export::Source;

/// The classes whose objects in a client package are assets even when no container path names
/// them: the player data holds many sprites that the client shows without loading them by path,
/// such as the default frames of honors.
const LOOSE: &[&str] = &["Sprite"];

/// A client package as its bundles, with the layouts their files are read with.
pub(crate) struct Package {
    bundles: Bundles,
    layouts: PlayerLayouts,
}

/// The bundles of a client package, the player data first, as [`bundles_from`] gives them.
struct Bundles(Vec<Held>);

/// A bundle of a client package.
struct Held {
    name: &'static str,
    bytes: Arc<Vec<u8>>,
    sha256: [u8; 32],
}

impl Held {
    fn new(bundle: PackageBundle) -> Self {
        Self {
            name: bundle.name,
            sha256: Sha256::digest(&bundle.bytes).into(),
            bytes: Arc::new(bundle.bytes),
        }
    }
}

impl Bundles {
    /// The bundle named `name`.
    fn get(&self, name: &str) -> Option<&Held> {
        self.0.iter().find(|bundle| bundle.name == name)
    }

    /// The manifest of the package: each of its bundles.
    fn manifest(&self) -> Vec<Entry> {
        let entry = |bundle: &Held| Entry {
            name: bundle.name.to_owned(),
            size: bundle.bytes.len() as u64,
            content: None,
            dependencies: Vec::new(),
        };
        self.0.iter().map(entry).collect()
    }
}

impl Inputs for Bundles {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        let bundle = self.get(name).ok_or_else(|| {
            let message = format!("the package has no bundle {name}");
            io::Error::new(io::ErrorKind::NotFound, message)
        })?;
        Ok(BundleInput {
            name: name.to_owned(),
            source: bundle.bytes.clone(),
            sha256: bundle.sha256,
        })
    }
}

/// The table in the file at `path`.
fn read_table<T>(
    path: &Path,
    parse: impl FnOnce(&[u8]) -> Result<T, TableError>,
) -> Result<T, String> {
    let read = std::fs::read(path).map_err(|e| e.to_string());
    read.and_then(|bytes| parse(&bytes).map_err(|e| e.to_string()))
        .map_err(|e| format!("{}: {e}", path.display()))
}

impl Package {
    /// Reads the client package at `package`, whose engine classes have the layouts of the TPK
    /// file at `classes` and whose scripts have those of the table at `scripts`.
    pub(crate) fn open(
        package: &Path,
        classes: &Path,
        scripts: Option<&Path>,
    ) -> Result<Self, String> {
        let classes = read_table(classes, VersionTable::from_tpk)?;
        let scripts = scripts
            .map(|path| read_table(path, ScriptTable::from_jsonl_gz))
            .transpose()?;
        let bundles = std::fs::File::open(package)
            .and_then(bundles_from)
            .map_err(|e| format!("{}: {e}", package.display()))?;
        Ok(Self {
            bundles: Bundles(bundles.into_iter().map(Held::new).collect()),
            layouts: PlayerLayouts::new(classes, scripts),
        })
    }

    /// The package as the source of an export.
    pub(crate) fn into_source(self) -> Source<'static> {
        Source {
            manifest: self.bundles.manifest(),
            inputs: Box::new(self.bundles),
            classes: Box::new(self.layouts),
            loose: LOOSE,
        }
    }
}

/// Reads the client package at `package` as the source of an export, as [`Package::open`] does.
pub(crate) fn source(
    package: &Path,
    classes: &Path,
    scripts: Option<&Path>,
) -> Result<Source<'static>, lexopt::Error> {
    Ok(Package::open(package, classes, scripts)?.into_source())
}

#[cfg(feature = "cdn")]
mod update;
#[cfg(feature = "cdn")]
pub(crate) use update::update_source;
