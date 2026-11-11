//! The content-addressed store in a local directory.
//!
//! An object is named `<aa>/<sha256>.<extension>`, where `<aa>` is the first two hexadecimal
//! digits of the SHA-256 of its bytes. The name carries no other meaning.

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use sha2::{Digest, Sha256};

/// The name of an object in the store: the SHA-256 of its bytes and its file extension.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    /// The SHA-256 of the bytes.
    pub sha256: [u8; 32],
    /// The file extension, without the dot, such as `webp` or `json`.
    pub extension: String,
}

impl Key {
    /// The key of `bytes` with `extension`.
    pub fn of(bytes: &[u8], extension: &str) -> Self {
        Self {
            sha256: Sha256::digest(bytes).into(),
            extension: extension.to_owned(),
        }
    }

    /// The path of the object relative to the store root: `<aa>/<sha256>.<extension>`.
    pub fn relative_path(&self) -> String {
        let hex = hex(&self.sha256);
        format!("{}/{hex}.{}", &hex[..2], self.extension)
    }
}

/// The name of a key is not `<sha256>.<extension>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidKey(pub String);

impl fmt::Display for InvalidKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} is not the name of a store object", self.0)
    }
}

impl std::error::Error for InvalidKey {}

impl FromStr for Key {
    type Err = InvalidKey;

    fn from_str(name: &str) -> Result<Self, InvalidKey> {
        let invalid = || InvalidKey(name.to_owned());
        let (hash, extension) = name.split_once('.').ok_or_else(invalid)?;
        Ok(Self {
            sha256: parse_hex(hash).ok_or_else(invalid)?,
            extension: extension.to_owned(),
        })
    }
}

/// The 32 bytes written as lowercase hexadecimal in `text`.
pub fn parse_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut bytes = [0; 32];
    let (pairs, _) = text.as_bytes().as_chunks::<2>();
    for (byte, pair) in bytes.iter_mut().zip(pairs) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(bytes)
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", hex(&self.sha256), self.extension)
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A store in a local directory.
#[derive(Debug)]
pub struct LocalStore {
    root: PathBuf,
    /// Numbers the temporary files of this process.
    next: AtomicU64,
}

/// The directory under the root where objects are written before they are renamed into place.
const INCOMING: &str = "incoming";

/// Where derivations write their files.
pub trait Store: Sync {
    /// Stores `bytes` and returns their key.
    fn put(&self, bytes: &[u8], extension: &str) -> io::Result<Key>;
    /// The bytes stored under `key`.
    fn read(&self, key: &Key) -> io::Result<Vec<u8>>;
    /// The size of the bytes stored under `key`, or `None` when nothing is stored under it.
    fn size(&self, key: &Key) -> io::Result<Option<u64>>;
}

impl Store for LocalStore {
    fn put(&self, bytes: &[u8], extension: &str) -> io::Result<Key> {
        LocalStore::put(self, bytes, extension)
    }

    fn size(&self, key: &Key) -> io::Result<Option<u64>> {
        match std::fs::metadata(self.path(key)) {
            Ok(metadata) => Ok(metadata.is_file().then_some(metadata.len())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn read(&self, key: &Key) -> io::Result<Vec<u8>> {
        LocalStore::read(self, key)
    }
}

/// A store in memory.
#[derive(Debug, Default)]
pub struct MemoryStore(Mutex<BTreeMap<Key, Vec<u8>>>);

impl Store for MemoryStore {
    fn put(&self, bytes: &[u8], extension: &str) -> io::Result<Key> {
        let key = Key::of(bytes, extension);
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), bytes.to_vec());
        Ok(key)
    }

    fn read(&self, key: &Key) -> io::Result<Vec<u8>> {
        MemoryStore::read(self, key)
    }

    fn size(&self, key: &Key) -> io::Result<Option<u64>> {
        let files = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        Ok(files.get(key).map(|bytes| bytes.len() as u64))
    }
}

impl MemoryStore {
    /// The bytes of `key`.
    pub fn read(&self, key: &Key) -> io::Result<Vec<u8>> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, key.to_string()))
    }

    /// Removes every file and returns them, in key order.
    pub fn drain(&self) -> Vec<(Key, Vec<u8>)> {
        let files = std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner));
        files.into_iter().collect()
    }
}

