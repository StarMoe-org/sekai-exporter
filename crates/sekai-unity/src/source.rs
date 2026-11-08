//! Byte sources: anything a serialized file can be read from.

use std::ops::Deref;
use std::sync::Arc;

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
}
