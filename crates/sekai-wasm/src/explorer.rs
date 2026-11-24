//! A session over a set of stored bundles, for exploring them one question at a time: the object
//! document of a bundle, the assets a bundle plans, an asset converted on request, and the objects
//! an asset's conversion may read.
//!
//! The bundles are read through their byte sources when a question needs them; nothing is copied
//! whole. Decompressed blocks, bundle indexes and converted assets are kept for later questions.

use std::io;
use std::sync::Arc;

use sekai_catalog::{Key, MemoryBundleIndexes, MemoryDerivations, MemoryStore};
use sekai_convert::Converter;
use sekai_format::SourceObject;
use sekai_pipeline::objects::{self, Indexed, Source};
use sekai_pipeline::stream::{self, Entry};
use sekai_pipeline::{Lazy, PlannedAsset};
use sekai_unity::document::Payloads;
use sekai_unity::{BoundedBlockCache, ByteSource, NoClasses};

use crate::stored::Stored;
use crate::{BLOCK_CACHE_BYTES, converters, evaluation};

/// An open set of bundles.
pub struct Explorer {
    bundles: Stored,
    manifest: Vec<Entry>,
    indexed: Indexed,
    converters: Vec<Box<dyn Converter>>,
    cache: BoundedBlockCache,
    indexes: MemoryBundleIndexes,
    store: MemoryStore,
    derivations: MemoryDerivations,
}

impl Explorer {
    /// Opens `bundles`, given as (name, stored bytes): finds what each depends on and indexes
    /// every object, which resolves the references between bundles.
    pub fn open(bundles: Vec<(String, Arc<dyn ByteSource>)>) -> io::Result<Self> {
        let bundles = Stored::new(bundles);
        let mut manifest = bundles.manifest();
        let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
        let indexes = MemoryBundleIndexes::default();
        stream::with_dependencies(&mut manifest, &bundles, &cache, &indexes, 1)?;
        let names: Vec<String> = manifest.iter().map(|entry| entry.name.clone()).collect();
        let source = Source {
            inputs: &bundles,
            classes: &NoClasses,
            cache: &cache,
        };
        let indexed = objects::index(&source, &names, 1);
        Ok(Self {
            bundles,
            manifest,
            indexed,
            converters: converters(),
            cache,
            indexes,
            store: MemoryStore::default(),
            derivations: MemoryDerivations::default(),
        })
    }

    /// The bundles that were indexed, in name order.
    pub fn bundles(&self) -> impl Iterator<Item = &str> {
        let bundles = self.indexed.version.bundles().iter();
        bundles.map(|bundle| bundle.name.as_str())
    }

    /// The bundles that could not be opened or indexed, as (name, reason), in name order.
    pub fn failed(&self) -> &[(String, String)] {
        &self.indexed.failed
    }

    /// The object document of `bundle`, with the payloads too large to inline handed to
    /// `payloads`.
    pub fn document(&self, bundle: &str, payloads: &mut dyn Payloads) -> Result<Vec<u8>, String> {
        let version = &self.indexed.version;
        let position = version
            .bundles()
            .binary_search_by(|indexed| indexed.name.as_str().cmp(bundle))
            .map_err(|_| format!("{bundle}: not indexed"))?;
        let source = Source {
            inputs: &self.bundles,
            classes: &NoClasses,
            cache: &self.cache,
        };
        objects::document(&source, version, position, payloads)
    }

    /// The assets `bundle` plans, in name order.
    pub fn assets(&self, bundle: &str) -> io::Result<Vec<PlannedAsset>> {
        self.lazy().assets(bundle)
    }

    /// The asset document of the asset `name`, converted if it has not been. `None` if no bundle
    /// has such an asset or its root holds none.
    pub fn convert(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        let output = self.lazy().asset(name)?;
        output
            .map(|output| self.store.read(&output.document))
            .transpose()
    }

    /// The objects the conversion of the asset `name` may read, its root first. `None` if no
    /// bundle has such an asset.
    pub fn closure(&self, name: &str) -> io::Result<Option<Vec<SourceObject>>> {
        self.lazy().closure(name)
    }

    /// A file of a converted asset: a member, or the asset document.
    pub fn file(&self, key: &Key) -> io::Result<Vec<u8>> {
        self.store.read(key)
    }

    /// An evaluator over the session: it keeps no windows of its own between questions, while
    /// the blocks, indexes and conversions it reaches stay in the session.
    fn lazy(&self) -> Lazy<'_> {
        Lazy::new(
            &self.manifest,
            &self.bundles,
            evaluation(
                &self.converters,
                &self.store,
                &self.derivations,
                &self.indexes,
                &self.cache,
            ),
        )
    }
}