impl LocalStore {
    /// Opens the store at `root`, creating the directory if it does not exist.
    pub fn open(root: impl Into<PathBuf>) -> io::Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(root.join(INCOMING))?;
        Ok(Self {
            root,
            next: AtomicU64::new(0),
        })
    }

    /// The path of an object, whether or not it exists.
    pub fn path(&self, key: &Key) -> PathBuf {
        self.root.join(key.relative_path())
    }

    /// Whether the store has an object named `key`.
    pub fn contains(&self, key: &Key) -> bool {
        self.path(key).is_file()
    }

    /// Writes `bytes` and returns their key. Content that is already stored is not written
    /// again. An object is either absent or complete: it is written under a temporary name in
    /// its own directory and then renamed.
    pub fn put(&self, bytes: &[u8], extension: &str) -> io::Result<Key> {
        let key = Key::of(bytes, extension);
        let path = self.path(&key);
        if holds(&path, bytes.len() as u64) {
            return Ok(key);
        }
        let temporary = self.beside(&path)?;
        std::fs::write(&temporary, bytes)?;
        rename(&temporary, &path)?;
        Ok(key)
    }

    /// A writer for an object whose bytes arrive in parts.
    pub fn writer(&self, extension: &str) -> io::Result<ObjectWriter<'_>> {
        let temporary = Temporary(self.temporary());
        Ok(ObjectWriter {
            store: self,
            file: BufWriter::new(File::create(&temporary.0)?),
            temporary,
            hash: Sha256::new(),
            size: 0,
            extension: extension.to_owned(),
        })
    }

    /// The keys of the objects with `extension`, in key order.
    pub fn keys(&self, extension: &str) -> io::Result<Vec<Key>> {
        let suffix = format!(".{extension}");
        let mut keys = Vec::new();
        for directory in std::fs::read_dir(&self.root)? {
            let directory = directory?;
            if !directory.file_type()?.is_dir() || directory.file_name() == INCOMING {
                continue;
            }
            for file in std::fs::read_dir(directory.path())? {
                let name = file?.file_name();
                let name = name.to_string_lossy();
                if name.ends_with(&suffix)
                    && let Ok(key) = name.parse()
                {
                    keys.push(key);
                }
            }
        }
        keys.sort();
        Ok(keys)
    }

    /// The sizes of the objects whose hashes start with the byte `prefix`, by key. Listing their
    /// directory opens no object, so it costs far less than asking for each object in turn.
    pub fn sizes(&self, prefix: u8) -> io::Result<HashMap<Key, u64>> {
        let entries = match std::fs::read_dir(self.root.join(format!("{prefix:02x}"))) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
            entries => entries?,
        };
        let mut sizes = HashMap::new();
        for entry in entries {
            let entry = entry?;
            // Temporary files have names that are not keys.
            let Ok(key) = entry.file_name().to_string_lossy().parse() else {
                continue;
            };
            let metadata = entry.metadata()?;
            if metadata.is_file() {
                sizes.insert(key, metadata.len());
            }
        }
        Ok(sizes)
    }

    /// Reads an object.
    pub fn read(&self, key: &Key) -> io::Result<Vec<u8>> {
        std::fs::read(self.path(key))
    }

    /// Whether the object exists and its bytes have the SHA-256 of its key.
    pub fn verify(&self, key: &Key) -> io::Result<bool> {
        match std::fs::read(self.path(key)) {
            Ok(bytes) => Ok(<[u8; 32]>::from(Sha256::digest(&bytes)) == key.sha256),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Places the object `key` at `destination`, replacing any file there: a hard link when
    /// the file system allows it, otherwise a copy. The replacement is atomic.
    pub fn materialize(&self, key: &Key, destination: &Path) -> io::Result<()> {
        self.link_or_copy(&self.path(key), destination)
    }

    /// Stores the file at `source`, whose bytes are those of `key`, under `key`: a hard link
    /// when the file system allows it, otherwise a copy. An object that is already stored is
    /// left as it is.
    pub fn adopt(&self, key: &Key, source: &Path) -> io::Result<()> {
        let path = self.path(key);
        if path.is_file() {
            return Ok(());
        }
        self.link_or_copy(source, &path)
    }

    /// Places `source` at `destination` atomically, as a hard link or a copy.
    fn link_or_copy(&self, source: &Path, destination: &Path) -> io::Result<()> {
        if link_new(source, destination) {
            return Ok(());
        }
        let temporary = self.beside(destination)?;
        if std::fs::hard_link(source, &temporary).is_err() {
            std::fs::copy(source, &temporary)?;
        }
        rename(&temporary, destination)
    }

    /// A temporary name in the directory of `path`, which is created if it does not exist. A
    /// rename within one directory never crosses file systems, and it is cheaper than one
    /// across directories.
    fn beside(&self, path: &Path) -> io::Result<PathBuf> {
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent directory"))?;
        std::fs::create_dir_all(parent)?;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        Ok(parent.join(format!(".{name}.{}.tmp", self.unique())))
    }

    fn temporary(&self) -> PathBuf {
        self.root
            .join(INCOMING)
            .join(format!("{}.tmp", self.unique()))
    }

    /// A name no other write of this process or of another process uses.
    fn unique(&self) -> String {
        let n = self.next.fetch_add(1, Ordering::Relaxed);
        format!("{}-{n}", std::process::id())
    }

    /// Names the complete file `temporary` of `size` bytes as `path`, replacing what is there
    /// unless another writer has just placed the same content. A hard link to a new name appears
    /// at once, as a rename would, and it is cheaper than a rename across directories; the
    /// temporary name is left for the caller to remove.
    fn place(&self, temporary: &Path, path: &Path, size: u64) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        match std::fs::hard_link(temporary, path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && holds(path, size) => Ok(()),
            Err(_) => std::fs::rename(temporary, path),
        }
    }
}

