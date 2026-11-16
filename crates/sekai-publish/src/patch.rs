//! Changes to a publication, applied to the documents of the previous one.
//!
//! The tables of a publication, the path index and the directories, listings and failures of the
//! index root, are cut into chunks whose ends depend only on their rows. A change reads and writes
//! only the chunks and shards it falls in; every other document of the previous publication is
//! kept. A publication from nothing is a change from an empty previous one, so both are written by
//! the same code and give the same documents.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sekai_catalog::{Key, LocalStore, Store};
use sekai_format::chunks::{self, Patched, Row};
use sekai_format::directory_tree::{self, DirectoryTree, TreeFile};
use sekai_format::paths::{self, Chunk, ChunkReference, PathFile, PathIndex};
use sekai_format::publication::{
    AssetFailure, IndexChunk, IndexEntry, IndexRoot, IndexShard, ListingEntry, ListingTable,
    ObjectReference, Pointer, ShardReference, TableChunk, VERSION,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::clock::Utc;
use crate::history::{self, Last, Made, PathDiff, Versions};
use crate::layout::{Layout, Object, Publication};
use crate::plan::{Binding, Contribution, File, Plan, PublishError, document_path, hex};

const JSON: &str = "application/json";
const CHUNK_MEDIA_TYPE: &str = "application/octet-stream";

/// What a change does at a path: whether it removes the file there, and the file it adds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PathEdit {
    removed: Option<Owner>,
    added: Option<File>,
}

/// The asset whose file a change removes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Owner {
    /// The file is one of this asset, which the previous publication must confirm.
    Asset(String),
    /// The file is whatever the previous publication has at the path.
    Any,
}

/// A change to a publication: the files, listing entries and failures of the assets it removes
/// and adds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Change {
    paths: BTreeMap<String, PathEdit>,
    /// The edits of the listing of each form, by asset name.
    listings: BTreeMap<String, BTreeMap<String, Option<ListingEntry>>>,
    failures: BTreeMap<String, Option<AssetFailure>>,
}

impl Change {
    /// Removes what `asset`, an asset of the previous publication, contributed to it.
    pub(crate) fn remove(&mut self, asset: &Contribution) {
        for file in &asset.files {
            let edit = self.paths.entry(file.path.clone()).or_default();
            edit.removed = Some(Owner::Asset(asset.asset.name.clone()));
        }
        let name = asset.asset.name.clone();
        match &asset.outcome {
            Ok(_) => {
                self.listing(&asset.asset.form).insert(name, None);
            }
            Err(_) => {
                self.failures.insert(name, None);
            }
        }
    }

    /// Adds what `asset` contributes. Fails if another added asset has a file at the same path.
    pub(crate) fn add(&mut self, asset: Contribution) -> Result<(), PublishError> {
        for file in asset.files {
            self.add_file(file)?;
        }
        let name = asset.asset.name;
        match asset.outcome {
            Ok(entry) => {
                self.listing(&asset.asset.form).insert(name, Some(entry));
            }
            Err(failure) => {
                self.failures.insert(name, Some(failure));
            }
        }
        Ok(())
    }

    /// Removes the file at `path`, whichever asset it belongs to, from a view whose assets are
    /// neither listed nor failed, such as the storage view.
    pub fn remove_file(&mut self, path: &str) {
        self.paths.entry(path.to_owned()).or_default().removed = Some(Owner::Any);
    }

    /// Adds `file` to a view whose assets are neither listed nor failed. Fails if the change adds
    /// another file at its path.
    pub fn add_file(&mut self, file: File) -> Result<(), PublishError> {
        let edit = self.paths.entry(file.path.clone()).or_default();
        if edit.added.is_some() {
            return Err(two_files(&file.path));
        }
        edit.added = Some(file);
        Ok(())
    }

    fn listing(&mut self, form: &str) -> &mut BTreeMap<String, Option<ListingEntry>> {
        self.listings.entry(form.to_owned()).or_default()
    }

    /// The change that publishes `plan` from nothing.
    pub fn all(plan: &Plan) -> Result<Self, PublishError> {
        let mut change = Self::default();
        for file in &plan.files {
            let edit = change.paths.entry(file.path.clone()).or_default();
            if edit.added.replace(file.clone()).is_some() {
                return Err(two_files(&file.path));
            }
        }
        for (form, entries) in &plan.listings {
            let listing = change.listing(form);
            listing.extend(entries.iter().map(|e| (e.name.clone(), Some(e.clone()))));
        }
        let failures = plan.failures.iter();
        change.failures = failures
            .map(|f| (f.name.clone(), Some(f.clone())))
            .collect();
        Ok(change)
    }

