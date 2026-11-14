//! The path index of a view, specified in `docs/spec/path-index.md`: its paths in chunks whose
//! boundaries depend only on the paths, so an unchanged range keeps its chunks.

use sekai_catalog::{Key, LocalStore};
use sekai_format::chunks;
use sekai_format::paths::{Chunk, ChunkReference, PathFile, PathIndex, VERSION};

use crate::layout::Object;
use crate::plan::{File, Plan, PublishError, hex};

/// The media type of a chunk object.
const CHUNK_MEDIA_TYPE: &str = "application/octet-stream";

/// A path index written to the store.
pub(crate) struct Written {
    /// The root of the index.
    pub(crate) root: Key,
    /// The chunks and the root, in the order they were written.
    pub(crate) objects: Vec<Object>,
}

fn path_file(file: &File) -> PathFile {
    PathFile {
        path: file.path.clone(),
        sha256: file.key.sha256,
        extension: file.key.extension.clone(),
        size: file.size,
        media_type: file.media_type.clone(),
    }
}

fn put(
    store: &LocalStore,
    bytes: &[u8],
    extension: &str,
    media_type: &str,
) -> Result<Object, PublishError> {
    let key = store
        .put(bytes, extension)
        .map_err(PublishError::wrap("documents"))?;
    Ok(Object {
        key,
        size: bytes.len() as u64,
        media_type: media_type.to_owned(),
    })
}

/// Writes the path index of the files of `plan`, the publication of `view`, to `store`.
pub(crate) fn write(store: &LocalStore, view: &str, plan: &Plan) -> Result<Written, PublishError> {
    let mut objects = Vec::new();
    let mut references = Vec::new();
    for files in chunks::cut(plan.files.iter().map(path_file).collect()) {
        let object = put(store, &Chunk::encode(&files), "paths", CHUNK_MEDIA_TYPE)?;
        references.push(ChunkReference {
            first: files[0].path.clone(),
            sha256: hex(&object.key.sha256),
            files: files.len() as u64,
        });
        objects.push(object);
    }
    let index = PathIndex {
        format: "path-index".to_owned(),
        version: VERSION,
        view: view.to_owned(),
        published_version: plan.version.clone(),
        revision: plan.revision.clone(),
        files: plan.files.len() as u64,
        chunks: references,
    };
    let bytes = serde_json::to_vec(&index).map_err(|e| PublishError::new("documents", e))?;
    let root = put(store, &bytes, "json", "application/json")?;
    let key = root.key.clone();
    objects.push(root);
    Ok(Written { root: key, objects })
}
