//! The `index` layout: the store itself, plus shards that map names to objects.

use std::collections::BTreeMap;
use std::path::Path;

use sekai_catalog::{Key, LocalStore};
use sekai_format::publication::{
    IndexEntry, IndexRoot, IndexShard, Listing, ListingReference, Pointer, ShardReference, VERSION,
};
use serde::Serialize;

use crate::plan::{File, Plan, PublishError, hex};

fn put(
    store: &LocalStore,
    written: &mut Vec<(Key, u64)>,
    document: &impl Serialize,
) -> Result<String, PublishError> {
    let bytes = serde_json::to_vec(document).map_err(|e| PublishError::new("documents", e))?;
    let key = store
        .put(&bytes, "json")
        .map_err(PublishError::wrap("documents"))?;
    let sha256 = hex(&key.sha256);
    written.push((key, bytes.len() as u64));
    Ok(sha256)
}

fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

fn entry(file: &File) -> IndexEntry {
    IndexEntry {
        name: split(&file.path).1.to_owned(),
        sha256: hex(&file.key.sha256),
        extension: file.key.extension.clone(),
        size: file.size,
        media_type: file.media_type.clone(),
        asset: file.asset.clone(),
        role: file.role.clone(),
    }
}

/// Whether the store holds `key` with `size` bytes. An object is written under a temporary name
/// and renamed, so one that exists under its name is complete.
fn stored(store: &LocalStore, key: &Key, size: u64) -> bool {
    std::fs::metadata(store.path(key)).is_ok_and(|m| m.is_file() && m.len() == size)
}

/// The documents of an `index` publication: the pointer, and the shards, listings and root it
/// refers to with their sizes, written to the local store.
pub(crate) struct Documents {
    /// The pointer of the publication.
    pub(crate) pointer: Pointer,
    /// The shards, listings and root, in the order they were written.
    pub(crate) objects: Vec<(Key, u64)>,
}

/// Writes the shards, listings and root of `plan` to `store`.
pub(crate) fn documents(store: &LocalStore, plan: &Plan) -> Result<Documents, PublishError> {
    let mut written = Vec::new();
    let mut directories: BTreeMap<&str, Vec<IndexEntry>> = BTreeMap::new();
    for file in &plan.files {
        directories
            .entry(split(&file.path).0)
            .or_default()
            .push(entry(file));
    }
    let mut shards = Vec::new();
    for (directory, mut entries) in directories {
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        let sha256 = put(
            store,
            &mut written,
            &IndexShard {
                format: "index-shard".to_owned(),
                version: VERSION,
                directory: directory.to_owned(),
                entries,
            },
        )?;
        shards.push(ShardReference {
            directory: directory.to_owned(),
            sha256,
        });
    }
    let mut listings = Vec::new();
    for (form, entries) in &plan.listings {
        let sha256 = put(
            store,
            &mut written,
            &Listing {
                format: "listing".to_owned(),
                version: VERSION,
                form: form.clone(),
                entries: entries.clone(),
            },
        )?;
        listings.push(ListingReference {
            form: form.clone(),
            sha256,
        });
    }
    let index_root = put(
        store,
        &mut written,
        &IndexRoot {
            format: "index-root".to_owned(),
            version: VERSION,
            published_version: plan.version.clone(),
            revision: plan.revision.clone(),
            shards,
            listings,
            failures: plan.failures.clone(),
        },
    )?;

    Ok(Documents {
        pointer: Pointer {
            format: "pointer".to_owned(),
            version: VERSION,
            published_version: plan.version.clone(),
            revision: plan.revision.clone(),
            root: index_root,
        },
        objects: written,
    })
}

/// Writes the shards, listings and root of `plan` to `store`, checks that every file exists, and
/// then points `<root>/<target>/current.json` at the publication. A failed stage leaves the
/// previous pointer in place.
pub fn publish(
    store: &LocalStore,
    root: &Path,
    target: &str,
    plan: &Plan,
) -> Result<Pointer, PublishError> {
    let Documents { pointer, objects } = documents(store, plan)?;
    if let Some(file) = plan
        .files
        .iter()
        .find(|file| !stored(store, &file.key, file.size))
    {
        return Err(PublishError::new(
            "verify",
            format!("{} is not in the store as {}", file.path, file.key),
        ));
    }
    if let Some((key, _)) = objects
        .iter()
        .find(|(key, size)| !stored(store, key, *size))
    {
        return Err(PublishError::new(
            "verify",
            format!("{key} is not in the store"),
        ));
    }

    let bytes = serde_json::to_vec(&pointer).map_err(|e| PublishError::new("pointer", e))?;
    let wrap = PublishError::wrap("pointer");
    let directory = root.join(target);
    std::fs::create_dir_all(&directory).map_err(&wrap)?;
    let temporary = directory.join("current.json.tmp");
    std::fs::write(&temporary, bytes).map_err(&wrap)?;
    std::fs::rename(&temporary, directory.join("current.json")).map_err(&wrap)?;
    Ok(pointer)
}
