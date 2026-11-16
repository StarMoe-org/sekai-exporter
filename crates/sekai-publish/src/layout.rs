//! The layouts of a publication: the documents each writes next to the files, and the pointers
//! that make a publication current.

use std::path::Path;
use std::time::SystemTime;

use sekai_catalog::{Key, LocalStore, Objects};
use sekai_format::publication::{Pointer, is_name};
use sha2::{Digest, Sha256};

use crate::history::NoVersions;
use crate::index;
use crate::patch::{self, Change, Edition, Previous};
use crate::plan::{Plan, PublishError, hex};

/// The documents a publication writes next to its files. Every layout has a path index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// Only the path index, for a view without asset documents.
    Paths,
    /// Index shards, listings and an index root.
    #[default]
    Index,
}

/// An object of the store that a publication refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// The name of the object.
    pub key: Key,
    /// The size in bytes.
    pub size: u64,
    /// The media type.
    pub media_type: String,
}

/// A file of a site, as a listing of the site finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The name of the file: its path under the site, with `/` separators.
    pub name: String,
    /// The size in bytes.
    pub size: u64,
    /// When the file was last written.
    pub modified: SystemTime,
}

/// A publication ready to be made current: its pointer, and every object it refers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publication {
    /// The pointer of the publication.
    pub pointer: Pointer,
    /// The files of the plan, then the documents written for it.
    pub objects: Vec<Object>,
}

/// The view of `target`, `<region>/<view>`.
fn view_of(target: &str) -> &str {
    target.rsplit('/').next().unwrap_or(target)
}

/// Writes the documents of `plan`, the publication of `target` (`<region>/<view>`), in `layout`
/// to `store`. The publication starts a version history of its own, and records no time.
pub fn documents(
    store: &LocalStore,
    target: &str,
    plan: &Plan,
    layout: Layout,
) -> Result<Publication, PublishError> {
    let edition = Edition {
        view: view_of(target),
        version: &plan.version,
        layout,
        published_at: None,
    };
    let change = Change::all(plan)?;
    let previous = Previous::default();
    patch::apply(store, store, &edition, &previous, change, &NoVersions)
}

/// Whether `version` can name its pointer file.
pub(crate) fn check_version(version: &str) -> Result<(), PublishError> {
    if !is_name(version) {
        return Err(PublishError::new(
            "pointer",
            format!("the version {version:?} cannot name a pointer"),
        ));
    }
    Ok(())
}

/// The path of the pointer of `version` of `target`, relative to the published root.
pub(crate) fn version_pointer(target: &str, version: &str) -> String {
    format!("{target}/versions/{version}.json")
}

/// Writes the documents of `plan` in `layout` to the local store of `objects`, checks that every
/// object it refers to is among `objects`, and then writes the pointer of its version and makes it
/// current at `<root>/<target>/current.json`. The store is written and checked on up to `threads`
/// threads. A failed stage leaves the previous pointer in place.
pub fn publish(
    objects: &Objects<'_>,
    root: &Path,
    target: &str,
    plan: &Plan,
    layout: Layout,
    threads: usize,
) -> Result<Pointer, PublishError> {
    check_version(&plan.version)?;
    let Publication {
        pointer,
        objects: referred,
    } = documents(objects.local, target, plan, layout)?;
    if let Some(object) = index::missing(objects, &referred, threads, |o| (&o.key, o.size))? {
        return Err(PublishError::new(
            "verify",
            format!("{} is not in the store", object.key),
        ));
    }
    make_current(root, target, &pointer)?;
    Ok(pointer)
}

/// The current pointer of `target` under the published root `root`, or `None` when there is none.
pub fn current(root: &Path, target: &str) -> Result<Option<Pointer>, PublishError> {
    read_pointer(&current_path(root, target))
}

/// The pointer of the published version `version` of `target` under the published root `root`,
/// or `None` when there is none.
pub fn version(root: &Path, target: &str, version: &str) -> Result<Option<Pointer>, PublishError> {
    check_version(version)?;
    read_pointer(&root.join(version_pointer(target, version)))
}

/// The pointer at `path`, or `None` when there is none.
fn read_pointer(path: &Path) -> Result<Option<Pointer>, PublishError> {
    let wrap = PublishError::wrap("pointer");
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(wrap(e)),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| PublishError::new("pointer", e))
}

/// Makes `pointer` the current publication of `target` under the published root `root`: writes
/// the pointer of its version, then the current pointer. Call it once every object of the
/// publication is in place.
pub fn make_current(root: &Path, target: &str, pointer: &Pointer) -> Result<(), PublishError> {
    check_version(&pointer.published_version)?;
    let version = root.join(version_pointer(target, &pointer.published_version));
    write_pointer(&version, pointer)?;
    write_pointer(&current_path(root, target), pointer)
}

fn current_path(root: &Path, target: &str) -> std::path::PathBuf {
    root.join(target).join("current.json")
}

/// Writes `pointer` to `path` through a temporary file, so a reader sees the old or the new one.
fn write_pointer(path: &Path, pointer: &Pointer) -> Result<(), PublishError> {
    let bytes = serde_json::to_vec(pointer).map_err(|e| PublishError::new("pointer", e))?;
    let wrap = PublishError::wrap("pointer");
    let parent = path
        .parent()
        .ok_or_else(|| PublishError::new("pointer", "the pointer path has no directory"))?;
    std::fs::create_dir_all(parent).map_err(&wrap)?;
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    let temporary = parent.join(format!(
        ".pointer.{}.{}.tmp",
        std::process::id(),
        hex(&digest[..8])
    ));
    std::fs::write(&temporary, &bytes).map_err(&wrap)?;
    std::fs::rename(&temporary, path).map_err(|e| {
        let _ = std::fs::remove_file(&temporary);
        wrap(e)
    })
}
