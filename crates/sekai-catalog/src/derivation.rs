//! The outputs of derivations, by fingerprint.

use std::collections::BTreeMap;
use std::io;
use std::sync::Mutex;

use crate::store::Key;

/// What a fingerprint produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Derived {
    /// The asset document.
    Document(Key),
    /// The root holds no asset of its form.
    Nothing,
}

/// The outputs of earlier evaluations, by fingerprint.
pub trait Derivations: Sync {
    /// The output of `fingerprint`, if it has been evaluated.
    fn get(&self, fingerprint: &[u8; 32]) -> io::Result<Option<Derived>>;
    /// Records the output of `fingerprint`.
    fn put(&self, fingerprint: [u8; 32], derived: Derived) -> io::Result<()>;
    /// Makes every recorded output durable.
    fn flush(&self) -> io::Result<()> {
        Ok(())
    }
}

/// The stored indexes of bundles, by the SHA-256 of the bundle bytes.
pub trait BundleIndexes: Sync {
    /// The stored index of a bundle, if there is one.
    fn get(&self, bundle: &[u8; 32]) -> io::Result<Option<Vec<u8>>>;
    /// Stores the index of a bundle.
    fn put(&self, bundle: [u8; 32], index: Vec<u8>) -> io::Result<()>;
    /// Makes every stored index durable.
    fn flush(&self) -> io::Result<()> {
        Ok(())
    }
}

/// [`BundleIndexes`] that keep nothing, so every bundle is indexed.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoBundleIndexes;

impl BundleIndexes for NoBundleIndexes {
    fn get(&self, _bundle: &[u8; 32]) -> io::Result<Option<Vec<u8>>> {
        Ok(None)
    }

    fn put(&self, _bundle: [u8; 32], _index: Vec<u8>) -> io::Result<()> {
        Ok(())
    }
}

/// [`Derivations`] held in memory.
#[derive(Debug, Default)]
pub struct MemoryDerivations(Mutex<BTreeMap<[u8; 32], Derived>>);

impl Derivations for MemoryDerivations {
    fn get(&self, fingerprint: &[u8; 32]) -> io::Result<Option<Derived>> {
        Ok(self.0.lock().unwrap().get(fingerprint).cloned())
    }

    fn put(&self, fingerprint: [u8; 32], derived: Derived) -> io::Result<()> {
        self.0.lock().unwrap().insert(fingerprint, derived);
        Ok(())
    }
}
