//! The objects of the store that a publication refers to, found from its pointer.

use sekai_catalog::{Key, Store};
use sekai_format::paths::Chunk;
use sekai_format::publication::{IndexRoot, Pointer, ShardReference};
use sekai_format::{directory_tree, version_history};

use crate::parallel;
use crate::patch::{object_key, read_object, read_path_index, read_rows};
use crate::plan::PublishError;

/// Every object of the store that the publication of `pointer` refers to, read from `store` on up
/// to `threads` threads: the documents of its layout and the files of its path index. An object
/// may be given more than once.
pub fn referred(
    store: &dyn Store,
    pointer: &Pointer,
    threads: usize,
) -> Result<Vec<Key>, PublishError> {
    let gzipped = [
        (&pointer.tree, directory_tree::EXTENSION),
        (&pointer.history, version_history::EXTENSION),
        (&pointer.changes, version_history::EXTENSION),
    ];
    let mut keys = Vec::new();
    for (document, extension) in gzipped {
        if let Some(document) = document {
            keys.push(object_key(&document.sha256, extension)?);
        }
    }
    if let Some(root) = &pointer.root {
        keys.extend(index_objects(store, root, threads)?);
    }
    if let Some(root) = &pointer.paths {
        keys.extend(path_objects(store, root, threads)?);
    }
    Ok(keys)
}

/// The index root `root`, the chunks of its tables and the shards of its directories.
fn index_objects(store: &dyn Store, root: &str, threads: usize) -> Result<Vec<Key>, PublishError> {
    let index: IndexRoot = serde_json::from_slice(&read_object(store, root, "json")?)
        .map_err(|e| PublishError::new("previous", e))?;
    let listings = index.listings.iter().flat_map(|listing| &listing.chunks);
    let chunks = index
        .directories
        .iter()
        .chain(listings)
        .chain(&index.failures);
    let mut keys = vec![object_key(root, "json")?];
    for chunk in chunks {
        keys.push(object_key(&chunk.sha256, "json")?);
    }
    let shards = parallel::map(&index.directories, threads, |chunk| {
        read_rows::<ShardReference>(store, &chunk.sha256)
    });
    for shards in shards {
        for shard in shards? {
            keys.push(object_key(&shard.sha256, "json")?);
        }
    }
    Ok(keys)
}

/// The root of the path index `root`, its chunks and the files they list.
fn path_objects(store: &dyn Store, root: &str, threads: usize) -> Result<Vec<Key>, PublishError> {
    let index = read_path_index(store, root)?;
    let mut keys = vec![object_key(root, "json")?];
    for chunk in &index.chunks {
        keys.push(object_key(&chunk.sha256, "paths")?);
    }
    let files = parallel::map(&index.chunks, threads, |chunk| {
        chunk_files(store, &chunk.sha256)
    });
    for files in files {
        keys.extend(files?);
    }
    Ok(keys)
}

/// The objects of the files that the path index chunk `sha256` lists.
fn chunk_files(store: &dyn Store, sha256: &str) -> Result<Vec<Key>, PublishError> {
    let chunk = Chunk::decode(&read_object(store, sha256, "paths")?)
        .map_err(|e| PublishError::new("previous", e))?;
    let files = chunk.files().map(|file| Key {
        sha256: file.sha256,
        extension: file.extension.to_owned(),
    });
    Ok(files.collect())
}