    /// Whether the change changes nothing.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.failures.is_empty() && self.listings.is_empty()
    }
}

fn two_files(path: &str) -> PublishError {
    PublishError::new("plan", format!("two files are published at {path}"))
}

/// The roots of the previous publication of a view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Previous {
    /// The index root, in a layout that has one.
    pub index: Option<IndexRoot>,
    /// The root of the path index.
    pub paths: Option<PathIndex>,
    /// The directory tree, which is read when a change needs it.
    pub tree: Option<ObjectReference>,
    /// The publication that the version history continues from.
    pub last: Option<Last>,
}

impl Previous {
    /// The roots that `pointer` names, read from `store`.
    pub fn read(store: &dyn Store, pointer: &Pointer) -> Result<Self, PublishError> {
        let index = pointer
            .root
            .as_deref()
            .map(|root| {
                let bytes = read_object(store, root, "json")?;
                serde_json::from_slice(&bytes).map_err(|e| PublishError::new("previous", e))
            })
            .transpose()?;
        let paths = named_paths(store, pointer)?;
        let tree = pointer.tree.clone();
        let last = Some(Last::of(pointer, paths.as_ref()));
        Ok(Self {
            index,
            paths,
            tree,
            last,
        })
    }

    /// No publication to change, for a view published anew after the publication that `pointer`
    /// names, whose version history goes on. Only the root of its path index is read from
    /// `store`, for its number of files.
    pub fn after(store: &dyn Store, pointer: &Pointer) -> Result<Self, PublishError> {
        let paths = named_paths(store, pointer)?;
        Ok(Self {
            last: Some(Last::of(pointer, paths.as_ref())),
            ..Self::default()
        })
    }

    /// The files of the publication, looked up in the chunks of its path index, which are read
    /// from `store` as the lookups need them and kept.
    pub fn lookup<'s>(&self, store: &'s dyn Store) -> Lookup<'s> {
        Lookup::new(store, self.paths.as_ref())
    }
}

/// The root of the path index that `pointer` names, if it names one, read from `store`.
fn named_paths(store: &dyn Store, pointer: &Pointer) -> Result<Option<PathIndex>, PublishError> {
    let root = pointer.paths.as_deref();
    root.map(|root| read_path_index(store, root)).transpose()
}

/// The root of the path index `sha256`, read from `store`.
pub(crate) fn read_path_index(store: &dyn Store, sha256: &str) -> Result<PathIndex, PublishError> {
    let bytes = read_object(store, sha256, "json")?;
    PathIndex::from_bytes(&bytes).map_err(|e| PublishError::new("previous", e))
}

/// The files of a previous publication, by path. Each chunk of its path index is read once, when
/// a lookup first needs it, and lookups may run on several threads at once.
pub struct Lookup<'a>(Table<'a, PathFile>);

impl<'a> Lookup<'a> {
    /// The files of the path index `index`, whose chunks are read from `store`.
    pub(crate) fn new(store: &'a dyn Store, index: Option<&PathIndex>) -> Self {
        let chunks = index.map_or(&[][..], |index| &index.chunks);
        let firsts = chunks.iter().map(|c| (c.first.clone(), c.sha256.clone()));
        Self(Table::new(store, firsts, read_paths))
    }

    /// The file at `path`, if the publication has one.
    pub fn file(&self, path: &str) -> Result<Option<PathFile>, PublishError> {
        self.0.get(path)
    }

    /// The asset document of the asset `name`, if the publication has the asset.
    pub fn document(&self, name: &str) -> Result<Option<Key>, PublishError> {
        let file = self.file(&document_path(name))?;
        Ok(file.map(|file| Key {
            sha256: file.sha256,
            extension: file.extension,
        }))
    }

