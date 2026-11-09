//! UnityFS containers.
//!
//! A UnityFS file holds one stream of data, split into blocks that are compressed separately,
//! and a directory of entries that are ranges of that stream. Opening a bundle reads only the
//! header and the directory; reading an entry decompresses the blocks the read touches.

use std::sync::Arc;

use crate::cache::{BlockCache, BlockKey};
use crate::endian::{Endian, Reader};
use crate::error::{Error, Result};
use crate::source::{ByteSource, Chunk, check_range};

const SIGNATURE: &[u8] = b"UnityFS";

/// The low six bits of the header flags and of each block's flags select the compression.
const COMPRESSION_MASK: u32 = 0x3f;
/// The directory is stored at the end of the file instead of after the header.
const BLOCKS_INFO_AT_END: u32 = 0x80;
/// The data starts at the next multiple of 16 after the directory.
const BLOCKS_INFO_PADDING: u32 = 0x200;
/// The entry is a serialized file.
const ENTRY_SERIALIZED: u32 = 0x4;

/// The container format version of the containers [`store_files`] writes.
const STORED_FORMAT_VERSION: u32 = 6;
/// The size of the hash at the start of the directory.
const DIRECTORY_HASH: usize = 16;

/// Whether `bytes` start with the UnityFS signature.
pub fn is_unityfs(bytes: &[u8]) -> bool {
    bytes.len() > SIGNATURE.len() && bytes.starts_with(SIGNATURE) && bytes[SIGNATURE.len()] == 0
}

/// The compression of a block or of the directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    /// Stored without compression.
    None,
    /// LZMA, with the five-byte properties header and no size field.
    Lzma,
    /// The LZ4 block format. Unity's LZ4 and LZ4HC settings store the same format.
    Lz4,
}

impl Compression {
    fn from_flags(flags: u32) -> Result<Self> {
        match flags & COMPRESSION_MASK {
            0 => Ok(Compression::None),
            1 => Ok(Compression::Lzma),
            2 | 3 => Ok(Compression::Lz4),
            other => Err(Error::Unsupported {
                what: "compression",
                value: other.into(),
            }),
        }
    }
}

/// The fixed header of a UnityFS file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleHeader {
    /// The container format version.
    pub format_version: u32,
    /// The engine version string, as stored.
    pub unity_version: String,
    /// The engine revision string, as stored.
    pub unity_revision: String,
    /// The total size declared by the header.
    pub size: u64,
    /// The header flags.
    pub flags: u32,
}

/// One compressed block of the data stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockInfo {
    /// The size of the block after decompression.
    pub uncompressed_size: u32,
    /// The size of the block as stored.
    pub compressed_size: u32,
    /// The block flags.
    pub flags: u16,
}

/// One entry of the directory: a named range of the data stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The offset of the entry in the data stream.
    pub offset: u64,
    /// The size of the entry.
    pub size: u64,
    /// The entry flags.
    pub flags: u32,
    /// The entry path, such as `CAB-<hash>` or `CAB-<hash>.resS`.
    pub path: String,
}

impl Entry {
    /// Whether the entry is a serialized file.
    pub fn is_serialized(&self) -> bool {
        self.flags & ENTRY_SERIALIZED != 0
    }
}

#[derive(Debug, Clone, Copy)]
struct Block {
    info: BlockInfo,
    compression: Compression,
    /// The offset of the stored block in the file.
    stored_offset: u64,
    /// The offset of the decompressed block in the data stream.
    stream_offset: u64,
}

/// An open UnityFS file.
pub struct Bundle {
    source: Arc<dyn ByteSource>,
    id: [u8; 32],
    header: BundleHeader,
    blocks: Vec<Block>,
    entries: Vec<Entry>,
    stream_len: u64,
}

impl std::fmt::Debug for Bundle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bundle")
            .field("header", &self.header)
            .field("blocks", &self.blocks.len())
            .field("entries", &self.entries)
            .finish()
    }
}

