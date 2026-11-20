//! The `apk` source, which reads the bundles of a client package.
//!
//! A client package is an APK, or an XAPK that holds several APKs. Player data lies under
//! `assets/bin/Data/`, as a UnityFS container or as loose files, and a large loose file is stored
//! in the parts `<name>.split0`, `<name>.split1` and so on. Either way it becomes one UnityFS
//! container, a bundle like any other, whose entries the files name each other by.
//!
//! The streaming assets lie directly under `assets/`. The cue sheets among them, which hold the
//! sounds the client plays from the package, become a second container whose entries are named by
//! their paths under `assets/`. They are the only CRI files a form reads: a cue sheet is converted
//! from its own bytes, so the other CRI files beside it are left out.
//!
//! The client also loads Unity bundles from its streaming assets without downloading them: bundles
//! of its own, such as the fonts of a client, and downloaded bundles shipped ahead in the package,
//! which keep the header of a download (see [`crate::obfuscation`]). Each is a bundle of the
//! package as it is, under the name its `AssetBundle` object gives it.

use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Seek};
use std::sync::Arc;

use sekai_unity::bundle::is_unityfs;
use sekai_unity::index::OpenFile;
use sekai_unity::{
    Bundle, ByteSource, NoBlockCache, NoClasses, SerializedFile, StoredFile, Value, open_files,
    store_files,
};
use zip::ZipArchive;

use crate::obfuscation::deobfuscate;

/// The name of the bundle that holds the player data.
pub const PLAYER_BUNDLE: &str = "player";
/// The name of the bundle that holds the cue sheets of the streaming assets.
pub const STREAMING_ASSETS_BUNDLE: &str = "streamingassets";

const DATA_DIRECTORY: &str = "assets/bin/Data/";
const STREAMING_ASSETS_DIRECTORY: &str = "assets/";
/// The extension of a cue sheet.
const CUE_SHEET_EXTENSION: &str = ".acb";
/// The class ID of the `AssetBundle` object that names a bundle.
const ASSET_BUNDLE: i32 = 142;
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

/// A bundle of a client package: its name and the bytes of its UnityFS container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageBundle {
    /// [`PLAYER_BUNDLE`], [`STREAMING_ASSETS_BUNDLE`], or the name that a Unity bundle of the
    /// streaming assets gives itself.
    pub name: String,
    /// The UnityFS container.
    pub bytes: Arc<Vec<u8>>,
}

/// The files of a client package that become bundles.
#[derive(Default)]
struct Members {
    /// The parts of the player data.
    data: Parts,
    /// The cue sheets of the streaming assets, by their paths under the streaming assets.
    streaming: BTreeMap<String, Vec<u8>>,
    /// The Unity bundles of the streaming assets as UnityFS containers, by their paths under the
    /// streaming assets.
    bundles: BTreeMap<String, Vec<u8>>,
}

/// The bundles of the client package `package`: the player data, then the cue sheets of the
/// streaming assets when it has any, then the Unity bundles of the streaming assets in name order.
pub fn bundles(package: &[u8]) -> io::Result<Vec<PackageBundle>> {
    bundles_from(Cursor::new(package))
}

/// The bundles of a seekable client package `reader`, as [`bundles`] gives them.
pub fn bundles_from<R: Read + Seek>(reader: R) -> io::Result<Vec<PackageBundle>> {
    let mut members = Members::default();
    let mut remaining = MAX_UNPACKED_BYTES;
    add_package(reader, 0, &mut remaining, &mut members)?;
    let mut bundles = vec![PackageBundle {
        name: PLAYER_BUNDLE.to_owned(),
        bytes: Arc::new(player_data(members.data)?),
    }];
    if !members.streaming.is_empty() {
        bundles.push(PackageBundle {
            name: STREAMING_ASSETS_BUNDLE.to_owned(),
            bytes: Arc::new(streaming_assets(members.streaming)?),
        });
    }
    bundles.extend(named_bundles(members.bundles)?);
    Ok(bundles)
}

