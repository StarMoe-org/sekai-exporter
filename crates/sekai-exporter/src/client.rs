//! The player data of a client package as the source of an export.

use std::io;
use std::path::Path;
use std::sync::Arc;

use sekai_game::client::{PLAYER_BUNDLE, player_bundle};
use sekai_layouts::VersionTable;
use sekai_pipeline::BundleInput;
use sekai_pipeline::stream::{Entry, Inputs};
use sekai_unity::ByteSource;
use sha2::{Digest, Sha256};

use crate::export::Source;

/// The one bundle of a client package.
struct Player {
    source: Arc<dyn ByteSource>,
    sha256: [u8; 32],
}

impl Inputs for Player {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        Ok(BundleInput {
            name: name.to_owned(),
            source: self.source.clone(),
            sha256: self.sha256,
        })
    }
}

/// Reads the client package at `package`, whose engine classes have the layouts of the TPK file
/// at `table`.
pub(crate) fn source(package: &Path, table: &Path) -> Result<Source<'static>, lexopt::Error> {
    let named = |path: &Path, message: String| format!("{}: {message}", path.display());
    let classes = std::fs::read(table)
        .map_err(|e| e.to_string())
        .and_then(|bytes| VersionTable::from_tpk(&bytes).map_err(|e| e.to_string()))
        .map_err(|e| named(table, e))?;
    let bundle = std::fs::read(package)
        .and_then(|bytes| player_bundle(&bytes))
        .map_err(|e| named(package, e.to_string()))?;
    let manifest = vec![Entry {
        name: PLAYER_BUNDLE.to_owned(),
        size: bundle.len() as u64,
        dependencies: Vec::new(),
    }];
    let player = Player {
        sha256: Sha256::digest(&bundle).into(),
        source: Arc::new(bundle),
    };
    Ok(Source {
        manifest,
        inputs: Box::new(player),
        classes: Box::new(classes),
    })
}
