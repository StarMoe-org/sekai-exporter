//! The player data of a client package as the source of an export or of an update.

use std::io;
use std::path::Path;
use std::sync::Arc;

use sekai_game::client::{PLAYER_BUNDLE, player_bundle_from};
use sekai_layouts::{PlayerLayouts, ScriptTable, TableError, VersionTable};
use sekai_pipeline::BundleInput;
use sekai_pipeline::stream::{Entry, Inputs};
use sha2::{Digest, Sha256};

use crate::export::Source;

/// A client package as one bundle, its player data, with the layouts its files are read with.
pub(crate) struct Package {
    player: Player,
    layouts: PlayerLayouts,
}

/// The one bundle of a client package.
struct Player {
    bytes: Arc<Vec<u8>>,
    sha256: [u8; 32],
}

impl Inputs for Player {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        Ok(BundleInput {
            name: name.to_owned(),
            source: self.bytes.clone(),
            sha256: self.sha256,
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
        let bundle = std::fs::File::open(package)
            .and_then(player_bundle_from)
            .map_err(|e| format!("{}: {e}", package.display()))?;
        let player = Player {
            sha256: Sha256::digest(&bundle).into(),
            bytes: Arc::new(bundle),
        };
        Ok(Self {
            player,
            layouts: PlayerLayouts::new(classes, scripts),
        })
    }

    /// The manifest of the package: its one bundle.
    fn manifest(&self) -> Vec<Entry> {
        vec![Entry {
            name: PLAYER_BUNDLE.to_owned(),
            size: self.player.bytes.len() as u64,
            content: None,
            dependencies: Vec::new(),
        }]
    }

    /// The package as the source of an export.
    pub(crate) fn into_source(self) -> Source<'static> {
        Source {
            manifest: self.manifest(),
            inputs: Box::new(self.player),
            classes: Box::new(self.layouts),
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