    /// The asset document of each asset of `names` that the publication has, in name order.
    pub fn bindings<'n>(
        &self,
        names: impl IntoIterator<Item = &'n str>,
    ) -> Result<Vec<Binding>, PublishError> {
        let mut names: Vec<&str> = names.into_iter().collect();
        names.sort_unstable();
        names.dedup();
        let mut bindings = Vec::with_capacity(names.len());
        for name in names {
            if let Some(document) = self.document(name)? {
                let name = name.to_owned();
                bindings.push(Binding { name, document });
            }
        }
        Ok(bindings)
    }

    /// The files directly in `directory`, in path order: those at `<directory>/<file>`, where the
    /// file has no `/`. Only the chunks that hold such paths are read.
    pub fn directory(&self, directory: &str) -> Result<Vec<PathFile>, PublishError> {
        let table = &self.0;
        let prefix = format!("{directory}/");
        // Every path in the directory sorts before this, since `0` follows `/`.
        let after = format!("{directory}0");
        let start = table.chunk_of(&prefix).unwrap_or(0);
        let mut files = Vec::new();
        for position in start..table.chunks.len() {
            if position > start && table.chunks[position].0 >= after {
                break;
            }
            let rows = table.rows(position)?;
            let inside = rows.iter().filter(|file| {
                let file_name = file.path.strip_prefix(&prefix);
                file_name.is_some_and(|name| !name.contains('/'))
            });
            files.extend(inside.cloned());
        }
        Ok(files)
    }
}

fn read_object(store: &dyn Store, sha256: &str, extension: &str) -> Result<Vec<u8>, PublishError> {
    let key = Key {
        sha256: sekai_catalog::store::parse_hex(sha256)
            .ok_or_else(|| PublishError::new("previous", format!("bad hash {sha256}")))?,
        extension: extension.to_owned(),
    };
    store.read(&key).map_err(PublishError::wrap("previous"))
}

/// The documents a publication writes, kept for upload.
pub(crate) struct Written<'a> {
    local: &'a LocalStore,
    objects: Vec<Object>,
}

impl Written<'_> {
    /// Stores `bytes` and returns the SHA-256 of the object, as lowercase hexadecimal.
    fn put(
        &mut self,
        bytes: &[u8],
        extension: &str,
        media_type: &str,
    ) -> Result<String, PublishError> {
        let key = self
            .local
            .put(bytes, extension)
            .map_err(PublishError::wrap("documents"))?;
        let sha256 = hex(&key.sha256);
        self.objects.push(Object {
            key,
            size: bytes.len() as u64,
            media_type: media_type.to_owned(),
        });
        Ok(sha256)
    }

    fn put_json(&mut self, document: &impl Serialize) -> Result<String, PublishError> {
        let bytes = serde_json::to_vec(document).map_err(|e| PublishError::new("documents", e))?;
        self.put(&bytes, "json", JSON)
    }

    /// Stores `document` as gzip-compressed JSON and returns the reference a pointer names it by.
    pub(crate) fn put_gzip(
        &mut self,
        document: &impl Serialize,
        extension: &str,
        media_type: &str,
    ) -> Result<ObjectReference, PublishError> {
        let json = serde_json::to_vec(document).map_err(|e| PublishError::new("documents", e))?;
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        let bytes = gzip
            .write_all(&json)
            .and_then(|()| gzip.finish())
            .map_err(PublishError::wrap("documents"))?;
        let sha256 = self.put(&bytes, extension, media_type)?;
        let size = bytes.len() as u64;
        Ok(ObjectReference { sha256, size })
    }
}

/// Reads the rows of the chunk object named by a SHA-256 from a store.
type ReadChunk<R> = fn(&dyn Store, &str) -> Result<Vec<R>, PublishError>;

/// A table of the previous publication whose chunks are read when a change needs them.
struct Table<'a, R> {
    store: &'a dyn Store,
    /// The first key and the object of each chunk.
    chunks: Vec<(String, String)>,
    read: ReadChunk<R>,
    loaded: Mutex<HashMap<usize, Arc<Vec<R>>>>,
}

impl<'a, R: Row<Key = String> + Clone> Table<'a, R> {
    fn new(
        store: &'a dyn Store,
        chunks: impl Iterator<Item = (String, String)>,
        read: ReadChunk<R>,
    ) -> Self {
        Self {
            store,
            chunks: chunks.collect(),
            read,
            loaded: Mutex::default(),
        }
    }