/// Whether `source` is now linked at `destination`, a name that did not exist; its directory is
/// created if needed. A link appears complete at once, so a new name needs no temporary one.
fn link_new(source: &Path, destination: &Path) -> bool {
    match std::fs::hard_link(source, destination) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            destination
                .parent()
                .is_some_and(|parent| std::fs::create_dir_all(parent).is_ok())
                && std::fs::hard_link(source, destination).is_ok()
        }
        linked => linked.is_ok(),
    }
}

/// Whether `path` is a file of `size` bytes. An object under its name with another size is the
/// remains of an interrupted write, and is written again.
fn holds(path: &Path, size: u64) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && metadata.len() == size)
}

/// Renames `temporary` to `path`, removing it when that fails.
fn rename(temporary: &Path, path: &Path) -> io::Result<()> {
    std::fs::rename(temporary, path).inspect_err(|_| {
        let _ = std::fs::remove_file(temporary);
    })
}

/// A temporary file, removed when it is dropped.
struct Temporary(PathBuf);

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// An object being written. It joins the store when it is finished, and leaves nothing behind
/// otherwise.
pub struct ObjectWriter<'s> {
    store: &'s LocalStore,
    temporary: Temporary,
    file: BufWriter<File>,
    hash: Sha256,
    size: u64,
    extension: String,
}

impl ObjectWriter<'_> {
    /// Forgets everything written so far.
    pub fn reset(&mut self) -> io::Result<()> {
        // Buffered bytes must reach the file before it is truncated, not when the writer drops.
        self.file.flush()?;
        self.file = BufWriter::new(File::create(&self.temporary.0)?);
        self.hash = Sha256::new();
        self.size = 0;
        Ok(())
    }

    /// Places the object in the store and returns its key and size. Content that is already
    /// stored is left as it is.
    pub fn finish(self) -> io::Result<(Key, u64)> {
        let Self {
            store,
            temporary,
            file,
            hash,
            size,
            extension,
        } = self;
        file.into_inner().map_err(io::IntoInnerError::into_error)?;
        let key = Key {
            sha256: hash.finalize().into(),
            extension,
        };
        let path = store.path(&key);
        if !holds(&path, size) {
            store.place(&temporary.0, &path, size)?;
        }
        Ok((key, size))
    }
}

