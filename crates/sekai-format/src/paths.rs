//! Path indexes, specified in `docs/spec/path-index.md`: a root document and binary chunks that
//! map every path of a view to the object that holds the file.

use std::fmt;
use std::mem::size_of;

use serde::{Deserialize, Serialize};

/// The version of the path index format.
pub const VERSION: u32 = 1;

const MAGIC: &[u8; 4] = b"SKPI";
const CHUNK_VERSION: u8 = 1;

/// One file of a view: its path and the object that holds it. A chunk is encoded from these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathFile {
    /// The path in the view, without a leading `/`.
    pub path: String,
    /// The SHA-256 of the object.
    pub sha256: [u8; 32],
    /// The extension of the object in the store, without the dot.
    pub extension: String,
    /// The size in bytes.
    pub size: u64,
    /// The media type.
    pub media_type: String,
}

/// A chunk, as the root lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkReference {
    /// The first path of the chunk.
    pub first: String,
    /// The SHA-256 of the chunk object, `<aa>/<sha256>.paths`, as lowercase hexadecimal.
    pub sha256: String,
    /// The number of paths in the chunk.
    pub files: u64,
}

/// The root of a path index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathIndex {
    /// Always `"path-index"`.
    pub format: String,
    /// The version of the format.
    pub version: u32,
    /// The view the paths belong to.
    pub view: String,
    /// The name of the published version.
    pub published_version: String,
    /// The revision of the publication.
    pub revision: String,
    /// The number of paths.
    pub files: u64,
    /// The chunks, in path order.
    pub chunks: Vec<ChunkReference>,
}

/// A path index or chunk that does not follow the format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathIndexError(String);

impl fmt::Display for PathIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "path index: {}", self.0)
    }
}

impl std::error::Error for PathIndexError {}

fn invalid(message: impl Into<String>) -> PathIndexError {
    PathIndexError(message.into())
}

impl PathIndex {
    /// Reads a root, checking its format and the order of its chunks.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, PathIndexError> {
        let index: Self = serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
        if index.format != "path-index" || index.version != VERSION {
            return Err(invalid(format!(
                "format {} version {} is not path-index version {VERSION}",
                index.format, index.version
            )));
        }
        if index
            .chunks
            .windows(2)
            .any(|pair| pair[0].first >= pair[1].first)
        {
            return Err(invalid("chunks are not in path order"));
        }
        Ok(index)
    }

    /// The position in `chunks` of the chunk whose range holds `path`: the last chunk whose
    /// first path is not after it.
    pub fn chunk_of(&self, path: &str) -> Option<usize> {
        self.chunks
            .partition_point(|chunk| chunk.first.as_str() <= path)
            .checked_sub(1)
    }
}

/// The files of one chunk, in path order.
///
/// A decoded chunk holds its files in a few allocations, whatever their number: the paths one
/// after another, a row for each file, and the distinct extensions and media types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// The paths of the files, one after another.
    paths: String,
    /// The files, in path order.
    rows: Vec<Row>,
    extensions: Vec<String>,
    media_types: Vec<String>,
}

/// A file of a chunk: where its path is in [`Chunk::paths`], and its strings in the tables.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    start: usize,
    end: usize,
    sha256: [u8; 32],
    size: u64,
    extension: usize,
    media_type: usize,
}

/// A file of a decoded chunk, borrowed from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkFile<'a> {
    /// The path in the view, without a leading `/`.
    pub path: &'a str,
    /// The SHA-256 of the object.
    pub sha256: [u8; 32],
    /// The extension of the object in the store, without the dot.
    pub extension: &'a str,
    /// The size in bytes.
    pub size: u64,
    /// The media type.
    pub media_type: &'a str,
}

impl From<ChunkFile<'_>> for PathFile {
    fn from(file: ChunkFile<'_>) -> Self {
        Self {
            path: file.path.to_owned(),
            sha256: file.sha256,
            extension: file.extension.to_owned(),
            size: file.size,
            media_type: file.media_type.to_owned(),
        }
    }
}

impl Chunk {
    /// Encodes `files`, which are in strictly increasing order of their paths.
    pub fn encode(files: &[PathFile]) -> Vec<u8> {
        let extensions = Strings::of(files.iter().map(|f| f.extension.as_str()));
        let media_types = Strings::of(files.iter().map(|f| f.media_type.as_str()));
        let mut out = MAGIC.to_vec();
        out.push(CHUNK_VERSION);
        extensions.write(&mut out);
        media_types.write(&mut out);
        write_length(&mut out, files.len());
        let mut previous: &[u8] = &[];
        for file in files {
            let path = file.path.as_bytes();
            let shared = path
                .iter()
                .zip(previous)
                .take_while(|(a, b)| a == b)
                .count();
            write_length(&mut out, shared);
            write_bytes(&mut out, &path[shared..]);
            out.extend_from_slice(&file.sha256);
            write_varint(&mut out, file.size);
            write_length(&mut out, extensions.index(&file.extension));
            write_length(&mut out, media_types.index(&file.media_type));
            previous = path;
        }
        out
    }

