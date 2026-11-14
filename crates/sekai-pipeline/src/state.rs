//! The state of a region between runs: what each bundle of its last version holds and names, what
//! each unit gave, and the assets built from tables. A run reads it at its start and writes what
//! changed at its end.
//!
//! Each table of the state is a sequence of rows in name order, cut into chunks whose ends depend
//! only on the rows ([`sekai_format::chunks`]). A chunk is a state file of gzipped JSON named by
//! its SHA-256, and the root of the region names the chunks of every table. A run therefore
//! writes only the chunks that hold a changed row, and the root.

use std::collections::{BTreeMap, HashSet};
use std::io::{self, Read, Write};
use std::sync::{Mutex, MutexGuard, PoisonError};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sekai_catalog::Key;
use sekai_format::chunks::{self, Row};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::eval::{Output, hex, parallel};
use crate::reuse::UnitResult;
use crate::version::BundleFiles;

/// The format version of the root of a region.
const VERSION: u32 = 1;

/// A chunk ends after a row whose name hashes to a multiple of this, so chunks hold this many
/// rows on average.
const AVERAGE: u32 = 256;

/// Where the state is kept, as files named by paths.
pub trait StateFiles: Sync {
    /// The bytes of the file `name`, or `None` when there is none.
    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>>;
    /// Writes `bytes` as the file `name`.
    fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()>;
}

/// State files in memory.
#[derive(Debug, Default)]
pub struct MemoryFiles(Mutex<BTreeMap<String, Vec<u8>>>);

impl MemoryFiles {
    fn files(&self) -> MutexGuard<'_, BTreeMap<String, Vec<u8>>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The names of the files, in order.
    pub fn names(&self) -> Vec<String> {
        self.files().keys().cloned().collect()
    }
}

impl StateFiles for MemoryFiles {
    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        Ok(self.files().get(name).cloned())
    }

    fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.files().insert(name.to_owned(), bytes.to_vec());
        Ok(())
    }
}

/// What a bundle holds and names, under the content its manifest entry declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleState {
    /// The name of the bundle.
    pub name: String,
    /// The content of the bundle, as [`crate::stream::Entry::content`] identifies it.
    pub content: String,
    /// Each serialized file of the bundle and the external files it names.
    pub files: BundleFiles,
}

/// An asset built from tables.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableOutput {
    /// The name of the asset.
    pub name: String,
    /// The asset form.
    pub form: String,
    /// The asset document.
    pub document: String,
    /// The error code, for an asset that could not be produced.
    pub error: Option<String>,
}

impl TableOutput {
    /// The state of `output`.
    pub fn of(output: &Output) -> Self {
        Self {
            name: output.name.clone(),
            form: output.form.to_owned(),
            document: output.document.to_string(),
            error: output.error.clone(),
        }
    }

    /// The output, with its form among `forms`.
    pub fn output(&self, forms: &[&'static str]) -> io::Result<Output> {
        let invalid = |what: String| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the table asset {} {what}", self.name),
            )
        };
        let form = forms
            .iter()
            .copied()
            .find(|&form| form == self.form)
            .ok_or_else(|| invalid(format!("has the unknown form {}", self.form)))?;
        let document: Key = self
            .document
            .parse()
            .map_err(|_| invalid(format!("names the document {}", self.document)))?;
        Ok(Output {
            name: self.name.clone(),
            bundle: String::new(),
            form,
            document,
            error: self.error.clone(),
        })
    }
}

/// The state of a region after a run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegionState {
    /// The name of the version.
    pub version: String,
    /// The revision of each view published from it, by target.
    pub published: BTreeMap<String, String>,
    /// The bundles of the version, in name order.
    pub bundles: Vec<BundleState>,
    /// The result of each unit, in name order.
    pub units: Vec<UnitResult>,
    /// The assets built from tables, in name order.
    pub tables: Vec<TableOutput>,
}

/// The root of a region: the chunks of each table of its state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Root {
    format: String,
    version: u32,
    published_version: String,
    published: BTreeMap<String, String>,
    bundles: Vec<ChunkReference>,
    units: Vec<ChunkReference>,
    tables: Vec<ChunkReference>,
}

/// A chunk of a table, as the root lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ChunkReference {
    /// The name of the first row.
    first: String,
    /// The SHA-256 of the chunk file, as lowercase hexadecimal.
    sha256: String,
    /// The number of rows.
    rows: u64,
}

impl Root {
    /// The SHA-256 of every chunk the root names.
    fn chunks(&self) -> HashSet<&str> {
        let tables = [&self.bundles, &self.units, &self.tables];
        let chunks = tables.into_iter().flatten();
        chunks.map(|chunk| chunk.sha256.as_str()).collect()
    }
}

/// What writing a state did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Written {
    /// The chunks written.
    pub chunks: usize,
    /// The bytes written, with the root.
    pub bytes: u64,
}

fn root_file(region: &str) -> String {
    format!("regions/{region}.json")
}