impl Bundle {
    /// Opens a UnityFS file, reading its header and directory.
    ///
    /// `id` is the SHA-256 of the file's bytes; it keys the file's blocks in a [`BlockCache`].
    pub fn open(source: Arc<dyn ByteSource>, id: [u8; 32]) -> Result<Self> {
        let total = source.len();
        let head_len = total.min(4096) as usize;
        let head = source.read(0, head_len)?;
        let mut r = Reader::new(&head, Endian::Big, "UnityFS header");
        if r.cstr()? != SIGNATURE {
            return Err(Error::Signature {
                expected: "UnityFS",
            });
        }
        let format_version = r.u32()?;
        if !(6..=8).contains(&format_version) {
            return Err(Error::Unsupported {
                what: "UnityFS format version",
                value: format_version.into(),
            });
        }
        let unity_version = r.cstring()?;
        let unity_revision = r.cstring()?;
        let size = r.i64()?;
        let compressed_info_size = r.u32()?;
        let uncompressed_info_size = r.u32()?;
        let flags = r.u32()?;
        if format_version >= 7 {
            r.align(16)?;
        }
        let after_header = r.pos() as u64;
        let header = BundleHeader {
            format_version,
            unity_version,
            unity_revision,
            size: u64::try_from(size)
                .map_err(|_| Error::malformed("UnityFS header", "negative size"))?,
            flags,
        };

        let info_offset = if flags & BLOCKS_INFO_AT_END != 0 {
            total
                .checked_sub(compressed_info_size.into())
                .ok_or_else(|| Error::malformed("UnityFS header", "directory outside the file"))?
        } else {
            after_header
        };
        let stored_info = source.read(info_offset, compressed_info_size as usize)?;
        let info = decompress(
            Compression::from_flags(flags)?,
            &stored_info,
            uncompressed_info_size as usize,
        )
        .map_err(|message| Error::Decompress {
            block: u32::MAX,
            message,
        })?;

        let mut data_offset = if flags & BLOCKS_INFO_AT_END != 0 {
            after_header
        } else {
            after_header + u64::from(compressed_info_size)
        };
        if flags & BLOCKS_INFO_PADDING != 0 {
            data_offset = data_offset.next_multiple_of(16);
        }

        let mut r = Reader::new(&info, Endian::Big, "UnityFS directory");
        r.skip(16)?;
        let block_count = r.i32()?;
        let block_count = usize::try_from(block_count)
            .map_err(|_| Error::malformed("UnityFS directory", "negative block count"))?;
        let mut blocks = Vec::with_capacity(block_count.min(r.remaining() / 10));
        let mut stored_offset = data_offset;
        let mut stream_offset = 0u64;
        for _ in 0..block_count {
            let info = BlockInfo {
                uncompressed_size: r.u32()?,
                compressed_size: r.u32()?,
                flags: r.u16()?,
            };
            blocks.push(Block {
                info,
                compression: Compression::from_flags(info.flags.into())?,
                stored_offset,
                stream_offset,
            });
            stored_offset += u64::from(info.compressed_size);
            stream_offset += u64::from(info.uncompressed_size);
        }
        if stored_offset > total {
            return Err(Error::malformed(
                "UnityFS directory",
                format!("blocks end at {stored_offset}, the file has {total} bytes"),
            ));
        }
        let entry_count = r.i32()?;
        let entry_count = usize::try_from(entry_count)
            .map_err(|_| Error::malformed("UnityFS directory", "negative entry count"))?;
        let mut entries = Vec::with_capacity(entry_count.min(r.remaining() / 21));
        for _ in 0..entry_count {
            let offset = r.i64()?;
            let size = r.i64()?;
            let flags = r.u32()?;
            let path = r.cstring()?;
            let (Ok(offset), Ok(size)) = (u64::try_from(offset), u64::try_from(size)) else {
                return Err(Error::malformed(
                    "UnityFS directory",
                    format!("entry {path} has a negative range"),
                ));
            };
            if offset
                .checked_add(size)
                .is_none_or(|end| end > stream_offset)
            {
                return Err(Error::malformed(
                    "UnityFS directory",
                    format!("entry {path} lies outside the data"),
                ));
            }
            entries.push(Entry {
                offset,
                size,
                flags,
                path,
            });
        }

        Ok(Bundle {
            source,
            id,
            header,
            blocks,
            entries,
            stream_len: stream_offset,
        })
    }

    /// The header.
    pub fn header(&self) -> &BundleHeader {
        &self.header
    }

