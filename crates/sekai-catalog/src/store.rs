//! The content-addressed store in a local directory.
//!
//! An object is named `<aa>/<sha256>.<extension>`, where `<aa>` is the first two hexadecimal
//! digits of the SHA-256 of its bytes. The name carries no other meaning.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
    /// again. An object is either absent or complete: it is written under a temporary name and
    /// then renamed.
    pub fn put(&self, bytes: &[u8], extension: &str) -> io::Result<Key> {
        let key = Key::of(bytes, extension);
        let path = self.path(&key);
        if path.is_file() {
            return Ok(key);
        }
        let temporary = self.temporary();
        std::fs::write(&temporary, bytes)?;
        self.place(&temporary, &path)?;
        Ok(key)
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
        let source = self.path(key);
        let parent = destination
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no parent directory"))?;
        std::fs::create_dir_all(parent)?;
        // The temporary name is in the destination's directory, so the rename does not cross
        // file systems.
        let name = destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let temporary = parent.join(format!(".{name}.{}.tmp", self.unique()));
        if std::fs::hard_link(&source, &temporary).is_err() {
            std::fs::copy(&source, &temporary)?;
        }
        std::fs::rename(&temporary, destination).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
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

    fn place(&self, temporary: &Path, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::rename(temporary, path).inspect_err(|_| {
            let _ = std::fs::remove_file(temporary);
        })
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
}