impl Write for ObjectWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let written = self.file.write(bytes)?;
        self.hash.update(&bytes[..written]);
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (PathBuf, LocalStore) {
        let root = std::env::temp_dir().join(format!("sekai-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let store = LocalStore::open(&root).unwrap();
        (root, store)
    }

    #[test]
    fn put_names_bytes_by_their_hash() {
        let (root, store) = store("put");
        let key = store.put(b"abc", "txt").unwrap();
        // The SHA-256 of "abc".
        let expected = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(key.relative_path(), format!("ba/{expected}.txt"));
        assert_eq!(store.read(&key).unwrap(), b"abc");
        assert!(store.verify(&key).unwrap());
        assert_eq!(store.put(b"abc", "txt").unwrap(), key);
        assert!(!store.contains(&Key::of(b"abd", "txt")));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_object_cut_short_is_written_again() {
        let (root, store) = store("cut");
        let key = store.put(b"abc", "txt").unwrap();
        std::fs::write(store.path(&key), b"").unwrap();
        assert_eq!(store.put(b"abc", "txt").unwrap(), key);
        assert_eq!(store.read(&key).unwrap(), b"abc");
        std::fs::write(store.path(&key), b"a").unwrap();
        let mut writer = store.writer("txt").unwrap();
        writer.write_all(b"abc").unwrap();
        assert_eq!(writer.finish().unwrap(), (key.clone(), 3));
        assert_eq!(store.read(&key).unwrap(), b"abc");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_writer_gives_the_key_of_its_bytes() {
        let (root, store) = store("writer");
        let mut writer = store.writer("bin").unwrap();
        writer.write_all(b"discarded").unwrap();
        writer.reset().unwrap();
        writer.write_all(b"ab").unwrap();
        writer.write_all(b"c").unwrap();
        let (key, size) = writer.finish().unwrap();
        assert_eq!(key, Key::of(b"abc", "bin"));
        assert_eq!(size, 3);
        assert_eq!(store.read(&key).unwrap(), b"abc");
        assert_eq!(std::fs::read_dir(root.join(INCOMING)).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_abandoned_writer_leaves_nothing() {
        let (root, store) = store("abandoned");
        let mut writer = store.writer("bin").unwrap();
        writer.write_all(b"abc").unwrap();
        drop(writer);
        assert_eq!(std::fs::read_dir(root.join(INCOMING)).unwrap().count(), 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn verify_reports_changed_and_missing_objects() {
        let (root, store) = store("verify");
        let key = store.put(b"abc", "txt").unwrap();
        std::fs::write(store.path(&key), b"abd").unwrap();
        assert!(!store.verify(&key).unwrap());
        assert!(!store.verify(&Key::of(b"none", "txt")).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn materialize_replaces_the_destination() {
        let (root, store) = store("materialize");
        let first = store.put(b"first", "bin").unwrap();
        let second = store.put(b"second", "bin").unwrap();
        let destination = root.join("tree").join("a").join("file.bin");
        store.materialize(&first, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"first");
        store.materialize(&second, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"second");
        assert_eq!(
            std::fs::read_dir(destination.parent().unwrap())
                .unwrap()
                .count(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sizes_list_the_objects_of_a_directory() {
        let (root, store) = store("sizes");
        let key = store.put(b"abc", "txt").unwrap();
        let directory = root.join(&key.relative_path()[..2]);
        std::fs::write(directory.join(".partial.1.tmp"), b"partial").unwrap();
        let sizes = store.sizes(key.sha256[0]).unwrap();
        assert_eq!(sizes, HashMap::from([(key.clone(), 3)]));
        assert!(store.sizes(key.sha256[0] ^ 1).unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
