//! Where a region is published and where its state is kept: a bucket, or the output directory.

use std::io;
use std::ops::AddAssign;
use std::path::{Path, PathBuf};

use sekai_catalog::{Key, NoRemote, Objects, Remote};
use sekai_format::identity::RecordedVersion;
use sekai_format::publication::Pointer;
use sekai_pipeline::state::{DirectoryFiles, StateFiles};
use sekai_publish::history::Versions;
use sekai_publish::{Layout, Listed, Object, PublishError};

use crate::bucket::Bucket;

/// Where a region is published and its state kept.
pub(crate) enum Site<'a> {
    /// The output directory, with the objects under `store/`, the state under `state/` and the
    /// views under `published/`.
    Local {
        output: PathBuf,
        state: DirectoryFiles,
    },
    /// A bucket, which keeps the state under its `state/`.
    Bucket(&'a Bucket),
}

/// What a store put at a site: how many objects it uploaded, and how many the site held already.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Stored {
    pub(crate) uploaded: usize,
    pub(crate) present: usize,
}

impl AddAssign for Stored {
    fn add_assign(&mut self, other: Self) {
        self.uploaded += other.uploaded;
        self.present += other.present;
    }
}

/// The state file that holds the last version of `region`.
fn version_file(region: &str) -> String {
    format!("versions/{region}.json")
}

impl<'a> Site<'a> {
    /// The output directory `output` as the site.
    pub(crate) fn local(output: &Path) -> Self {
        Self::Local {
            output: output.to_owned(),
            state: DirectoryFiles::new(output.join("state")),
        }
    }

    /// Runs `work` while this run holds the lease of the site as one that does `operation`, so
    /// that one run at a time writes the state and pointers of a bucket. The output directory
    /// belongs to the run that has it.
    pub(crate) fn while_held<T>(
        &self,
        operation: &str,
        work: impl FnOnce() -> T,
    ) -> Result<T, String> {
        match self {
            Self::Local { .. } => Ok(work()),
            Self::Bucket(bucket) => bucket.while_held(operation, work),
        }
    }

    /// The state files of the site.
    pub(crate) fn files(&self) -> &dyn StateFiles {
        match self {
            Self::Local { state, .. } => state,
            Self::Bucket(bucket) => *bucket,
        }
    }

    /// Where objects that the local store lacks are read from.
    pub(crate) fn remote(&self) -> &dyn Remote {
        match self {
            Self::Local { .. } => &NoRemote,
            Self::Bucket(bucket) => bucket.remote(),
        }
    }

    /// How many reads and writes of objects and state files run at once: the connections of a
    /// bucket, or `threads` for the output directory.
    pub(crate) fn requests(&self, threads: usize) -> usize {
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
    pub(crate) fn current(&self, target: &str) -> Result<Option<Pointer>, String> {
        match self {
            Self::Local { output, .. } => {
                sekai_publish::current(&published(output), target).map_err(|e| e.to_string())
            }
            Self::Bucket(bucket) => bucket.current(target),
        }
    }

    /// The published versions of `target`.
    pub(crate) fn versions<'s>(&'s self, target: &'s str) -> PublishedVersions<'s> {
        PublishedVersions { site: self, target }
    }

    /// Puts the objects of `referred` where the site serves them, and records in the holdings of
    /// `objects` that the site holds them. The output directory serves its local store, which
    /// holds them already.
    pub(crate) fn store(
        &self,
        objects: &Objects<'_>,
        referred: Vec<Object>,
    ) -> Result<Stored, String> {
        match self {
            Self::Local { .. } => {
                let held: Vec<(Key, u64)> = referred
                    .into_iter()
                    .map(|object| (object.key, object.size))
                    .collect();
                objects.held.hold(&held).map_err(|e| e.to_string())?;
                Ok(Stored {
                    uploaded: 0,
                    present: held.len(),
                })
            }
            Self::Bucket(bucket) => bucket.store(objects, referred),
        }
    }

    /// Makes `pointer` the current publication of `target`.
    pub(super) fn make_current(&self, target: &str, pointer: &Pointer) -> Result<(), String> {
        match self {
            Self::Local { output, .. } => {
                let published = published(output);
                sekai_publish::make_current(&published, target, pointer).map_err(|e| e.to_string())
            }
            Self::Bucket(bucket) => bucket.make_current(target, pointer),
        }
    }

    /// The files under `directory` of the site, named by their paths under the site.
    pub(crate) fn list(&self, directory: &str) -> Result<Vec<Listed>, String> {
        match self {
            Self::Local { output, .. } => {
                let mut files = Vec::new();
                walk(output, directory, &mut files).map_err(|e| format!("{directory}: {e}"))?;
                Ok(files)
            }
            Self::Bucket(bucket) => bucket.files(directory),
        }
    }

    /// The files of the subdirectories of `directory` named by two hexadecimal digits, as objects
    /// and records lie, named by their paths under the site. A bucket lists one subdirectory per
    /// request.
    pub(crate) fn list_sharded(&self, directory: &str) -> Result<Vec<Listed>, String> {
        match self {
            Self::Local { output, .. } => {
                sharded_files(output, directory).map_err(|e| format!("{directory}: {e}"))
            }
            Self::Bucket(bucket) => bucket.sharded_files(directory),
        }
    }

    /// Deletes the file `name` of the site, named by its path under the site. A file the site
    /// does not hold is deleted already.
    pub(crate) fn delete(&self, name: &str) -> Result<(), String> {
        match self {
            Self::Local { output, .. } => match std::fs::remove_file(output.join(name)) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => Err(format!("{name}: {e}")),
                _ => Ok(()),
            },
            Self::Bucket(bucket) => bucket.delete(name),
        }
    }