    /// The block table.
    pub fn blocks(&self) -> impl ExactSizeIterator<Item = BlockInfo> + '_ {
        self.blocks.iter().map(|b| b.info)
    }

    /// The directory entries, in stored order.
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The SHA-256 of the file's bytes, as given to [`Bundle::open`].
    pub fn id(&self) -> &[u8; 32] {
        &self.id
    }

    /// A byte source over one entry. Reads decompress the blocks they touch, through `cache`.
    ///
    /// # Panics
    ///
    /// Panics if `index` is not an index of [`Bundle::entries`].
    pub fn entry<'a>(&'a self, index: usize, cache: &'a dyn BlockCache) -> EntrySource<'a> {
        let entry = &self.entries[index];
        EntrySource {
            bundle: self,
            cache,
            offset: entry.offset,
            size: entry.size,
        }
    }

    /// The decompressed block at `index`, from the cache or decompressed now.
    fn block(&self, index: usize, cache: &dyn BlockCache) -> Result<Chunk<'_>> {
        let block = &self.blocks[index];
        let read_stored = || {
            self.source
                .read(block.stored_offset, block.info.compressed_size as usize)
        };
        if block.compression == Compression::None {
            if block.info.compressed_size != block.info.uncompressed_size {
                return Err(Error::Decompress {
                    block: index as u32,
                    message: "an uncompressed block declares two sizes".into(),
                });
            }
            return read_stored();
        }
        let key = BlockKey {
            bundle: self.id,
            block: index as u32,
        };
        if let Some(hit) = cache.get(&key) {
            let end = <[u8]>::len(&hit);
            return Ok(Chunk::Shared {
                block: hit,
                start: 0,
                end,
            });
        }
        let stored = read_stored()?;
        let bytes: Arc<[u8]> = decompress(
            block.compression,
            &stored,
            block.info.uncompressed_size as usize,
        )
        .map_err(|message| Error::Decompress {
            block: index as u32,
            message,
        })?
        .into();
        cache.insert(key, bytes.clone());
        let end = <[u8]>::len(&bytes);
        Ok(Chunk::Shared {
            block: bytes,
            start: 0,
            end,
        })
    }

    /// The index of the block that contains stream offset `offset`.
    fn block_at(&self, offset: u64) -> usize {
        self.blocks
            .partition_point(|b| b.stream_offset + u64::from(b.info.uncompressed_size) <= offset)
    }
}

/// A byte source over one entry of a [`Bundle`].
#[derive(Clone, Copy)]
pub struct EntrySource<'a> {
    bundle: &'a Bundle,
    cache: &'a dyn BlockCache,
    offset: u64,
    size: u64,
}

impl std::fmt::Debug for EntrySource<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntrySource")
            .field("offset", &self.offset)
            .field("size", &self.size)
            .finish()
    }
}

impl ByteSource for EntrySource<'_> {
    fn len(&self) -> u64 {
        self.size
    }

    fn read(&self, offset: u64, len: usize) -> Result<Chunk<'_>> {
        check_range("bundle entry", self.size, offset, len)?;
        let start = self.offset + offset;
        let end = start + len as u64;
        debug_assert!(end <= self.bundle.stream_len);
        if len == 0 {
            return Ok(Chunk::Borrowed(&[]));
        }
        let first = self.bundle.block_at(start);
        let last = self.bundle.block_at(end - 1);
        if first == last {
            let block = self.bundle.block(first, self.cache)?;
            let base = self.bundle.blocks[first].stream_offset;
            return Ok(sub(block, (start - base) as usize, (end - base) as usize));
        }
        let mut out = Vec::with_capacity(len);
        for index in first..=last {
            let block = self.bundle.block(index, self.cache)?;
            let base = self.bundle.blocks[index].stream_offset;
            let from = start.max(base) - base;
            let to = end.min(base + block.len() as u64) - base;
            out.extend_from_slice(&block[from as usize..to as usize]);
        }
        Ok(Chunk::Owned(out))
    }
}