    /// The rows of the chunk at `position`, read once. Two threads that ask at once may both read
    /// it, and keep the same rows.
    fn rows(&self, position: usize) -> Result<Arc<Vec<R>>, PublishError> {
        let loaded = || self.loaded.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(rows) = loaded().get(&position) {
            return Ok(Arc::clone(rows));
        }
        let rows = Arc::new((self.read)(self.store, &self.chunks[position].1)?);
        loaded().insert(position, Arc::clone(&rows));
        Ok(rows)
    }

    /// The position of the chunk that holds `key` if any chunk does, which is the last chunk
    /// whose first key is not after it.
    fn chunk_of(&self, key: &str) -> Option<usize> {
        let after = self
            .chunks
            .partition_point(|(first, _)| first.as_str() <= key);
        after.checked_sub(1)
    }

    /// The row of `key`.
    fn get(&self, key: &str) -> Result<Option<R>, PublishError> {
        let Some(position) = self.chunk_of(key) else {
            return Ok(None);
        };
        let rows = self.rows(position)?;
        let found = rows.binary_search_by(|row| row.key().as_str().cmp(key));
        Ok(found.ok().map(|index| rows[index].clone()))
    }

    /// The chunks of the table after `edits`.
    fn patch(&self, edits: BTreeMap<String, Option<R>>) -> Result<Vec<Patched<R>>, PublishError> {
        let firsts: Vec<String> = self.chunks.iter().map(|(first, _)| first.clone()).collect();
        let load = |position| self.rows(position).map(Arc::unwrap_or_clone);
        chunks::patch(&firsts, load, edits)
    }
}

/// What a publication publishes: a version of a view, in a layout, at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edition<'a> {
    /// The view.
    pub view: &'a str,
    /// The name of the published version.
    pub version: &'a str,
    /// The layout, [`Layout::Paths`] or [`Layout::Index`].
    pub layout: Layout,
    /// When the publication is made, which its version history records, or `None` to record no
    /// time.
    pub published_at: Option<SystemTime>,
}

/// Applies `change` to `previous`, the publication of the view before it, as the publication
/// `edition`. The documents of earlier publications are read from `store` as the change needs
/// them, and their pointers from `versions`; the new documents are written to `local`. Returns
/// the pointer and the objects the publication adds: the files the change adds, and the
/// documents written.
pub fn apply(
    store: &dyn Store,
    local: &LocalStore,
    edition: &Edition<'_>,
    previous: &Previous,
    change: Change,
    versions: &dyn Versions,
) -> Result<Publication, PublishError> {
    let Edition {
        view,
        version,
        layout,
        published_at,
    } = *edition;
    let Change {
        paths: edits,
        listings,
        failures,
    } = change;
    let mut written = Written {
        local,
        objects: Vec::new(),
    };
    let tables = match layout {
        Layout::Paths => None,
        _ => {
            let edits = TableEdits {
                paths: &edits,
                listings,
                failures,
            };
            let previous = previous.index.as_ref();
            Some(index_tables(store, &mut written, previous, edits)?)
        }
    };
    let changes = PathChanges::new(store, previous.paths.as_ref(), &edits)?;
    let tree_files = changes.tree_files(previous.tree.as_ref())?;
    // The paths tell what changed since the last publication only when the change applies to it.
    let diffs = match (&previous.last, &previous.paths) {
        (Some(_), Some(_)) => Some(changes.diffs()?),
        _ => None,
    };
    let (paths, files) = changes.chunks(&mut written)?;
    let failures = tables.as_ref().map_or(&[][..], |t| &t.failures);
    let revision = revision(version, &paths, failures);
    let tree = DirectoryTree::new(view, version, &revision, tree_files)
        .map_err(|e| PublishError::new("documents", e))?;
    let tree = written.put_gzip(&tree, directory_tree::EXTENSION, directory_tree::MEDIA_TYPE)?;
    let made = Made {
        view,
        version,
        revision: &revision,
        files,
        published_at: published_at.map(|time| Utc::of(time).rfc3339()),
    };
    let last = previous.last.as_ref();
    let recorded = history::record(store, &mut written, &made, last, diffs, versions)?;

    let paths = written.put_json(&PathIndex {
        format: "path-index".to_owned(),
        version: paths::VERSION,
        view: view.to_owned(),
        published_version: version.to_owned(),
        revision: revision.clone(),
        files,
        chunks: paths,
    })?;
    let root = tables
        .map(|tables| {
            written.put_json(&IndexRoot {
                format: "index-root".to_owned(),
                version: VERSION,
                published_version: version.to_owned(),
                revision: revision.clone(),
                directories: tables.directories,
                listings: tables.listings,
                failures: tables.failures,
            })
        })
        .transpose()?;
    let added = edits.into_values().filter_map(|edit| edit.added);
    let mut objects: Vec<Object> = added
        .map(|file| Object {
            key: file.key,
            size: file.size,
            media_type: file.media_type,
        })
        .collect();
    objects.extend(written.objects);
    Ok(Publication {
        pointer: Pointer {
            format: "pointer".to_owned(),
            version: VERSION,
            published_version: version.to_owned(),
            revision,
            root,
            paths: Some(paths),
            tree: Some(tree),
            history: Some(recorded.history),
            changes: recorded.changes,
        },
        objects,
    })
}

