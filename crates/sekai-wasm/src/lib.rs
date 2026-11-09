//! The exporter core for WebAssembly: unpacking, conversion and streaming evaluation without a
//! server.
//!
//! [`run`] evaluates bundles given as bytes, in memory, one bundle after another, and hands the
//! files of each bundle's assets over once they are written. On `wasm32` it is exported to
//! JavaScript as `evaluate`.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::sync::Arc;

use sekai_catalog::{Key, MemoryDerivations, MemoryStore, NoBundleIndexes};
use sekai_convert::Converter;
use sekai_game::obfuscation::deobfuscate;
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::stream::{self, Entry, InBundle, Inputs, Segment, Stream};
use sekai_pipeline::{BundleInput, Options, Output};
use sekai_unity::BoundedBlockCache;
use sha2::{Digest, Sha256};

/// The most memory one asset may be estimated to need: half of the 32-bit address space.
const MEMORY_LIMIT: u64 = 1 << 31;
const BLOCK_CACHE_BYTES: usize = 64 << 20;

/// The converters of the core, in the order the command line uses them.
pub fn converters() -> Vec<Box<dyn Converter>> {
    vec![
        Box::new(sekai_convert::image::Texture),
        Box::new(sekai_convert::image::Sprite),
        Box::new(sekai_convert::audio::Audio::new(None)),
        Box::new(sekai_convert::video::Video),
        Box::new(sekai_convert::text::Text),
        Box::new(sekai_convert::text::Font),
        Box::new(sekai_moesekai::scenario::Scenario),
    ]
}

/// What an evaluation produced.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Report {
    /// The assets, including those that could not be produced.
    pub assets: usize,
    /// The number of assets with each error code.
    pub errors: BTreeMap<String, usize>,
    /// The number of roots that cannot be told apart from another root by name.
    pub unnamed: usize,
    /// The bundles that could not be opened, as (name, error code).
    pub bundle_errors: Vec<(String, String)>,
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "assets\t{}", self.assets)?;
        writeln!(f, "unnamed\t{}", self.unnamed)?;
        for (code, count) in &self.errors {
            writeln!(f, "error\t{code}\t{count}")?;
        }
        for (name, code) in &self.bundle_errors {
            writeln!(f, "bundle\t{name}\t{code}")?;
        }
        Ok(())
    }
}

/// The bundles given to [`run`], deobfuscated, with the SHA-256 of their bytes.
struct Memory(BTreeMap<String, (Arc<[u8]>, [u8; 32])>);

impl Inputs for Memory {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        let (bytes, sha256) = self
            .0
            .get(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, name.to_owned()))?;
        Ok(BundleInput {
            name: name.to_owned(),
            source: Arc::new(bytes.clone()),
            sha256: *sha256,
        })
    }
}

/// Evaluates every asset of `bundles`, given as (name, bytes), with the dependencies their files
/// name. `on_file` receives the files of the assets of each bundle, which are then released, and
/// `on_asset` each asset at the end, in name order.
pub fn run(
    bundles: Vec<(String, Vec<u8>)>,
    on_file: &mut dyn FnMut(&Key, &[u8]) -> io::Result<()>,
    on_asset: &mut dyn FnMut(&Output) -> io::Result<()>,
) -> io::Result<Report> {
    let mut manifest = Vec::with_capacity(bundles.len());
    let mut memory = BTreeMap::new();
    let mut report = Report::default();
    for (name, mut bytes) in bundles {
        match deobfuscate(&mut bytes) {
            Ok(()) => {
                manifest.push(Entry {
                    name: name.clone(),
                    size: bytes.len() as u64,
                    dependencies: Vec::new(),
                });
                let sha256 = Sha256::digest(&bytes).into();
                memory.insert(name, (Arc::<[u8]>::from(bytes), sha256));
            }
            Err(_) => report.bundle_errors.push((name, "obfuscation".to_owned())),
        }
    }
    let inputs = Memory(memory);
    let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
    stream::with_dependencies(&mut manifest, &inputs, &cache, &NoBundleIndexes, 1)?;
    let converters = converters();
    let store = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let streamed = stream::evaluate_in_segments(
        &manifest,
        &inputs,
        &Stream {
            converters: &converters,
            naming: &InBundle,
            store: &store,
            derivations: &derivations,
            indexes: &NoBundleIndexes,
            cache: &cache,
            recorder: &NoRecorder,
            fusion: None,
            options: Options::threads(1),
            segment: Segment { units: 1, bytes: 0 },
            ledger: None,
            limit: Some(MEMORY_LIMIT),
        },
        &mut || {
            for (key, bytes) in store.drain() {
                on_file(&key, &bytes)?;
            }
            Ok(())
        },
    )?;
    let errors = streamed.errors.into_iter();
    report
        .bundle_errors
        .extend(errors.map(|e| (e.name, e.code.to_owned())));
    report.unnamed = streamed.evaluation.unnamed.len();
    for output in &streamed.evaluation.outputs {
        on_asset(output)?;
        report.assets += 1;
        if let Some(code) = &output.error {
            *report.errors.entry(code.clone()).or_default() += 1;
        }
    }
    Ok(report)
}

/// One row of the list of assets: name, form, document and error code, separated by tabs.
pub fn row(output: &Output) -> String {
    format!(
        "{}\t{}\t{}\t{}\n",
        output.name,
        output.form,
        output.document,
        output.error.as_deref().unwrap_or("-")
    )
}

/// The interface for JavaScript.
#[cfg(target_arch = "wasm32")]
pub mod javascript {
    use std::io;

    use js_sys::{Function, Uint8Array};
    use wasm_bindgen::prelude::*;

    fn failed(error: JsValue) -> io::Error {
        io::Error::other(format!("{error:?}"))
    }

    /// Evaluates bundles given as parallel lists of names and bytes. `on_file(path, bytes)`
    /// receives each file of the store and `on_asset(row)` each row of the list of assets.
    /// Returns the report.
    #[wasm_bindgen]
    pub fn evaluate(
        names: Vec<String>,
        bundles: Vec<Uint8Array>,
        on_file: &Function,
        on_asset: &Function,
    ) -> Result<String, JsError> {
        let bundles = names
            .into_iter()
            .zip(bundles.iter().map(Uint8Array::to_vec))
            .collect();
        let report = super::run(
            bundles,
            &mut |key, bytes| {
                let path = JsValue::from(key.relative_path());
                let bytes = JsValue::from(Uint8Array::from(bytes));
                on_file
                    .call2(&JsValue::NULL, &path, &bytes)
                    .map(drop)
                    .map_err(failed)
            },
            &mut |output| {
                on_asset
                    .call1(&JsValue::NULL, &JsValue::from(super::row(output)))
                    .map(drop)
                    .map_err(failed)
            },
        )?;
        Ok(report.to_string())
    }
}