    /// Decodes a chunk, checking everything the format requires.
    pub fn decode(bytes: &[u8]) -> Result<Self, PathIndexError> {
        let mut reader = Reader { bytes, at: 0 };
        if reader.take(MAGIC.len())? != MAGIC || reader.take(1)? != [CHUNK_VERSION] {
            return Err(invalid("not a chunk of version 1"));
        }
        let extensions = reader.strings()?;
        let media_types = reader.strings()?;
        let count = reader.length()?;
        let mut paths = String::new();
        let mut rows: Vec<Row> = Vec::with_capacity(count.min(bytes.len()));
        let mut path = Vec::new();
        for _ in 0..count {
            let shared = reader.length()?;
            if shared > path.len() {
                return Err(invalid("a path shares more than the previous path has"));
            }
            path.truncate(shared);
            path.extend_from_slice(reader.bytes()?);
            let text = std::str::from_utf8(&path).map_err(|_| invalid("a path is not UTF-8"))?;
            if rows
                .last()
                .is_some_and(|last| &paths[last.start..last.end] >= text)
            {
                return Err(invalid("paths are not in strictly increasing order"));
            }
            let start = paths.len();
            paths.push_str(text);
            rows.push(Row {
                start,
                end: paths.len(),
                sha256: reader.take(32)?.try_into().expect("32 bytes were taken"),
                size: reader.varint()?,
                extension: position(&extensions, reader.length()?)?,
                media_type: position(&media_types, reader.length()?)?,
            });
        }
        if reader.at != bytes.len() {
            return Err(invalid("bytes follow the last path"));
        }
        paths.shrink_to_fit();
        Ok(Self {
            paths,
            rows,
            extensions,
            media_types,
        })
    }

    /// The file at `path`.
    pub fn get(&self, path: &str) -> Option<ChunkFile<'_>> {
        let at = self
            .rows
            .binary_search_by(|row| self.path(row).cmp(path))
            .ok()?;
        Some(self.file(&self.rows[at]))
    }

    /// The files, in path order.
    pub fn files(&self) -> impl ExactSizeIterator<Item = ChunkFile<'_>> {
        self.rows.iter().map(|row| self.file(row))
    }

    /// The bytes the chunk has allocated besides its own size, without the bookkeeping of the
    /// allocator, which is small for so few allocations.
    pub fn allocated(&self) -> usize {
        let table = |strings: &Vec<String>| {
            strings.capacity() * size_of::<String>()
                + strings.iter().map(String::capacity).sum::<usize>()
        };
        self.paths.capacity()
            + self.rows.capacity() * size_of::<Row>()
            + table(&self.extensions)
            + table(&self.media_types)
    }

    fn path(&self, row: &Row) -> &str {
        &self.paths[row.start..row.end]
    }

    fn file(&self, row: &Row) -> ChunkFile<'_> {
        ChunkFile {
            path: self.path(row),
            sha256: row.sha256,
            extension: &self.extensions[row.extension],
            size: row.size,
            media_type: &self.media_types[row.media_type],
        }
    }
}

/// The distinct strings of a chunk, in order of first use.
struct Strings<'a>(Vec<&'a str>);

impl<'a> Strings<'a> {
    fn of(all: impl Iterator<Item = &'a str>) -> Self {
        let mut distinct = Vec::new();
        for text in all {
            if !distinct.contains(&text) {
                distinct.push(text);
            }
        }
        Self(distinct)
    }

    fn index(&self, text: &str) -> usize {
        self.0
            .iter()
            .position(|known| *known == text)
            .expect("every string of the chunk is in its table")
    }

    fn write(&self, out: &mut Vec<u8>) {
        write_length(out, self.0.len());
        for text in &self.0 {
            write_bytes(out, text.as_bytes());
        }
    }
}

/// `index`, checked to be a position in `table`.
fn position(table: &[String], index: usize) -> Result<usize, PathIndexError> {
    if index < table.len() {
        Ok(index)
    } else {
        Err(invalid("a string index is out of range"))
    }
}

fn write_varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push((value & 0x7f) as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn write_length(out: &mut Vec<u8>, length: usize) {
    write_varint(
        out,
        u64::try_from(length).expect("a length fits in 64 bits"),
    );
}

fn write_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    write_length(out, bytes.len());
    out.extend_from_slice(bytes);
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], PathIndexError> {
        let end = self
            .at
            .checked_add(length)
            .filter(|&end| end <= self.bytes.len())
            .ok_or_else(|| invalid("the chunk ends early"))?;
        let taken = &self.bytes[self.at..end];
        self.at = end;
        Ok(taken)
    }

    fn varint(&mut self) -> Result<u64, PathIndexError> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.take(1)?[0];
            let bits = u64::from(byte & 0x7f);
            if bits << shift >> shift != bits {
                break;
            }
            value |= bits << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(invalid("a varint does not fit in 64 bits"))
    }

    fn length(&mut self) -> Result<usize, PathIndexError> {
        usize::try_from(self.varint()?).map_err(|_| invalid("a length does not fit in memory"))
    }

    fn bytes(&mut self) -> Result<&'a [u8], PathIndexError> {
        let length = self.length()?;
        self.take(length)
    }

    fn strings(&mut self) -> Result<Vec<String>, PathIndexError> {
        let count = self.length()?;
        (0..count)
            .map(|_| {
                let bytes = self.bytes()?;
                String::from_utf8(bytes.to_vec()).map_err(|_| invalid("a string is not UTF-8"))
            })
            .collect()
    }
}
