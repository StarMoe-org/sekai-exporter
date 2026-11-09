//! The exporter core for WebAssembly: unpacking, conversion and lazy evaluation without a server.
//!
//! [`run`] evaluates bundles given as bytes, in memory, and hands each file over as soon as it is
//! written. On `wasm32` it is exported to JavaScript as `evaluate`.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::sync::Arc;

use sekai_catalog::{Key, MemoryDerivations, MemoryStore};
use sekai_convert::Converter;
use sekai_game::obfuscation::deobfuscate;
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::{BundleInput, Lazy, Output, Version};
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

/// Evaluates every asset of `bundles`, given as (name, bytes). `on_file` receives the files of an
/// asset, which are then released, and `on_asset` the asset after them.
pub fn run(
    bundles: Vec<(String, Vec<u8>)>,
    on_file: &mut dyn FnMut(&Key, &[u8]) -> io::Result<()>,
    on_asset: &mut dyn FnMut(&Output) -> io::Result<()>,
) -> io::Result<Report> {
    let mut inputs = Vec::with_capacity(bundles.len());
    let mut report = Report::default();
    for (name, mut bytes) in bundles {
        match deobfuscate(&mut bytes) {
            Ok(()) => inputs.push(BundleInput {
                name,
                sha256: Sha256::digest(&bytes).into(),
                source: Arc::new(Arc::<[u8]>::from(bytes)),
            }),
            Err(_) => report.bundle_errors.push((name, "obfuscation".to_owned())),
        }
    }
    let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
    let (version, errors) = Version::open(inputs, &cache);
    let errors = errors.into_iter().map(|e| (e.name, e.code.to_owned()));
    report.bundle_errors.extend(errors);
    let converters = converters();
    let store = MemoryStore::default();
    let derivations = MemoryDerivations::default();
    let lazy = Lazy::new(&version, &converters, &store, &derivations, &NoRecorder, 1)
        .with_memory_limit(MEMORY_LIMIT);
    report.unnamed = lazy.unnamed().len();
    for name in lazy.names() {
        let Some(output) = lazy.asset(name)? else {
            continue;
        };
        for (key, bytes) in store.drain() {
            on_file(&key, &bytes)?;
        }
        on_asset(&output)?;
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