/// The revision of a publication of `version` whose path index has the chunks `paths` and whose
/// failures have the chunks `failures`. Both depend only on what they list, so the revision does.
fn revision(version: &str, paths: &[ChunkReference], failures: &[TableChunk]) -> String {
    let mut text = format!("publication {VERSION}\nversion {version}\n");
    for chunk in paths {
        text.push_str(&format!("paths {}\n", chunk.sha256));
    }
    for chunk in failures {
        text.push_str(&format!("failures {}\n", chunk.sha256));
    }
    hex(&Sha256::digest(text.as_bytes()))
}

/// The path index of the previous publication and what a change does to its paths.
struct PathChanges<'a> {
    store: &'a dyn Store,
    /// The chunks of the previous index.
    old: &'a [ChunkReference],
    table: Table<'a, PathFile>,
    /// The file at each path the change edits, or `None` where it removes the file.
    rows: BTreeMap<String, Option<PathFile>>,
}

impl<'a> PathChanges<'a> {
    /// The changes that `edits` make to `previous`, the path index of the previous publication.
    /// Fails if a file is added at a path that the change does not free.
    fn new(
        store: &'a dyn Store,
        previous: Option<&'a PathIndex>,
        edits: &BTreeMap<String, PathEdit>,
    ) -> Result<Self, PublishError> {
        let old = previous.map_or(&[][..], |index| index.chunks.as_slice());
        let firsts = old.iter().map(|c| (c.first.clone(), c.sha256.clone()));
        let table = Table::new(store, firsts, read_paths);
        let mut rows = BTreeMap::new();
        for (path, edit) in edits {
            if edit.removed.is_none() && edit.added.is_some() && table.get(path)?.is_some() {
                return Err(two_files(path));
            }
            rows.insert(path.clone(), edit.added.as_ref().map(path_file));
        }
        Ok(Self {
            store,
            old,
            table,
            rows,
        })
    }

    /// The files of the publication after the change, in path order: those of `previous`, the
    /// directory tree of the previous publication, with the edits applied. Without a tree, they
    /// are read from every chunk of the previous path index.
    fn tree_files(
        &self,
        previous: Option<&ObjectReference>,
    ) -> Result<Vec<TreeFile>, PublishError> {
        let before = match previous {
            Some(tree) => read_tree(self.store, tree)?,
            None => self.files()?,
        };
        let kept = before
            .into_iter()
            .filter(|file| !self.rows.contains_key(&file.path));
        let added = self.rows.values().flatten().map(TreeFile::from);
        let mut files: Vec<TreeFile> = kept.chain(added).collect();
        files.sort_unstable_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }

    /// The paths whose bytes the change changes, in path order, with the files before and after.
    fn diffs(&self) -> Result<Vec<PathDiff>, PublishError> {
        let mut diffs = Vec::new();
        for (path, after) in &self.rows {
            let before = self.table.get(path)?;
            if before.as_ref().map(|f| f.sha256) != after.as_ref().map(|f| f.sha256) {
                let (path, after) = (path.clone(), after.clone());
                diffs.push(PathDiff {
                    path,
                    before,
                    after,
                });
            }
        }
        Ok(diffs)
    }

    /// Every file of the previous path index.
    fn files(&self) -> Result<Vec<TreeFile>, PublishError> {
        let mut files = Vec::new();
        for position in 0..self.old.len() {
            files.extend(self.table.rows(position)?.iter().map(TreeFile::from));
        }
        Ok(files)
    }