    /// Deletes the state file `name`. A file the site does not hold is deleted already.
    pub(crate) fn delete_state(&self, name: &str) -> Result<(), String> {
        self.delete(&format!("state/{name}"))
    }
}

/// The directory of the views in the output directory `output`.
fn published(output: &Path) -> PathBuf {
    output.join("published")
}

/// Whether `name` names a file that is still being written, which a listing leaves out.
fn partial(name: &str) -> bool {
    name.starts_with('.')
}

/// The file `path`, named `name` under the site, as a listing finds it.
fn listed(name: String, path: &Path) -> io::Result<Listed> {
    let metadata = std::fs::metadata(path)?;
    Ok(Listed {
        name,
        size: metadata.len(),
        modified: metadata.modified()?,
    })
}

/// The entries of the directory `path`, none when it does not exist.
fn entries(path: &Path) -> io::Result<Vec<std::fs::DirEntry>> {
    match std::fs::read_dir(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        entries => entries?.collect(),
    }
}

/// Adds the files under `directory` of the output directory `output` to `files`.
fn walk(output: &Path, directory: &str, files: &mut Vec<Listed>) -> io::Result<()> {
    for entry in entries(&output.join(directory))? {
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        let name = format!("{directory}/{file_name}");
        if entry.file_type()?.is_dir() {
            walk(output, &name, files)?;
        } else if !partial(&file_name) {
            files.push(listed(name, &entry.path())?);
        }
    }
    Ok(())
}

/// The files of the subdirectories of `directory` of the output directory `output` named by two
/// hexadecimal digits.
fn sharded_files(output: &Path, directory: &str) -> io::Result<Vec<Listed>> {
    let mut files = Vec::new();
    for entry in entries(&output.join(directory))? {
        let shard = entry.file_name();
        let shard = shard.to_string_lossy();
        let named_by_digits = shard.len() == 2 && shard.bytes().all(|b| b.is_ascii_hexdigit());
        if named_by_digits && entry.file_type()?.is_dir() {
            walk(output, &format!("{directory}/{shard}"), &mut files)?;
        }
    }
    Ok(files)
}

/// The published versions of a target of a site.
pub(crate) struct PublishedVersions<'s> {
    site: &'s Site<'s>,
    target: &'s str,
}

impl Versions for PublishedVersions<'_> {
    fn pointer(&self, version: &str) -> Result<Option<Pointer>, PublishError> {
        match self.site {
            Site::Local { output, .. } => {
                sekai_publish::version(&published(output), self.target, version)
            }
            Site::Bucket(bucket) => bucket
                .version(self.target, version)
                .map_err(|e| PublishError::new("previous", e)),
        }
    }
}

/// The last version of `region` that a run recorded in `files`.
pub(super) fn last_version(
    files: &dyn StateFiles,
    region: &str,
) -> Result<Option<RecordedVersion>, String> {
    let failed = |e: String| format!("the last version of {region}: {e}");
    let bytes = files
        .read(&version_file(region))
        .map_err(|e| failed(e.to_string()))?;
    bytes
        .map(|bytes| serde_json::from_slice(&bytes))
        .transpose()
        .map_err(|e| failed(e.to_string()))
}

/// Records `recorded` in `files` as the last version of `region`.
pub(super) fn set_last_version(
    files: &dyn StateFiles,
    region: &str,
    recorded: &RecordedVersion,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(recorded).map_err(|e| e.to_string())?;
    files
        .write(&version_file(region), &bytes)
        .map_err(|e| format!("the last version of {region}: {e}"))
}
