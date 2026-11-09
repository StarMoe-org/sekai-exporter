//! TPK files: the type tree information of engine classes.
//!
//! A TPK file is a 20-byte header and one blob, compressed as the header says. The blob holds the
//! Unity versions it covers, the classes with an entry per version range, the common strings, the
//! nodes of every layout and the strings the nodes name.

use std::collections::BTreeMap;

use lzma_rs::decompress::{Options, UnpackedSize};
use sha2::{Digest, Sha256};

use crate::buffer::Buffer;
use crate::error::TableError;
use crate::table::{ClassVersion, Node, VersionTable};

/// `TPK*` as a little-endian number.
const MAGIC: u32 = 0x2A4B_5054;
const FORMAT_VERSIONS: [u8; 2] = [1, 2];
const TYPE_TREE_INFORMATION: u8 = 0;
const NO_COMPRESSION: u8 = 0;
const LZMA: u8 = 2;
/// The class flag of a class with a root node for the editor.
const HAS_EDITOR_ROOT: u8 = 0x40;
/// The class flag of a class with a root node for the player.
const HAS_RELEASE_ROOT: u8 = 0x80;

/// Reads a TPK file that holds type tree information.
pub(crate) fn read(file: &[u8]) -> Result<VersionTable, TableError> {
    let mut header = Buffer::new(file);
    if header.u32()? != MAGIC {
        return Err(TableError::new("the file does not start with `TPK*`"));
    }
    let format_version = header.byte()?;
    if !FORMAT_VERSIONS.contains(&format_version) {
        return Err(TableError::new(format!(
            "format version {format_version} is not supported"
        )));
    }
    let compression = header.byte()?;
    let data_type = header.byte()?;
    if data_type != TYPE_TREE_INFORMATION {
        return Err(TableError::new(format!(
            "data type {data_type} is not type tree information"
        )));
    }
    header.skip(5)?;
    let stored = header.u32()? as usize;
    let size = header.u32()? as usize;
    let blob = decompress(compression, header.bytes(stored)?, size)?;
    read_blob(
        &mut Buffer::new(&blob),
        format_version,
        Sha256::digest(file).into(),
    )
}

fn decompress(compression: u8, stored: &[u8], size: usize) -> Result<Vec<u8>, TableError> {
    let blob = match compression {
        NO_COMPRESSION => stored.to_vec(),
        LZMA => decompress_lzma(stored, size)?,
        other => {
            return Err(TableError::new(format!(
                "compression type {other} is not supported"
            )));
        }
    };
    if blob.len() != size {
        return Err(TableError::new(format!(
            "the blob has {} bytes instead of {size}",
            blob.len()
        )));
    }
    Ok(blob)
}

/// Decompresses LZMA data that starts with its properties, without a stored length.
fn decompress_lzma(mut stored: &[u8], size: usize) -> Result<Vec<u8>, TableError> {
    let options = Options {
        unpacked_size: UnpackedSize::UseProvided(Some(size as u64)),
        ..Options::default()
    };
    let mut blob = Vec::with_capacity(size);
    lzma_rs::lzma_decompress_with_options(&mut stored, &mut blob, &options)
        .map_err(|e| TableError::new(format!("LZMA: {e}")))?;
    Ok(blob)
}

fn read_blob(
    blob: &mut Buffer<'_>,
    format_version: u8,
    sha256: [u8; 32],
) -> Result<VersionTable, TableError> {
    blob.skip(8)?; // the creation time
    blob.skip_list(8)?; // the Unity versions
    let classes = read_classes(blob)?;
    skip_common_strings(blob, format_version)?;
    let nodes = (0..blob.count()?)
        .map(|_| read_node(blob))
        .collect::<Result<_, _>>()?;
    let strings = (0..blob.count()?)
        .map(|_| blob.string())
        .collect::<Result<_, _>>()?;
    Ok(VersionTable {
        classes,
        nodes,
        strings,
        sha256,
    })
}

fn read_classes(blob: &mut Buffer<'_>) -> Result<BTreeMap<i32, Vec<ClassVersion>>, TableError> {
    let mut classes = BTreeMap::new();
    for _ in 0..blob.count()? {
        let id = blob.i32()?;
        let versions = (0..blob.count()?)
            .map(|_| read_class_version(blob))
            .collect::<Result<_, _>>()?;
        classes.insert(id, versions);
    }
    Ok(classes)
}

fn read_class_version(blob: &mut Buffer<'_>) -> Result<ClassVersion, TableError> {
    let from = blob.u64()?;
    let exists = blob.byte()? != 0;
    Ok(ClassVersion {
        from,
        release_root: if exists {
            read_release_root(blob)?
        } else {
            None
        },
    })
}

/// Reads a class of one version range, for the root node of its release layout.
fn read_release_root(blob: &mut Buffer<'_>) -> Result<Option<u16>, TableError> {
    blob.skip(4)?; // the name and the base of the class
    let flags = blob.byte()?;
    if flags & HAS_EDITOR_ROOT != 0 {
        blob.skip(2)?;
    }
    (flags & HAS_RELEASE_ROOT != 0)
        .then(|| blob.u16())
        .transpose()
}

/// Skips the common strings, whose form depends on the version of the file.
fn skip_common_strings(blob: &mut Buffer<'_>, format_version: u8) -> Result<(), TableError> {
    if format_version == 1 {
        blob.skip_list(9)?;
        return blob.skip_list(2);
    }
    for _ in 0..blob.count()? {
        blob.skip(8)?;
        blob.skip_list(4)?;
    }
    Ok(())
}

fn read_node(blob: &mut Buffer<'_>) -> Result<Node, TableError> {
    Ok(Node {
        type_name: blob.u16()?,
        name: blob.u16()?,
        byte_size: blob.i32()?,
        version: blob.u16()?,
        type_flags: blob.byte()?,
        meta_flags: blob.u32()?,
        subnodes: (0..blob.u16()?)
            .map(|_| blob.u16())
            .collect::<Result<_, _>>()?,
    })
}
