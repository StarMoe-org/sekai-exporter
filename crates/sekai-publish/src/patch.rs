//! Changes to a publication, applied to the documents of the previous one.
//!
//! The tables of a publication, the path index and the directories, listings and failures of the
//! index root, are cut into chunks whose ends depend only on their rows. A change reads and writes
//! only the chunks and shards it falls in; every other document of the previous publication is
//! kept. A publication from nothing is a change from an empty previous one, so both are written by
//! the same code and give the same documents.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use sekai_catalog::{Key, LocalStore, Store};
use sekai_format::chunks::{self, Patched, Row};
use sekai_format::paths::{self, Chunk, ChunkReference, PathFile, PathIndex};
use sekai_format::publication::{
    AssetFailure, IndexChunk, IndexEntry, IndexRoot, IndexShard, ListingEntry, ListingTable,
    Pointer, ShardReference, TableChunk, VERSION,
};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::layout::{Layout, Object, Publication};
use crate::plan::{Contribution, File, Plan, PublishError, hex};

const JSON: &str = "application/json";
const CHUNK_MEDIA_TYPE: &str = "application/octet-stream";

/// What a change does at a path: the file of an asset it removes, and the file it adds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PathEdit {
    removed: Option<String>,
    added: Option<File>,
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
            edit.removed = Some(asset.asset.name.clone());
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
            let edit = self.paths.entry(file.path.clone()).or_default();
            if edit.added.is_some() {
                return Err(two_files(&file.path));
            }
            edit.added = Some(file);
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
}

impl Previous {
    /// The roots that `pointer` names, read from `store`.
    pub fn read(store: &dyn Store, pointer: &Pointer) -> Result<Self, PublishError> {
        let read = |sha256: &str| read_object(store, sha256, "json");
        let index = pointer
            .root
            .as_deref()
            .map(|root| {
                serde_json::from_slice(&read(root)?).map_err(|e| PublishError::new("previous", e))
            })
            .transpose()?;
        let paths = pointer
            .paths
            .as_deref()
            .map(|root| {
                PathIndex::from_bytes(&read(root)?).map_err(|e| PublishError::new("previous", e))
            })
            .transpose()?;
        Ok(Self { index, paths })
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
struct Written<'a> {
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
}

/// Reads the rows of the chunk object named by a SHA-256 from a store.
type ReadChunk<R> = fn(&dyn Store, &str) -> Result<Vec<R>, PublishError>;

/// A table of the previous publication whose chunks are read when a change needs them.
struct Table<'a, R> {
    store: &'a dyn Store,
    /// The first key and the object of each chunk.
    chunks: Vec<(String, String)>,
    read: ReadChunk<R>,
    loaded: RefCell<HashMap<usize, Rc<Vec<R>>>>,
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
            loaded: RefCell::default(),
        }
    }

    fn rows(&self, position: usize) -> Result<Rc<Vec<R>>, PublishError> {
        if let Some(rows) = self.loaded.borrow().get(&position) {
            return Ok(Rc::clone(rows));
        }
        let rows = Rc::new((self.read)(self.store, &self.chunks[position].1)?);
        self.loaded.borrow_mut().insert(position, Rc::clone(&rows));
        Ok(rows)
    }

    /// The row of `key`.
    fn get(&self, key: &str) -> Result<Option<R>, PublishError> {
        let position = self
            .chunks
            .partition_point(|(first, _)| first.as_str() <= key);
        let Some(position) = position.checked_sub(1) else {
            return Ok(None);
        };
        let rows = self.rows(position)?;
        let found = rows.binary_search_by(|row| row.key().as_str().cmp(key));
        Ok(found.ok().map(|index| rows[index].clone()))
    }

    /// The chunks of the table after `edits`.
    fn patch(&self, edits: BTreeMap<String, Option<R>>) -> Result<Vec<Patched<R>>, PublishError> {
        let firsts: Vec<String> = self.chunks.iter().map(|(first, _)| first.clone()).collect();
        let load = |position| self.rows(position).map(Rc::unwrap_or_clone);
        chunks::patch(&firsts, load, edits)
    }
}

/// Applies `change` to `previous`, the publication of `view` before it, as the publication of
/// `version` in `layout`, which is [`Layout::Paths`] or [`Layout::Index`]. The documents of the
/// previous publication are read from `store` as the change needs them, and the new ones are
/// written to `local`. Returns the pointer and the objects the publication adds: the files the
/// change adds, and the documents written.
pub fn apply(
    store: &dyn Store,
    local: &LocalStore,
    view: &str,
    version: &str,
    layout: Layout,
    previous: &Previous,
    change: Change,
) -> Result<Publication, PublishError> {
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
    let (paths, files) = path_chunks(store, &mut written, previous.paths.as_ref(), &edits)?;
    let failures = tables.as_ref().map_or(&[][..], |t| &t.failures);
    let revision = revision(version, &paths, failures);

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
            sqlite: None,
            paths: Some(paths),
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

/// The chunks of the path index after `change`, and the number of paths.
fn path_chunks(
    store: &dyn Store,
    written: &mut Written<'_>,
    previous: Option<&PathIndex>,
    edits: &BTreeMap<String, PathEdit>,
) -> Result<(Vec<ChunkReference>, u64), PublishError> {
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
    let mut chunks = Vec::new();
    for chunk in table.patch(rows)? {
        chunks.push(match chunk {
            Patched::Kept(position) => old[position].clone(),
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
                (Some(entry), Some(owner)) if entry.asset == *owner => {}
                (Some(_), None) => return Err(two_files(path)),
                (found, Some(owner)) => {
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
