//! The `index` layout: the store itself, plus shards that map names to objects.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use sekai_catalog::{Key, LocalStore};
use sekai_format::publication::{
    IndexEntry, IndexRoot, IndexShard, Listing, ListingReference, Pointer, ShardReference, VERSION,
};
use serde::Serialize;

use crate::parallel;
use crate::plan::{File, Plan, PublishError, hex};

/// Stores `document` as JSON and gives its key and size.
fn put(store: &LocalStore, document: &impl Serialize) -> Result<(Key, u64), PublishError> {
    let bytes = serde_json::to_vec(document).map_err(|e| PublishError::new("documents", e))?;
    let key = store
        .put(&bytes, "json")
        .map_err(PublishError::wrap("documents"))?;
    Ok((key, bytes.len() as u64))
}

/// The first of `files` that is not in `store`; `key` gives the key and size of a file. The files
/// are checked against listings of the store, a directory of it at a time on up to `threads`
/// threads.
pub(crate) fn missing<'a, T: Sync>(
    store: &LocalStore,
    files: &'a [T],
    threads: usize,
    key: impl Fn(&T) -> (&Key, u64) + Sync,
) -> Result<Option<&'a T>, PublishError> {
    let mut by_prefix: Vec<(u8, Vec<usize>)> =
        (0..=u8::MAX).map(|prefix| (prefix, Vec::new())).collect();
    for (index, file) in files.iter().enumerate() {
        by_prefix[usize::from(key(file).0.sha256[0])].1.push(index);
    }
    let first_missing = parallel::map(&by_prefix, threads, |(prefix, indices)| {
        if indices.is_empty() {
            return Ok(None);
        }
        let sizes = store.sizes(*prefix)?;
        Ok(indices.iter().copied().find(|&index| {
            let (key, size) = key(&files[index]);
            sizes.get(key) != Some(&size)
        }))
    });
    let first_missing = first_missing
        .into_iter()
        .collect::<io::Result<Vec<_>>>()
        .map_err(PublishError::wrap("verify"))?
        .into_iter()
        .flatten()
        .min();
    Ok(first_missing.map(|index| &files[index]))
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
pub(crate) fn stored(store: &LocalStore, key: &Key, size: u64) -> bool {
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

/// Writes the shards, listings and root of `plan` to `store`, the shards on up to `threads`
/// threads.
pub(crate) fn documents(
    store: &LocalStore,
    plan: &Plan,
    threads: usize,
) -> Result<Documents, PublishError> {
    let mut directories: BTreeMap<&str, Vec<IndexEntry>> = BTreeMap::new();
    for file in &plan.files {
        directories
            .entry(split(&file.path).0)
            .or_default()
            .push(entry(file));
    }
    let shards: Vec<IndexShard> = directories
        .into_iter()
        .map(|(directory, mut entries)| {
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            IndexShard {
                format: "index-shard".to_owned(),
                version: VERSION,
                directory: directory.to_owned(),
                entries,
            }
        })
        .collect();
    let mut written = parallel::map(&shards, threads, |shard| put(store, shard))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let shards = shards
        .into_iter()
        .zip(&written)
        .map(|(shard, (key, _))| ShardReference {
            directory: shard.directory,
            sha256: hex(&key.sha256),
        })
        .collect();
    let mut listings = Vec::new();
    for (form, entries) in &plan.listings {
        let listing = put(
            store,
            &Listing {
                format: "listing".to_owned(),
                version: VERSION,
                form: form.clone(),
                entries: entries.clone(),
            },
        )?;
        listings.push(ListingReference {
            form: form.clone(),
            sha256: hex(&listing.0.sha256),
        });
        written.push(listing);
    }
    let (index_root, size) = put(
        store,
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
    let root = hex(&index_root.sha256);
    written.push((index_root, size));

    Ok(Documents {
        pointer: Pointer {
            format: "pointer".to_owned(),
            version: VERSION,
            published_version: plan.version.clone(),
            revision: plan.revision.clone(),
            root,
            sqlite: None,
        },
        objects: written,
    })
}

/// Writes the shards, listings and root of `plan` to `store`, checks that every file exists, and
/// then points `<root>/<target>/current.json` at the publication. The store is written and
/// checked on up to `threads` threads. A failed stage leaves the previous pointer in place.
pub fn publish(
    store: &LocalStore,
    root: &Path,
    target: &str,
    plan: &Plan,
    threads: usize,
) -> Result<Pointer, PublishError> {
    let Documents { pointer, objects } = documents(store, plan, threads)?;
    if let Some(file) = missing(store, &plan.files, threads, |file| (&file.key, file.size))? {
        return Err(PublishError::new(
            "verify",
            format!("{} is not in the store as {}", file.path, file.key),
        ));
    }
    if let Some((key, _)) = missing(store, &objects, threads, |(key, size)| (key, *size))? {
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