/// The Unity bundles of the streaming assets, `files` by their paths, under the names they give
/// themselves, in name order. No two bundles of a package share a name.
fn named_bundles(files: BTreeMap<String, Vec<u8>>) -> io::Result<Vec<PackageBundle>> {
    let mut named = BTreeMap::new();
    for (path, bytes) in files {
        let bytes = Arc::new(bytes);
        let name = asset_bundle_name(&bytes).map_err(|e| in_file(&path, e))?;
        let taken = [PLAYER_BUNDLE, STREAMING_ASSETS_BUNDLE].contains(&name.as_str());
        if taken || named.contains_key(&name) {
            let message = format!("another bundle of the package is named {name}");
            return Err(in_file(&path, message));
        }
        named.insert(name, bytes);
    }
    let bundle = |(name, bytes)| PackageBundle { name, bytes };
    Ok(named.into_iter().map(bundle).collect())
}

/// The name that the `AssetBundle` object of the UnityFS container `bytes` gives its bundle.
fn asset_bundle_name(bytes: &Arc<Vec<u8>>) -> Result<String, String> {
    let bundle = Bundle::open(bytes.clone(), [0; 32]).map_err(|e| e.to_string())?;
    let files = open_files(&bundle, &NoBlockCache, &NoClasses).map_err(|e| e.to_string())?;
    for (entry, file, layouts) in &files {
        let open = OpenFile {
            path: &bundle.entries()[*entry].path,
            file,
            source: &bundle.entry(*entry, &NoBlockCache),
            layouts,
        };
        let objects = file.objects.iter().enumerate();
        for (index, _) in objects.filter(|(_, object)| object.class_id == ASSET_BUNDLE) {
            let value = open.value(index).map_err(|e| e.to_string())?;
            if let Some(name) = value.get("m_AssetBundleName").and_then(Value::as_str) {
                return Ok(name.to_owned());
            }
        }
    }
    Err("no AssetBundle object names the bundle".to_owned())
}

fn in_file(path: &str, message: impl std::fmt::Display) -> io::Error {
    io::Error::other(format!("{path}: {message}"))
}

/// The player data as one UnityFS container.
fn player_data(parts: Parts) -> io::Result<Vec<u8>> {
    let mut files = Vec::new();
    for (path, parts) in parts {
        add_file(&path, joined(&path, parts)?, &mut files)?;
    }
    if !files.iter().any(|file| file.serialized) {
        return Err(io::Error::other("the package holds no player data"));
    }
    store_files(&files).map_err(io::Error::other)
}

/// The cue sheets of the streaming assets as one UnityFS container.
fn streaming_assets(files: BTreeMap<String, Vec<u8>>) -> io::Result<Vec<u8>> {
    let files: Vec<StoredFile> = files
        .into_iter()
        .map(|(path, bytes)| StoredFile {
            path,
            bytes,
            serialized: false,
        })
        .collect();
    store_files(&files).map_err(io::Error::other)
}

/// Whether the file at `path` is a cue sheet, by its extension in any case.
fn is_cue_sheet(path: &str) -> bool {
    let start = path.len().saturating_sub(CUE_SHEET_EXTENSION.len());
    path.get(start..)
        .is_some_and(|ending| ending.eq_ignore_ascii_case(CUE_SHEET_EXTENSION))
}

/// Adds the files of an APK, or of each APK of an XAPK, that become bundles to `members`.
fn add_package<R: Read + Seek>(
    reader: R,
    depth: usize,
    remaining: &mut u64,
    members: &mut Members,
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
            add_package(Cursor::new(nested), depth + 1, remaining, members)?;
        } else if let Some(path) = name.strip_prefix(DATA_DIRECTORY) {
            let (file, part) = file_and_part(path);
            let bytes = contents(&mut member, remaining)?;
            let parts = members.data.entry(file.to_owned()).or_default();
            if parts.insert(part, bytes).is_some() {
                return Err(in_two_packages(path));
            }
        } else if let Some(path) = name.strip_prefix(STREAMING_ASSETS_DIRECTORY) {
            let mut bytes = contents(&mut member, remaining)?;
            let kept = if is_cue_sheet(path) {
                &mut members.streaming
            } else if deobfuscate(&mut bytes).is_ok() {
                &mut members.bundles
            } else {
                continue;
            };
            if kept.insert(path.to_owned(), bytes).is_some() {
                return Err(in_two_packages(path));
            }
        }
    }
    Ok(())
}

fn in_two_packages(path: &str) -> io::Error {
    io::Error::other(format!("{path} is in two packages"))
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
