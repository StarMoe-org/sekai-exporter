//! Bundles as they are stored, read as their UnityFS bytes without copying them whole.

use std::collections::BTreeMap;
use std::io;
use std::sync::{Arc, Mutex, PoisonError};

use sekai_game::obfuscation::{HEAD_LEN, Obfuscation};
use sekai_pipeline::BundleInput;
use sekai_pipeline::stream::{Entry, Inputs};
use sekai_unity::{ByteSource, Chunk, source};

/// A bundle as it is stored, read as its UnityFS bytes: the obfuscated head is restored as it is
/// read.
pub struct Restored {
    stored: Arc<dyn ByteSource>,
    obfuscation: Obfuscation,
}

impl Restored {
    /// The bundle whose stored bytes are `stored`.
    pub fn new(stored: Arc<dyn ByteSource>) -> io::Result<Self> {
        let head = stored
            .read(0, stored.len().min(HEAD_LEN as u64) as usize)
            .map_err(io::Error::other)?;
        let obfuscation = Obfuscation::of(&head).map_err(io::Error::other)?;
        Ok(Self {
            stored,
            obfuscation,
        })
    }

    fn marker_len(&self) -> u64 {
        self.obfuscation.marker_len() as u64
    }
}

impl ByteSource for Restored {
    fn len(&self) -> u64 {
        self.stored.len() - self.marker_len()
    }

    fn read(&self, offset: u64, len: usize) -> sekai_unity::Result<Chunk<'_>> {
        let chunk = self.stored.read(offset + self.marker_len(), len)?;
        if offset >= self.obfuscation.changed_len() {
            return Ok(chunk);
        }
        let mut bytes = chunk.to_vec();
        self.obfuscation.restore(&mut bytes, offset);
        Ok(Chunk::Owned(bytes))
    }
}

/// Stored bundles by name, which a run opens as its inputs. The SHA-256 of a bundle is computed
/// the first time it is opened.
pub struct Stored {
    bundles: BTreeMap<String, Arc<dyn ByteSource>>,
    sha256: Mutex<BTreeMap<String, [u8; 32]>>,
}

impl Stored {
    /// The bundles `bundles`, given as (name, stored bytes).
    pub fn new(bundles: impl IntoIterator<Item = (String, Arc<dyn ByteSource>)>) -> Self {
        Self {
            bundles: bundles.into_iter().collect(),
            sha256: Mutex::default(),
        }
    }

    /// A manifest of the bundles, in name order, without dependencies.
    pub fn manifest(&self) -> Vec<Entry> {
        let entry = |(name, stored): (&String, &Arc<dyn ByteSource>)| Entry {
            name: name.clone(),
            size: stored.len(),
            content: None,
            dependencies: Vec::new(),
        };
        self.bundles.iter().map(entry).collect()
    }

    fn sha256(&self, name: &str, bundle: &Restored) -> io::Result<[u8; 32]> {
        let mut known = self.sha256.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sha256) = known.get(name) {
            return Ok(*sha256);
        }
        let sha256 = source::sha256(bundle, 0, bundle.len()).map_err(io::Error::other)?;
        known.insert(name.to_owned(), sha256);
        Ok(sha256)
    }
}

impl Inputs for Stored {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        let stored = self
            .bundles
            .get(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name.to_owned()))?;
        let bundle = Restored::new(stored.clone())?;
        Ok(BundleInput {
            name: name.to_owned(),
            sha256: self.sha256(name, &bundle)?,
            source: Arc::new(bundle),
        })
    }
}