fn chunk_file(sha256: &str) -> String {
    format!("chunks/{}/{sha256}.json.gz", &sha256[..2])
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

impl Row for BundleState {
    type Key = String;
    const MOST: usize = 4 * AVERAGE as usize;

    fn key(&self) -> &String {
        &self.name
    }

    fn ends_chunk(&self) -> bool {
        chunks::ends_after(self.name.as_bytes(), AVERAGE)
    }
}

impl Row for UnitResult {
    type Key = String;
    const MOST: usize = 4 * AVERAGE as usize;

    fn key(&self) -> &String {
        &self.unit
    }

    fn ends_chunk(&self) -> bool {
        chunks::ends_after(self.unit.as_bytes(), AVERAGE)
    }
}

impl Row for TableOutput {
    type Key = String;
    const MOST: usize = 4 * AVERAGE as usize;

    fn key(&self) -> &String {
        &self.name
    }

    fn ends_chunk(&self) -> bool {
        chunks::ends_after(self.name.as_bytes(), AVERAGE)
    }
}

impl RegionState {
    /// The state of `region` that `files` keep, and its root, or `None` when they keep none.
    /// The chunks are read on up to `threads` threads.
    pub fn read(
        files: &dyn StateFiles,
        region: &str,
        threads: usize,
    ) -> io::Result<Option<(Self, Root)>> {
        let Some(bytes) = files.read(&root_file(region))? else {
            return Ok(None);
        };
        let root: Root = serde_json::from_slice(&bytes)
            .map_err(|e| invalid(format!("the state of {region}: {e}")))?;
        if root.format != "region-state" || root.version != VERSION {
            return Err(invalid(format!(
                "the state of {region} is {} version {}",
                root.format, root.version
            )));
        }
        let state = Self {
            version: root.published_version.clone(),
            published: root.published.clone(),
            bundles: read_table(files, &root.bundles, threads)?,
            units: read_table(files, &root.units, threads)?,
            tables: read_table(files, &root.tables, threads)?,
        };
        Ok(Some((state, root)))
    }

    /// Writes the state to `files` as the state of `region`: the chunks that `previous` does not
    /// name, on up to `threads` threads, and then the root.
    pub fn write(
        &self,
        files: &dyn StateFiles,
        region: &str,
        previous: Option<&Root>,
        threads: usize,
    ) -> io::Result<Written> {
        let kept = previous.map(Root::chunks).unwrap_or_default();
        let mut written = Written::default();
        let root = Root {
            format: "region-state".to_owned(),
            version: VERSION,
            published_version: self.version.clone(),
            published: self.published.clone(),
            bundles: write_table(files, &self.bundles, &kept, threads, &mut written)?,
            units: write_table(files, &self.units, &kept, threads, &mut written)?,
            tables: write_table(files, &self.tables, &kept, threads, &mut written)?,
        };
        let bytes = serde_json::to_vec(&root).map_err(io::Error::other)?;
        files.write(&root_file(region), &bytes)?;
        written.bytes += bytes.len() as u64;
        Ok(written)
    }
}

/// The rows of the chunks `table`, read from `files` on up to `threads` threads.
fn read_table<R: DeserializeOwned + Send>(
    files: &dyn StateFiles,
    table: &[ChunkReference],
    threads: usize,
) -> io::Result<Vec<R>> {
    let chunks = parallel(table.len(), threads, |i| read_chunk(files, &table[i]));
    let mut rows = Vec::new();
    for chunk in chunks {
        rows.extend(chunk?);
    }
    Ok(rows)
}

fn read_chunk<R: DeserializeOwned>(
    files: &dyn StateFiles,
    chunk: &ChunkReference,
) -> io::Result<Vec<R>> {
    let name = chunk_file(&chunk.sha256);
    let bytes = files
        .read(&name)?
        .ok_or_else(|| invalid(format!("the state file {name} is missing")))?;
    if hex(&Sha256::digest(&bytes)) != chunk.sha256 {
        return Err(invalid(format!("the state file {name} holds other bytes")));
    }
    let mut json = Vec::new();
    GzDecoder::new(bytes.as_slice()).read_to_end(&mut json)?;
    let rows: Vec<R> =
        serde_json::from_slice(&json).map_err(|e| invalid(format!("{name}: {e}")))?;
    if rows.len() as u64 != chunk.rows {
        return Err(invalid(format!("{name} does not hold {} rows", chunk.rows)));
    }
    Ok(rows)
}

/// Writes the chunks of `rows` that are not among `kept` to `files`, on up to `threads` threads,
/// adds them to `written`, and returns the chunks of the table.
fn write_table<R: Row<Key = String> + Serialize + Clone + Sync>(
    files: &dyn StateFiles,
    rows: &[R],
    kept: &HashSet<&str>,
    threads: usize,
    written: &mut Written,
) -> io::Result<Vec<ChunkReference>> {
    let chunks = chunks::cut(rows.to_vec());
    let encoded = parallel(chunks.len(), threads, |i| {
        write_chunk(files, &chunks[i], kept)
    });
    let mut table = Vec::with_capacity(chunks.len());
    for chunk in encoded {
        let (reference, new) = chunk?;
        if let Some(bytes) = new {
            written.chunks += 1;
            written.bytes += bytes;
        }
        table.push(reference);
    }
    Ok(table)
}

/// Writes `chunk` to `files` unless it is among `kept`. Returns the chunk as the root lists it,
/// and its size when it was written.
fn write_chunk<R: Row<Key = String> + Serialize>(
    files: &dyn StateFiles,
    chunk: &[R],
    kept: &HashSet<&str>,
) -> io::Result<(ChunkReference, Option<u64>)> {
    let bytes = encode(chunk)?;
    let sha256 = hex(&Sha256::digest(&bytes));
    let new = !kept.contains(sha256.as_str());
    if new {
        files.write(&chunk_file(&sha256), &bytes)?;
    }
    let reference = ChunkReference {
        first: chunk[0].key().clone(),
        sha256,
        rows: chunk.len() as u64,
    };
    Ok((reference, new.then_some(bytes.len() as u64)))
}

/// The gzipped JSON of `rows`. The same rows always give the same bytes.
fn encode<R: Serialize>(rows: &[R]) -> io::Result<Vec<u8>> {
    let json = serde_json::to_vec(rows).map_err(io::Error::other)?;
    let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
    gzip.write_all(&json)?;
    gzip.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reuse::{Record, UnitKey};

    fn unit(name: String, reusable: bool) -> UnitResult {
        UnitResult {
            key: Some(UnitKey(Sha256::digest(name.as_bytes()).into())),
            unit: name,
            reusable,
            record: Record::default(),
        }
    }

    fn state(units: usize) -> RegionState {
        let name = |i: usize| format!("bundle/{i:06}");
        RegionState {
            version: "1.0".to_owned(),
            published: BTreeMap::from([("jp/canonical".to_owned(), "r1".to_owned())]),
            bundles: (0..units)
                .map(|i| BundleState {
                    name: name(i),
                    content: format!("declared {i}"),
                    files: vec![(format!("CAB-{i}"), vec!["CAB-0".to_owned()])],
                })
                .collect(),
            units: (0..units).map(|i| unit(name(i), i % 7 != 0)).collect(),
            tables: vec![TableOutput {
                name: "table/a".to_owned(),
                form: "costume".to_owned(),
                document: format!("{}.json", "ab".repeat(32)),
                error: None,
            }],
        }
    }

    #[test]
    fn a_state_reads_back_as_it_was_written() {
        let files = MemoryFiles::default();
        let written = state(3000);
        written.write(&files, "jp", None, 4).unwrap();
        let (read, _) = RegionState::read(&files, "jp", 4).unwrap().unwrap();
        assert_eq!(read, written);
        assert!(RegionState::read(&files, "tw", 4).unwrap().is_none());
    }

    #[test]
    fn a_change_writes_only_the_chunks_it_falls_in() {
        let files = MemoryFiles::default();
        let first = state(20_000);
        let all = first.write(&files, "jp", None, 4).unwrap();
        let (mut second, root) = RegionState::read(&files, "jp", 4).unwrap().unwrap();
        second.version = "1.1".to_owned();
        second.units[12_345].reusable = !second.units[12_345].reusable;
        second.bundles.remove(777);

        let changed = second.write(&files, "jp", Some(&root), 4).unwrap();
        assert!(all.chunks > 100);
        assert!(changed.chunks <= 4, "{changed:?}");
        assert!(changed.bytes * 10 < all.bytes, "{changed:?} {all:?}");
        let (read, _) = RegionState::read(&files, "jp", 4).unwrap().unwrap();
        assert_eq!(read, second);
    }

    #[test]
    fn a_chunk_with_other_bytes_is_refused() {
        let files = MemoryFiles::default();
        state(10).write(&files, "jp", None, 1).unwrap();
        let names = files.names();
        let chunk = names
            .iter()
            .find(|name| name.starts_with("chunks/"))
            .unwrap();
        let mut bytes = files.read(chunk).unwrap().unwrap();
        bytes.push(0);
        files.write(chunk, &bytes).unwrap();
        let error = RegionState::read(&files, "jp", 1).unwrap_err();
        assert!(error.to_string().contains(chunk.as_str()), "{error}");
    }

    #[test]
    fn a_table_output_gives_back_its_output() {
        let output = Output {
            name: "table/a".to_owned(),
            bundle: String::new(),
            form: "costume",
            document: Key::of(b"{}", "json"),
            error: Some("missing_asset".to_owned()),
        };
        let state = TableOutput::of(&output);
        assert_eq!(state.output(&["costume"]).unwrap(), output);
        assert!(state.output(&["episode"]).is_err());
    }
}