fn sub(chunk: Chunk<'_>, start: usize, end: usize) -> Chunk<'_> {
    match chunk {
        Chunk::Borrowed(bytes) => Chunk::Borrowed(&bytes[start..end]),
        Chunk::Shared {
            block,
            start: base,
            end: _,
        } => Chunk::Shared {
            block,
            start: base + start,
            end: base + end,
        },
        Chunk::Owned(bytes) => Chunk::Owned(bytes[start..end].to_vec()),
    }
}

/// A file to store in a new UnityFS container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFile {
    /// The entry path.
    pub path: String,
    /// The bytes of the file.
    pub bytes: Vec<u8>,
    /// Whether the file is a serialized file.
    pub serialized: bool,
}

/// A UnityFS container of `files`, in the given order, each one block without compression.
pub fn store_files(files: &[StoredFile]) -> Result<Vec<u8>> {
    let directory = stored_directory(files)?;
    let directory_size = u32::try_from(directory.len())
        .map_err(|_| Error::malformed("UnityFS directory", "too large to store"))?;
    let mut out = Vec::new();
    out.extend_from_slice(SIGNATURE);
    out.push(0);
    out.extend_from_slice(&STORED_FORMAT_VERSION.to_be_bytes());
    // Neither the engine version nor its revision is known here: serialized files state their
    // own.
    out.extend_from_slice(&[0, 0]);
    let size_at = out.len();
    out.extend_from_slice(&0u64.to_be_bytes());
    out.extend_from_slice(&directory_size.to_be_bytes());
    out.extend_from_slice(&directory_size.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&directory);
    for file in files {
        out.extend_from_slice(&file.bytes);
    }
    let size = out.len() as u64;
    out[size_at..size_at + size_of::<u64>()].copy_from_slice(&size.to_be_bytes());
    Ok(out)
}

/// The directory of a container of `files`, without compression.
fn stored_directory(files: &[StoredFile]) -> Result<Vec<u8>> {
    let count = i32::try_from(files.len())
        .map_err(|_| Error::malformed("UnityFS directory", "too many files to store"))?;
    let mut blocks = Vec::new();
    let mut entries = Vec::new();
    let mut offset = 0u64;
    for file in files {
        let size = u32::try_from(file.bytes.len()).map_err(|_| {
            Error::malformed("UnityFS directory", format!("{} is too large", file.path))
        })?;
        blocks.extend_from_slice(&size.to_be_bytes());
        blocks.extend_from_slice(&size.to_be_bytes());
        blocks.extend_from_slice(&0u16.to_be_bytes());
        entries.extend_from_slice(&offset.to_be_bytes());
        entries.extend_from_slice(&u64::from(size).to_be_bytes());
        let flags = if file.serialized { ENTRY_SERIALIZED } else { 0 };
        entries.extend_from_slice(&flags.to_be_bytes());
        entries.extend_from_slice(file.path.as_bytes());
        entries.push(0);
        offset += u64::from(size);
    }
    let mut directory = vec![0; DIRECTORY_HASH];
    directory.extend_from_slice(&count.to_be_bytes());
    directory.append(&mut blocks);
    directory.extend_from_slice(&count.to_be_bytes());
    directory.append(&mut entries);
    Ok(directory)
}

/// Decompresses `stored` into exactly `size` bytes.
fn decompress(
    compression: Compression,
    stored: &[u8],
    size: usize,
) -> std::result::Result<Vec<u8>, String> {
    match compression {
        Compression::None => {
            if stored.len() != size {
                return Err(format!(
                    "{} stored bytes for {size} uncompressed",
                    stored.len()
                ));
            }
            Ok(stored.to_vec())
        }
        Compression::Lz4 => {
            let mut out = vec![0; size];
            let written =
                lz4_flex::block::decompress_into(stored, &mut out).map_err(|e| e.to_string())?;
            if written != size {
                return Err(format!("decompressed to {written} of {size} bytes"));
            }
            Ok(out)
        }
        Compression::Lzma => {
            let mut out = Vec::with_capacity(size);
            let options = lzma_rs::decompress::Options {
                unpacked_size: lzma_rs::decompress::UnpackedSize::UseProvided(Some(size as u64)),
                ..Default::default()
            };
            lzma_rs::lzma_decompress_with_options(&mut &*stored, &mut out, &options)
                .map_err(|e| e.to_string())?;
            if out.len() != size {
                return Err(format!("decompressed to {} of {size} bytes", out.len()));
            }
            Ok(out)
        }
    }
}
