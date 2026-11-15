//! Where a region is published and where its state is kept: a bucket, or the output directory.

use std::path::{Path, PathBuf};

use sekai_catalog::{Key, NoRemote, Objects, Remote};
use sekai_format::identity::VersionIdentity;
use sekai_format::publication::Pointer;
use sekai_pipeline::state::{DirectoryFiles, StateFiles};
use sekai_publish::{Layout, Object};

use crate::bucket::Bucket;

/// Where a region is published and its state kept.
pub(super) enum Site<'a> {
    /// The output directory, with the state under `state/` and the views under `published/`.
    Local {
        state: DirectoryFiles,
        published: PathBuf,
    },
    /// A bucket, which keeps the state under its `state/`.
    Bucket(&'a Bucket),
}

/// The state file that holds the last version of `region`.
fn version_file(region: &str) -> String {
    format!("versions/{region}.json")
}

impl<'a> Site<'a> {
    /// The output directory `output` as the site.
    pub(super) fn local(output: &Path) -> Self {
        Self::Local {
            state: DirectoryFiles::new(output.join("state")),
            published: output.join("published"),
        }
    }

    /// The state files of the site.
    pub(super) fn files(&self) -> &dyn StateFiles {
        match self {
            Self::Local { state, .. } => state,
            Self::Bucket(bucket) => *bucket,
        }
    }

    /// Where objects that the local store lacks are read from.
    pub(super) fn remote(&self) -> &dyn Remote {
        match self {
            Self::Local { .. } => &NoRemote,
            Self::Bucket(bucket) => bucket.remote(),
        }
    }

    /// How many reads and writes of objects and state files run at once: the connections of a
    /// bucket, or `threads` for the output directory.
    pub(super) fn requests(&self, threads: usize) -> usize {
        match self {
            Self::Local { .. } => threads,
            Self::Bucket(bucket) => bucket.connections(),
        }
    }

    /// The layout of the canonical view.
    pub(super) fn layout(&self) -> Layout {
        match self {
            Self::Local { .. } => Layout::Index,
            Self::Bucket(bucket) => bucket.layout(),
        }
    }

    /// The current pointer of `target`.
    pub(super) fn current(&self, target: &str) -> Result<Option<Pointer>, String> {
        match self {
            Self::Local { published, .. } => {
                sekai_publish::current(published, target).map_err(|e| e.to_string())
            }
            Self::Bucket(bucket) => bucket.current(target),
        }
    }

    /// Puts the objects of `referred` where the site serves them, and records in the holdings of
    /// `objects` that the site holds them. The output directory serves its local store, which
    /// holds them already.
    pub(super) fn store(&self, objects: &Objects<'_>, referred: Vec<Object>) -> Result<(), String> {
        match self {
            Self::Local { .. } => {
                let held: Vec<(Key, u64)> = referred
                    .into_iter()
                    .map(|object| (object.key, object.size))
                    .collect();
                objects.held.hold(&held).map_err(|e| e.to_string())
            }
            Self::Bucket(bucket) => bucket.store(objects, referred),
        }
    }

    /// Makes `pointer` the current publication of `target`.
    pub(super) fn make_current(&self, target: &str, pointer: &Pointer) -> Result<(), String> {
        match self {
            Self::Local { published, .. } => {
                sekai_publish::make_current(published, target, pointer).map_err(|e| e.to_string())
            }
            Self::Bucket(bucket) => bucket.make_current(target, pointer),
        }
    }
}

/// The last version of `region` that a run recorded in `files`.
pub(super) fn last_version(
    files: &dyn StateFiles,
    region: &str,
) -> Result<Option<VersionIdentity>, String> {
    let failed = |e: String| format!("the last version of {region}: {e}");
    let bytes = files
        .read(&version_file(region))
        .map_err(|e| failed(e.to_string()))?;
    bytes
        .map(|bytes| serde_json::from_slice(&bytes))
        .transpose()
        .map_err(|e| failed(e.to_string()))
}

/// Records `identity` in `files` as the last version of `region`.
pub(super) fn set_last_version(
    files: &dyn StateFiles,
    region: &str,
    identity: &VersionIdentity,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(identity).map_err(|e| e.to_string())?;
    let written = files.write(&version_file(region), &bytes);
    written.map_err(|e| format!("the last version of {region}: {e}"))
}
