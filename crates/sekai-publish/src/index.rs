//! The documents of the `index` layout: shards that map the names of a directory to objects,
//! listings of the assets of each form, and the root that lists both.

use std::collections::BTreeMap;
use std::io;

use sekai_catalog::{Key, LocalStore, Objects};
use sekai_format::publication::{
    IndexEntry, IndexRoot, IndexShard, Listing, ListingReference, ShardReference, VERSION,
};
use serde::Serialize;

use crate::layout::Object;
use crate::parallel;
use crate::plan::{File, Plan, PublishError, hex};

/// Stores `document` as JSON.
fn put(store: &LocalStore, document: &impl Serialize) -> Result<Object, PublishError> {
    let bytes = serde_json::to_vec(document).map_err(|e| PublishError::new("documents", e))?;
    let key = store
        .put(&bytes, "json")
        .map_err(PublishError::wrap("documents"))?;
    Ok(Object {
        key,
        size: bytes.len() as u64,
        media_type: "application/json".to_owned(),
    })
}

/// The first of `files` that is not among `objects`; `key` gives the key and size of a file. The
/// files are checked against listings of the objects, a store directory at a time on up to
/// `threads` threads.
pub(crate) fn missing<'a, T: Sync>(
    objects: &Objects<'_>,
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
        let sizes = objects.sizes(*prefix)?;
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

/// The index documents of a publication: the root, and the shards, listings and root it consists
/// of, in the order they were written.
pub(crate) struct Documents {
    /// The index root.
    pub(crate) root: Key,
    /// The shards, the listings and the root.
    pub(crate) objects: Vec<Object>,
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
    let mut objects = parallel::map(&shards, threads, |shard| put(store, shard))
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
    let shards = shards
        .into_iter()
        .zip(&objects)
        .map(|(shard, object)| ShardReference {
            directory: shard.directory,
            sha256: hex(&object.key.sha256),
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
            sha256: hex(&listing.key.sha256),
        });
        objects.push(listing);
    }
    let root = put(
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
    let key = root.key.clone();
    objects.push(root);
    Ok(Documents { root: key, objects })
}