    /// The chunks of the path index after the change, and the number of paths.
    fn chunks(self, written: &mut Written<'_>) -> Result<(Vec<ChunkReference>, u64), PublishError> {
        let mut chunks = Vec::new();
        for chunk in self.table.patch(self.rows)? {
            chunks.push(match chunk {
                Patched::Kept(position) => self.old[position].clone(),
                Patched::New(files) => ChunkReference {
                    first: files[0].path.clone(),
                    sha256: written.put(&Chunk::encode(&files), "paths", CHUNK_MEDIA_TYPE)?,
                    files: files.len() as u64,
                },
            });
        }
        let files = chunks.iter().map(|chunk| chunk.files).sum();
        Ok((chunks, files))
    }
}

/// The JSON of the gzip-compressed document `document`, read from `store`.
pub(crate) fn read_gzip(
    store: &dyn Store,
    document: &ObjectReference,
    extension: &str,
) -> Result<Vec<u8>, PublishError> {
    let bytes = read_object(store, &document.sha256, extension)?;
    let mut json = Vec::new();
    GzDecoder::new(bytes.as_slice())
        .read_to_end(&mut json)
        .map_err(PublishError::wrap("previous"))?;
    Ok(json)
}

/// The files of the directory tree `tree`, read from `store`.
fn read_tree(store: &dyn Store, tree: &ObjectReference) -> Result<Vec<TreeFile>, PublishError> {
    let json = read_gzip(store, tree, directory_tree::EXTENSION)?;
    DirectoryTree::from_json(&json)
        .and_then(|tree| tree.files())
        .map_err(|e| PublishError::new("previous", e))
}

fn read_paths(store: &dyn Store, sha256: &str) -> Result<Vec<PathFile>, PublishError> {
    let chunk = Chunk::decode(&read_object(store, sha256, "paths")?)
        .map_err(|e| PublishError::new("previous", e))?;
    Ok(chunk.files().map(PathFile::from).collect())
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

/// The edits of the tables of an index root: the path edits, which change the shards of their
/// directories, and the edits of the listing of each form and of the failures.
struct TableEdits<'a> {
    paths: &'a BTreeMap<String, PathEdit>,
    listings: BTreeMap<String, BTreeMap<String, Option<ListingEntry>>>,
    failures: BTreeMap<String, Option<AssetFailure>>,
}

/// The tables of an index root.
struct IndexTables {
    directories: Vec<TableChunk>,
    listings: Vec<ListingTable>,
    failures: Vec<TableChunk>,
}

/// The tables of the index root after `edits`, with the shards of the directories they fall in.
fn index_tables(
    store: &dyn Store,
    written: &mut Written<'_>,
    previous: Option<&IndexRoot>,
    edits: TableEdits<'_>,
) -> Result<IndexTables, PublishError> {
    let TableEdits {
        paths,
        listings: listing_edits,
        failures: failure_edits,
    } = edits;
    let empty = IndexRoot::default();
    let previous = previous.unwrap_or(&empty);

    let directories = index_table(store, &previous.directories);
    let shards = shard_edits(store, written, &directories, paths)?;
    let directories = write_table(written, &directories, &previous.directories, shards)?;

    // The chunks of each form's listing before the change, and the edits of it.
    type Listing<'a> = (&'a [TableChunk], BTreeMap<String, Option<ListingEntry>>);
    let mut forms: BTreeMap<String, Listing<'_>> = listing_edits
        .into_iter()
        .map(|(form, edits)| (form, (&[][..], edits)))
        .collect();
    for listing in &previous.listings {
        forms.entry(listing.form.clone()).or_default().0 = &listing.chunks;
    }
    let mut listings = Vec::new();
    for (form, (old, edits)) in forms {
        let chunks = write_table(written, &index_table(store, old), old, edits)?;
        if !chunks.is_empty() {
            listings.push(ListingTable { form, chunks });
        }
    }

    let failures = index_table(store, &previous.failures);
    let failures = write_table(written, &failures, &previous.failures, failure_edits)?;
    Ok(IndexTables {
        directories,
        listings,
        failures,
    })
}

/// The table of the index root whose chunks are `chunks`.
fn index_table<'a, R: Row<Key = String> + Clone + DeserializeOwned>(
    store: &'a dyn Store,
    chunks: &[TableChunk],
) -> Table<'a, R> {
    let firsts = chunks.iter().map(|c| (c.first.clone(), c.sha256.clone()));
    Table::new(store, firsts, read_rows)
}

