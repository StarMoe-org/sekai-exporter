//! The layouts of a publication: the documents each writes next to the files, and the pointers
//! that make a publication current.

use std::path::Path;

use sekai_catalog::{Key, LocalStore};
use sekai_format::publication::{Pointer, VERSION, is_name};
use sha2::{Digest, Sha256};

use crate::plan::{Plan, PublishError, hex};
use crate::{index, paths, sqlite};

/// The documents a publication writes next to its files. Every layout has a path index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// Only the path index, for a view without asset documents.
    Paths,
    /// Index shards, listings and an index root.
    Index,
    /// The `index` layout and an `index.sqlite` database.
    #[default]
    Sqlite,
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
/// to `store`, on up to `threads` threads.
pub fn documents(
    store: &LocalStore,
    target: &str,
    plan: &Plan,
    layout: Layout,
    threads: usize,
) -> Result<Publication, PublishError> {
    let mut objects: Vec<Object> = plan
        .files
        .iter()
        .map(|file| Object {
            key: file.key.clone(),
            size: file.size,
            media_type: file.media_type.clone(),
        })
        .collect();
    let paths = paths::write(store, view_of(target), plan)?;
    objects.extend(paths.objects);
    let mut pointer = Pointer {
        format: "pointer".to_owned(),
        version: VERSION,
        published_version: plan.version.clone(),
        revision: plan.revision.clone(),
        root: None,
        sqlite: None,
        paths: Some(hex(&paths.root.sha256)),
    };
    if layout != Layout::Paths {
        let index = index::documents(store, plan, threads)?;
        pointer.root = Some(hex(&index.root.sha256));
        objects.extend(index.objects);
    }
    if layout == Layout::Sqlite {
        let sqlite = sqlite::build(store, plan)?;
        pointer.sqlite = Some(hex(&sqlite.key.sha256));
        objects.push(sqlite);
    }
    Ok(Publication { pointer, objects })
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

/// Writes the documents of `plan` in `layout` to `store`, checks that every object it refers to
/// is in `store`, and then writes the pointer of its version and makes it current at
/// `<root>/<target>/current.json`. The store is written and checked on up to `threads` threads.
/// A failed stage leaves the previous pointer in place.
pub fn publish(
    store: &LocalStore,
    root: &Path,
    target: &str,
    plan: &Plan,
    layout: Layout,
    threads: usize,
) -> Result<Pointer, PublishError> {
    check_version(&plan.version)?;
    let Publication { pointer, objects } = documents(store, target, plan, layout, threads)?;
    if let Some(object) = index::missing(store, &objects, threads, |o| (&o.key, o.size))? {
        return Err(PublishError::new(
            "verify",
            format!("{} is not in the store", object.key),
        ));
    }
    write_pointer(&root.join(version_pointer(target, &plan.version)), &pointer)?;
    write_pointer(&root.join(target).join("current.json"), &pointer)?;
    Ok(pointer)
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
