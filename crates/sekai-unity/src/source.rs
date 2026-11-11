//! Byte sources: anything a serialized file can be read from.

use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use crate::error::{Error, Result};

/// Bytes returned by a [`ByteSource`]: borrowed from the source, shared with a cached block, or
/// assembled from several blocks.
#[derive(Debug, Clone)]
pub enum Chunk<'a> {
    /// A slice of the source's own bytes.
    Borrowed(&'a [u8]),
    /// A range of a decompressed block.
    Shared {
        /// The block.
        block: Arc<[u8]>,
        /// The start of the range in the block.
        start: usize,
        /// The end of the range in the block.
        end: usize,
    },
    /// Bytes copied together from several blocks.
    Owned(Vec<u8>),
}

impl Deref for Chunk<'_> {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Chunk::Borrowed(bytes) => bytes,
            Chunk::Shared { block, start, end } => &block[*start..*end],
            Chunk::Owned(bytes) => bytes,
        }
    }
}

/// A random-access source of bytes.
///
/// Implemented by byte slices and vectors, and by the entries of a UnityFS container, which
/// decompress only the blocks a read touches.
pub trait ByteSource: Send + Sync {
    /// The number of bytes in the source.
    fn len(&self) -> u64;

    /// Whether the source is empty.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `len` bytes starting at `offset`, or an error if the range is not inside the
    /// source.
    fn read(&self, offset: u64, len: usize) -> Result<Chunk<'_>>;
}

pub(crate) fn check_range(what: &'static str, total: u64, offset: u64, len: usize) -> Result<()> {
    match offset.checked_add(len as u64) {
        Some(end) if end <= total => Ok(()),
        _ => Err(Error::Truncated {
            what,
            offset,
            needed: len as u64,
            available: total.saturating_sub(offset),
        }),
    }
}

/// A file. Any number of files can be sources at once: at most 256 of them are kept open between
/// reads, the ones read most recently, and a file is read at an offset without moving a cursor,
/// so threads share its handle. The file must not change while it is a source.
#[derive(Debug, Clone)]
pub struct FileSource {
    path: PathBuf,
    len: u64,
}

impl FileSource {
    /// The file at `path`.
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let len = std::fs::metadata(&path)
            .map_err(|e| read_error(&path, e))?
            .len();
        Ok(Self { path, len })
    }
}

fn read_error(path: &Path, error: io::Error) -> Error {
    Error::Read {
        path: path.display().to_string(),
        message: error.to_string(),
    }
}

impl ByteSource for FileSource {
    fn len(&self) -> u64 {
        self.len
    }

    fn read(&self, offset: u64, len: usize) -> Result<Chunk<'_>> {
        check_range("file", self.len, offset, len)?;
        let mut bytes = vec![0; len];
        let file = open(&self.path).map_err(|e| read_error(&self.path, e))?;
        read_exact_at(&file, &mut bytes, offset).map_err(|e| read_error(&self.path, e))?;
        Ok(Chunk::Owned(bytes))
    }
}

/// How many files the sources keep open between reads.
const OPEN_FILES: usize = 256;

/// The files kept open for [`FileSource`]s.
static OPEN: LazyLock<Mutex<OpenFiles>> = LazyLock::new(Mutex::default);

/// Open files by path, each with when it was last used.
#[derive(Default)]
struct OpenFiles {
    by_path: HashMap<PathBuf, (Arc<File>, u64)>,
    clock: u64,
}

impl OpenFiles {
    /// The handle of the file at `path` if it is open, now marked as used.
    fn get(&mut self, path: &Path) -> Option<Arc<File>> {
        self.clock += 1;
        let (file, used) = self.by_path.get_mut(path)?;
        *used = self.clock;
        Some(file.clone())
    }

    /// Keeps `file` open as the file at `path`, closing the one used longest ago when
    /// [`OPEN_FILES`] are open. A closed file stays readable by the reads that hold it.
    fn keep(&mut self, path: &Path, file: Arc<File>) {
        if self.by_path.len() >= OPEN_FILES
            && let Some(oldest) = self.oldest()
        {
            self.by_path.remove(&oldest);
        }
        self.by_path.insert(path.to_owned(), (file, self.clock));
    }

    fn oldest(&self) -> Option<PathBuf> {
        let (path, _) = self.by_path.iter().min_by_key(|(_, (_, used))| *used)?;
        Some(path.clone())
    }
}

/// The handle of the file at `path`, opened if it is not open. The file is opened outside the
/// lock, so reads of open files do not wait for it.
fn open(path: &Path) -> io::Result<Arc<File>> {
    let lock = || OPEN.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(file) = lock().get(path) {
        return Ok(file);
    }
    let file = Arc::new(File::open(path)?);
    lock().keep(path, file.clone());
    Ok(file)
}

#[cfg(unix)]
fn read_exact_at(file: &File, bytes: &mut [u8], offset: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, bytes, offset)
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut bytes: &mut [u8], mut offset: u64) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !bytes.is_empty() {
        match file.seek_read(bytes, offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(read) => {
                bytes = &mut bytes[read..];
                offset += read as u64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Targets without positional reads have no files to open either.
#[cfg(not(any(unix, windows)))]
fn read_exact_at(_file: &File, _bytes: &mut [u8], _offset: u64) -> io::Result<()> {
    Err(io::ErrorKind::Unsupported.into())
}

impl ByteSource for [u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    fn read(&self, offset: u64, len: usize) -> Result<Chunk<'_>> {
        check_range("byte source", ByteSource::len(self), offset, len)?;
        let start = offset as usize;
        Ok(Chunk::Borrowed(&self[start..start + len]))
    }
}

impl ByteSource for Vec<u8> {
    fn len(&self) -> u64 {
        self.as_slice().len() as u64
    }

    fn read(&self, offset: u64, len: usize) -> Result<Chunk<'_>> {
        self.as_slice().read(offset, len)
    }
}

impl ByteSource for Arc<[u8]> {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    fn read(&self, offset: u64, len: usize) -> Result<Chunk<'_>> {
        (**self).read(offset, len)
    }
}

impl<T: ByteSource + ?Sized> ByteSource for &T {
    fn len(&self) -> u64 {
        (**self).len()
    }

    fn read(&self, offset: u64, len: usize) -> Result<Chunk<'_>> {
        (**self).read(offset, len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_inside_and_rejects_outside() {
        let data = vec![1u8, 2, 3, 4];
        assert_eq!(&*data.read(1, 2).unwrap(), &[2, 3]);
        assert!(data.read(3, 2).is_err());
        assert!(data.read(u64::MAX, 1).is_err());
    }

    #[test]
    fn more_files_than_are_kept_open_stay_readable() {
        let directory =
            std::env::temp_dir().join(format!("sekai-unity-files-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let sources: Vec<FileSource> = (0u32..)
            .take(OPEN_FILES + 10)
            .map(|i| {
                let path = directory.join(format!("{i}"));
                std::fs::write(&path, i.to_le_bytes()).unwrap();
                FileSource::open(path).unwrap()
            })
            .collect();
        for _ in 0..2 {
            for (i, source) in (0u32..).zip(&sources) {
                assert_eq!(&*source.read(1, 3).unwrap(), &i.to_le_bytes()[1..]);
            }
        }
        assert!(sources[0].read(2, 3).is_err());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