fn read_rows<R: DeserializeOwned>(store: &dyn Store, sha256: &str) -> Result<Vec<R>, PublishError> {
    let chunk: IndexChunk<R> = serde_json::from_slice(&read_object(store, sha256, "json")?)
        .map_err(|e| PublishError::new("previous", e))?;
    Ok(chunk.rows)
}

/// The chunks of `table`, whose chunks are `previous`, after `edits`: the kept chunks, and the new
/// ones, which are written.
fn write_table<R: Row<Key = String> + Clone + Serialize>(
    written: &mut Written<'_>,
    table: &Table<'_, R>,
    previous: &[TableChunk],
    edits: BTreeMap<String, Option<R>>,
) -> Result<Vec<TableChunk>, PublishError> {
    let mut chunks = Vec::new();
    for chunk in table.patch(edits)? {
        chunks.push(match chunk {
            Patched::Kept(position) => previous[position].clone(),
            Patched::New(rows) => TableChunk {
                first: rows[0].key().clone(),
                rows: rows.len() as u64,
                sha256: written.put_json(&IndexChunk {
                    format: "index-chunk".to_owned(),
                    version: VERSION,
                    rows,
                })?,
            },
        });
    }
    Ok(chunks)
}

fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

/// The edits of the directories that `edits` fall in: the new shard of each, which is written,
/// or none for a directory left empty.
fn shard_edits(
    store: &dyn Store,
    written: &mut Written<'_>,
    directories: &Table<'_, ShardReference>,
    edits: &BTreeMap<String, PathEdit>,
) -> Result<BTreeMap<String, Option<ShardReference>>, PublishError> {
    let mut by_directory: BTreeMap<&str, Vec<(&str, &PathEdit)>> = BTreeMap::new();
    for (path, edit) in edits {
        by_directory
            .entry(split(path).0)
            .or_default()
            .push((path, edit));
    }
    let mut edits = BTreeMap::new();
    for (directory, changes) in by_directory {
        let old = match directories.get(directory)? {
            Some(reference) => {
                let shard: IndexShard =
                    serde_json::from_slice(&read_object(store, &reference.sha256, "json")?)
                        .map_err(|e| PublishError::new("previous", e))?;
                shard.entries
            }
            None => Vec::new(),
        };
        let mut entries: BTreeMap<String, IndexEntry> =
            old.into_iter().map(|e| (e.name.clone(), e)).collect();
        for (path, edit) in changes {
            let name = split(path).1;
            match (entries.remove(name), &edit.removed) {
                (Some(_), Some(Owner::Any)) => {}
                (Some(entry), Some(Owner::Asset(owner))) if entry.asset == *owner => {}
                (Some(_), None) => return Err(two_files(path)),
                (None, Some(Owner::Any)) => {
                    return Err(PublishError::new(
                        "previous",
                        format!("{path} holds no file to remove"),
                    ));
                }
                (found, Some(Owner::Asset(owner))) => {
                    let found = found.map_or("nothing".to_owned(), |entry| entry.asset);
                    return Err(PublishError::new(
                        "previous",
                        format!("{path} holds {found}, not a file of {owner}"),
                    ));
                }
                (None, None) => {}
            }
            if let Some(file) = &edit.added {
                entries.insert(name.to_owned(), entry(name, file));
            }
        }
        let shard = (!entries.is_empty())
            .then(|| -> Result<ShardReference, PublishError> {
                let sha256 = written.put_json(&IndexShard {
                    format: "index-shard".to_owned(),
                    version: VERSION,
                    directory: directory.to_owned(),
                    entries: entries.into_values().collect(),
                })?;
                Ok(ShardReference {
                    directory: directory.to_owned(),
                    sha256,
                })
            })
            .transpose()?;
        edits.insert(directory.to_owned(), shard);
    }
    Ok(edits)
}

fn entry(name: &str, file: &File) -> IndexEntry {
    IndexEntry {
        name: name.to_owned(),
        sha256: hex(&file.key.sha256),
        extension: file.key.extension.clone(),
        size: file.size,
        media_type: file.media_type.clone(),
        asset: file.asset.clone(),
        role: file.role.clone(),
    }
}
