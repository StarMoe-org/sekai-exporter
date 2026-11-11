//! The `apk` source, which reads the player data of a client package.
//!
//! A client package is an APK, or an XAPK that holds several APKs. Player data lies under
//! `assets/bin/Data/`, as a UnityFS container or as loose files, and a large loose file is stored
//! in the parts `<name>.split0`, `<name>.split1` and so on. Either way it becomes one UnityFS
//! container, a bundle like any other, whose entries the files name each other by.

use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Seek};
use std::sync::Arc;

use sekai_unity::bundle::is_unityfs;
use sekai_unity::{Bundle, ByteSource, NoBlockCache, SerializedFile, StoredFile, store_files};
use zip::ZipArchive;

/// The name of the bundle that holds the player data.
pub const PLAYER_BUNDLE: &str = "player";

const DATA_DIRECTORY: &str = "assets/bin/Data/";
const APK_EXTENSION: &str = ".apk";
/// The marker of a part of a file, followed by the number of the part.
const SPLIT_MARKER: &str = ".split";
/// The extensions of the files that hold the large data of serialized files.
const RESOURCE_EXTENSIONS: [&str; 2] = [".resS", ".resource"];
/// Maximum archive nesting depth (`0` = `.apk`/`.xapk`, `1` = `.apk` inside `.xapk`).
const MAX_ARCHIVE_DEPTH: usize = 1;
/// Upper bound on total uncompressed bytes read from a client package (`8 GiB`).
const MAX_UNPACKED_BYTES: u64 = 8 << 30;

/// The parts of each file under the data directory, by the path under it and the part number.
type Parts = BTreeMap<String, BTreeMap<usize, Vec<u8>>>;

/// The player data of the client package `package`, as the bytes of one UnityFS container.
pub fn player_bundle(package: &[u8]) -> io::Result<Vec<u8>> {
    player_bundle_from(Cursor::new(package))
}

/// Streams the player data from a seekable client package `reader` into one UnityFS container.
pub fn player_bundle_from<R: Read + Seek>(reader: R) -> io::Result<Vec<u8>> {
    let mut parts = Parts::new();
    let mut remaining = MAX_UNPACKED_BYTES;
    add_package(reader, 0, &mut remaining, &mut parts)?;
    let mut files = Vec::new();
    for (path, parts) in parts {
        add_file(&path, joined(&path, parts)?, &mut files)?;
    }
    if !files.iter().any(|file| file.serialized) {
        return Err(io::Error::other("the package holds no player data"));
    }
    store_files(&files).map_err(io::Error::other)
}

/// Adds the data files of an APK, or of each APK of an XAPK, to `parts`.
fn add_package<R: Read + Seek>(
    reader: R,
    depth: usize,
    remaining: &mut u64,
    parts: &mut Parts,
) -> io::Result<()> {
    let mut archive = ZipArchive::new(reader)?;
    for index in 0..archive.len() {
        let mut member = archive.by_index(index)?;
        let name = member.name()?.into_owned();
        if name.ends_with('/') {
            continue;
        }
        if name.to_ascii_lowercase().ends_with(APK_EXTENSION) {
            if depth >= MAX_ARCHIVE_DEPTH {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("nested APK {name} exceeds maximum archive depth"),
                ));
            }
            let nested = contents(&mut member, remaining)?;
            add_package(Cursor::new(nested), depth + 1, remaining, parts)?;
        } else if let Some(path) = name.strip_prefix(DATA_DIRECTORY) {
            let (file, part) = file_and_part(path);
            let bytes = contents(&mut member, remaining)?;
            let added = parts
                .entry(file.to_owned())
                .or_default()
                .insert(part, bytes);
            if added.is_some() {
                return Err(io::Error::other(format!("{path} is in two packages")));
            }
        }
    }
    Ok(())
}

fn contents(member: &mut impl Read, remaining: &mut u64) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let limit = remaining.saturating_add(1);
    let read = member.take(limit).read_to_end(&mut bytes)? as u64;
    if read > *remaining {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "client package exceeds maximum unpacked size",
        ));
    }
    *remaining -= read;
    Ok(bytes)
}

/// The file a path under the data directory belongs to, and the number of its part.
fn file_and_part(path: &str) -> (&str, usize) {
    path.rsplit_once(SPLIT_MARKER)
        .and_then(|(file, part)| Some((file, part.parse().ok()?)))
        .unwrap_or((path, 0))
}

/// The bytes of a file from its parts, which must be numbered from 0 without a gap.
fn joined(path: &str, mut parts: BTreeMap<usize, Vec<u8>>) -> io::Result<Vec<u8>> {
    if !parts.keys().copied().eq(0..parts.len()) {
        return Err(io::Error::other(format!("{path} is missing a part")));
    }
    if parts.len() == 1 {
        return Ok(parts.remove(&0).unwrap_or_default());
    }
    let total: usize = parts.values().map(Vec::len).sum();
    let mut out = Vec::with_capacity(total);
    for chunk in parts.into_values() {
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// Adds what the file at `path` holds of the player data to `files`: the entries of a container,
/// or the file itself when it is a serialized file or holds the large data of one.
fn add_file(path: &str, bytes: Vec<u8>, files: &mut Vec<StoredFile>) -> io::Result<()> {
    if is_unityfs(&bytes) {
        return add_entries(bytes, files);
    }
    let name = path.rsplit_once('/').map_or(path, |(_, name)| name);
    let serialized = SerializedFile::parse(&bytes).is_ok();
    if serialized || RESOURCE_EXTENSIONS.iter().any(|e| name.ends_with(e)) {
        files.push(StoredFile {
            path: name.to_owned(),
            bytes,
            serialized,
        });
    }
    Ok(())
}

/// Adds every entry of the UnityFS container `bytes` to `files`.
fn add_entries(bytes: Vec<u8>, files: &mut Vec<StoredFile>) -> io::Result<()> {
    let source: Arc<dyn ByteSource> = Arc::new(Arc::<[u8]>::from(bytes));
    let bundle = Bundle::open(source, [0; 32]).map_err(io::Error::other)?;
    for (index, entry) in bundle.entries().iter().enumerate() {
        let size = usize::try_from(entry.size).map_err(io::Error::other)?;
        let source = bundle.entry(index, &NoBlockCache);
        let bytes = source.read(0, size).map_err(io::Error::other)?;
        files.push(StoredFile {
            path: entry.path.clone(),
            bytes: bytes.to_vec(),
            serialized: entry.is_serialized(),
        });
    }
    Ok(())
}
