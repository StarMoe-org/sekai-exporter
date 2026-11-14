//! What a destination holds, and the objects of an output: its local store together with them.
//!
//! A run that publishes to a destination records each object the destination is confirmed to
//! hold, and only those count as held. They stay part of the output when their files leave the
//! local store: the local store works as a cache of the destination, and an object it lacks is
//! read back from the destination.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::store::{Key, LocalStore, MemoryStore, Store};

/// The objects a destination is confirmed to hold, with their sizes.
pub trait Holdings: Sync {
    /// The size of `key` at the destination, or `None` when it is not known to hold it.
    fn size(&self, key: &Key) -> io::Result<Option<u64>>;
    /// The objects whose hashes start with the byte `prefix`, with their sizes.
    fn sizes(&self, prefix: u8) -> io::Result<HashMap<Key, u64>>;
    /// Records that the destination holds `objects`.
    fn hold(&self, objects: &[(Key, u64)]) -> io::Result<()>;
}

/// No destination: it holds nothing, and nothing can be recorded for it.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoHoldings;

impl Holdings for NoHoldings {
    fn size(&self, _key: &Key) -> io::Result<Option<u64>> {
        Ok(None)
    }

    fn sizes(&self, _prefix: u8) -> io::Result<HashMap<Key, u64>> {
        Ok(HashMap::new())
    }

    fn hold(&self, _objects: &[(Key, u64)]) -> io::Result<()> {
        Err(io::Error::other("there is no destination to hold objects"))
    }
}

/// Holdings kept in memory.
#[derive(Debug, Default)]
pub struct MemoryHoldings(Mutex<BTreeMap<Key, u64>>);

impl MemoryHoldings {
    /// Holdings of exactly `objects`, such as a listing of the destination.
    pub fn new(objects: BTreeMap<Key, u64>) -> Self {
        Self(Mutex::new(objects))
    }

    fn objects(&self) -> MutexGuard<'_, BTreeMap<Key, u64>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Holdings for MemoryHoldings {
    fn size(&self, key: &Key) -> io::Result<Option<u64>> {
        Ok(self.objects().get(key).copied())
    }

    fn sizes(&self, prefix: u8) -> io::Result<HashMap<Key, u64>> {
        let objects = self.objects();
        let held = objects.iter().filter(|(key, _)| key.sha256[0] == prefix);
        Ok(held.map(|(key, &size)| (key.clone(), size)).collect())
    }

    fn hold(&self, objects: &[(Key, u64)]) -> io::Result<()> {
        self.objects().extend(objects.iter().cloned());
        Ok(())
    }
}

/// Where the bytes of objects that the local store lacks are read from.
pub trait Remote: Sync {
    /// The bytes of `key`, or `None` when the remote does not have them.
    fn fetch(&self, key: &Key) -> io::Result<Option<Vec<u8>>>;
}

/// No remote: every object is read from the local store.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRemote;

impl Remote for NoRemote {
    fn fetch(&self, _key: &Key) -> io::Result<Option<Vec<u8>>> {
        Ok(None)
    }
}

/// A remote of objects kept in memory.
impl Remote for MemoryStore {
    fn fetch(&self, key: &Key) -> io::Result<Option<Vec<u8>>> {
        match MemoryStore::read(self, key) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            read => read.map(Some),
        }
    }
}

/// The objects of an output: the files of its local store and the objects its destination
/// holds. Bytes are written to the local store, and read from it or else from the destination.
#[derive(Clone, Copy)]
pub struct Objects<'a> {
    /// The local store.
    pub local: &'a LocalStore,
    /// What the destination holds.
    pub held: &'a dyn Holdings,
    /// Where the bytes of what the destination holds are read from.
    pub remote: &'a dyn Remote,
}

impl<'a> Objects<'a> {
    /// The objects of a local store that publishes to no destination.
    pub fn local(local: &'a LocalStore) -> Self {
        Self {
            local,
            held: &NoHoldings,
            remote: &NoRemote,
        }
    }

    /// Reads `key` from the destination into the local store, when the destination is recorded
    /// to hold it. Bytes that do not match the key are refused.
    fn read_back(&self, key: &Key) -> io::Result<Option<Vec<u8>>> {
        if self.held.size(key)?.is_none() {
            return Ok(None);
        }
        let Some(bytes) = self.remote.fetch(key)? else {
            return Ok(None);
        };
        if Key::of(&bytes, &key.extension) != *key {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the destination holds other bytes under {key}"),
            ));
        }
        self.local.put(&bytes, &key.extension)?;
        Ok(Some(bytes))
    }

    /// The objects whose hashes start with the byte `prefix`, with their sizes: the files of the
    /// local store, and the objects only the destination holds.
    pub fn sizes(&self, prefix: u8) -> io::Result<HashMap<Key, u64>> {
        let mut sizes = self.held.sizes(prefix)?;
        sizes.extend(self.local.sizes(prefix)?);
        Ok(sizes)
    }
}

impl Store for Objects<'_> {
    fn put(&self, bytes: &[u8], extension: &str) -> io::Result<Key> {
        self.local.put(bytes, extension)
    }

    fn read(&self, key: &Key) -> io::Result<Vec<u8>> {
        match self.local.read(key) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => self.read_back(key)?.ok_or(e),
            read => read,
        }
    }

    fn size(&self, key: &Key) -> io::Result<Option<u64>> {
        match self.local.size(key)? {
            Some(size) => Ok(Some(size)),
            None => self.held.size(key),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_evicted_file_stays_in_the_output_while_the_destination_holds_it() {
        let root = std::env::temp_dir().join(format!("sekai-objects-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let local = LocalStore::open(&root).unwrap();
        let kept = local.put(b"kept", "json").unwrap();
        let evicted = local.put(b"evicted", "webp").unwrap();
        let held = MemoryHoldings::default();
        held.hold(&[(evicted.clone(), 7)]).unwrap();
        std::fs::remove_file(local.path(&evicted)).unwrap();
        let remote = MemoryStore::default();
        remote.put(b"evicted", "webp").unwrap();
        let objects = Objects {
            local: &local,
            held: &held,
            remote: &remote,
        };

        assert_eq!(objects.size(&kept).unwrap(), Some(4));
        assert_eq!(objects.size(&evicted).unwrap(), Some(7));
        assert_eq!(Objects::local(&local).size(&evicted).unwrap(), None);
        let sizes = objects.sizes(evicted.sha256[0]).unwrap();
        assert_eq!(sizes.get(&evicted), Some(&7));
        assert_eq!(objects.read(&evicted).unwrap(), b"evicted");
        assert!(local.contains(&evicted));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
