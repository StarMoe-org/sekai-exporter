//! The `cp` and `nuverse` sources, which download from the distribution services.

mod cp;
mod crypto;
mod dir;
mod discover;
mod http;
mod manifest;
mod nuverse;
mod region;
mod tables;

use std::fmt;

use sekai_format::identity::VersionIdentity;

pub use crypto::Secrets;
pub use region::{Client, Kind, Network, Region, SecretNames, Urls};
pub use tables::{TableSource, Tables};

use crate::sink::Sink;

/// A failure of a source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError {
    /// The kind of failure: `config`, `secret`, `network`, `status`, `decode`, `length`,
    /// `no_version`, `read` or `sink`.
    pub code: &'static str,
    /// A description for people. It names secrets but never contains their values.
    pub message: String,
    /// The HTTP status, when the failure is one.
    pub status: Option<u16>,
}

impl SourceError {
    pub(crate) fn new(code: &'static str, message: impl fmt::Display) -> Self {
        Self {
            code,
            message: message.to_string(),
            status: None,
        }
    }
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for SourceError {}

/// One bundle of a manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleEntry {
    /// The bundle name.
    pub name: String,
    /// The hash the manifest declares, which decides whether the bundle is fetched again.
    pub declared: String,
    /// The directory of the download, for the sources that have one.
    pub download_path: Option<String>,
    /// The size the manifest declares.
    pub size: u64,
    /// The names of the bundles this one depends on.
    pub dependencies: Vec<String>,
}

/// A supplier of the current version of a region, its manifest and its bundles.
pub trait Source: Sync {
    /// Reads the current identity of the region. `previous` is the identity of the last complete
    /// run, which discovery can fall back to.
    fn identity(&self, previous: Option<&VersionIdentity>) -> Result<VersionIdentity, SourceError>;

    /// Reads the manifest of a version, in name order.
    fn manifest(&self, identity: &VersionIdentity) -> Result<Vec<BundleEntry>, SourceError>;

    /// Streams the bytes of a bundle into `sink`, and returns how many were read. `sink` is reset
    /// before a transfer is tried again.
    fn fetch(
        &self,
        identity: &VersionIdentity,
        entry: &BundleEntry,
        sink: &mut dyn Sink,
    ) -> Result<u64, SourceError>;
}

/// Creates the guest account of a region and stores its credential in the directory of secrets.
/// Returns the name of the secret. A secret that is already set is left alone.
pub fn create_account(region: Region, secrets: &Secrets) -> Result<String, SourceError> {
    if region.kind != Kind::Cp {
        return Err(SourceError::new(
            "config",
            "only a cp region has guest accounts",
        ));
    }
    cp::create_account(region, secrets)
}

/// Opens the source of `region`, reading its secrets from `secrets`.
pub fn open(region: Region, secrets: &Secrets) -> Result<Box<dyn Source>, SourceError> {
    if region.kind != Kind::Dir && region.urls.manifest.is_empty() {
        return Err(SourceError::new("config", "the region has no urls"));
    }
    Ok(match region.kind {
        Kind::Cp => Box::new(cp::Cp::new(region, secrets)?),
        Kind::Nuverse => Box::new(nuverse::Nuverse::new(region, secrets)?),
        Kind::Dir => {
            let directory = region
                .directory
                .ok_or_else(|| SourceError::new("config", "a dir region needs a directory"))?;
            Box::new(dir::Dir::new(directory))
        }
    })
}
