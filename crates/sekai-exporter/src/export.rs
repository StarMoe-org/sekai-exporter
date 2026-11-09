//! `sekai-exporter export`: canonical assets for a directory of bundles.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sekai_catalog::{Derivations, LocalStore};
use sekai_convert::Converter;
use sekai_format::AssetDocument;
use sekai_format::publication::Pointer;
use sekai_game::obfuscation::deobfuscate;
use sekai_pipeline::ledger::account;
use sekai_pipeline::metrics::Recorder;
use sekai_pipeline::{BundleInput, Evaluation, NoBundleIndexes, Version, evaluate_with, parallel};
use sekai_publish::{Binding, Plan, index, plan, tree};
use sekai_unity::BoundedBlockCache;
use sha2::{Digest, Sha256};

use crate::budget;
use crate::monitor::Monitor;

pub(crate) const BLOCK_CACHE_BYTES: usize = 256 << 20;

#[cfg(feature = "image")]
pub(crate) fn image() -> Vec<Box<dyn Converter>> {
    vec![
        Box::new(sekai_convert::image::Texture),
        Box::new(sekai_convert::image::Sprite),
    ]
}

#[cfg(not(feature = "image"))]
pub(crate) fn image() -> Vec<Box<dyn Converter>> {
    use sekai_convert::roots::{Absent, sprite, texture};
    vec![
        Absent::boxed("texture", texture),
        Absent::boxed("sprite", sprite),
    ]
}

#[cfg(feature = "video")]
pub(crate) fn video() -> Vec<Box<dyn Converter>> {
    vec![Box::new(sekai_convert::video::Video)]
}

#[cfg(not(feature = "video"))]
pub(crate) fn video() -> Vec<Box<dyn Converter>> {
    use sekai_convert::roots::{Absent, video};
    vec![Absent::boxed("video", video)]
}

#[cfg(feature = "audio")]
fn audio(hca_key: Option<u64>) -> Vec<Box<dyn Converter>> {
    vec![Box::new(sekai_convert::audio::Audio::new(hca_key))]
}

#[cfg(not(feature = "audio"))]
fn audio(_hca_key: Option<u64>) -> Vec<Box<dyn Converter>> {
    use sekai_convert::roots::{Absent, audio};
    vec![Absent::boxed("audio", audio)]
}

#[cfg(feature = "text")]
fn text() -> Vec<Box<dyn Converter>> {
    vec![
        Box::new(sekai_convert::text::Text),
        Box::new(sekai_convert::text::Font),
    ]
}

#[cfg(not(feature = "text"))]
fn text() -> Vec<Box<dyn Converter>> {
    use sekai_convert::roots::{Absent, font, text};
    vec![Absent::boxed("text", text), Absent::boxed("font", font)]
}

#[cfg(feature = "scenario")]
fn scenario() -> Vec<Box<dyn Converter>> {
    vec![Box::new(sekai_moesekai::scenario::Scenario)]
}

#[cfg(not(feature = "scenario"))]
fn scenario() -> Vec<Box<dyn Converter>> {
    use sekai_convert::roots::Absent;
    vec![Absent::boxed("scenario", sekai_moesekai::roots::scenario)]
}

/// Every converter of this build. `hca_key` decrypts encrypted HCA audio.
pub(crate) fn converters(hca_key: Option<u64>) -> Vec<Box<dyn Converter>> {
    [image(), audio(hca_key), video(), text(), scenario()]
        .into_iter()
        .flatten()
        .collect()
}

#[cfg(feature = "catalog-db")]
fn derivations(output: &Path, store: &LocalStore) -> std::io::Result<Box<dyn Derivations>> {
    let catalog = sekai_catalog::db::Catalog::open_or_rebuild(&output.join("catalog.redb"), store)?;
    Ok(Box::new(catalog.into_derivations()))
}

#[cfg(not(feature = "catalog-db"))]
fn derivations(_output: &Path, _store: &LocalStore) -> std::io::Result<Box<dyn Derivations>> {
    Ok(Box::new(sekai_pipeline::MemoryDerivations::default()))
}

struct Options {
    input: PathBuf,
    output: PathBuf,
    tree: Option<PathBuf>,
    threads: usize,
    memory: Option<u64>,
    plain: bool,
}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Options, lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let mut positional = Vec::new();
    let mut threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut tree = None;
    let mut memory = None;
    let mut plain = false;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("threads") => threads = parser.value()?.parse()?,
            Long("memory") => memory = Some(budget::parse_bytes(&parser.value()?.string()?)?),
            Long("plain") => plain = true,
            Long("tree") => tree = Some(PathBuf::from(parser.value()?)),
            Value(v) => positional.push(PathBuf::from(v)),
            _ => return Err(arg.unexpected()),
        }
    }
    let [input, output]: [PathBuf; 2] = positional
        .try_into()
        .map_err(|_| lexopt::Error::from("expected <input> and <output>"))?;
    Ok(Options {
        input,
        output,
        tree,
        threads: threads.max(1),
        memory,
        plain,
    })
}

fn read(path: &Path) -> Result<BundleInput, String> {
    let mut bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    deobfuscate(&mut bytes).map_err(|e| e.to_string())?;
    Ok(BundleInput {
        name: path.file_name().unwrap().to_string_lossy().into_owned(),
        sha256: Sha256::digest(&bytes).into(),
        source: Arc::new(Arc::<[u8]>::from(bytes)),
    })
}

pub(crate) fn run(args: impl Iterator<Item = OsString>) -> Result<bool, lexopt::Error> {
    let options = parse(args)?;
    std::fs::create_dir_all(&options.output)
        .map_err(|e| format!("{}: {e}", options.output.display()))?;
    Monitor::watch(&options.output.join("metrics.jsonl"), |monitor| {
        export(&options, monitor)
    })
    .map_err(|e| lexopt::Error::from(e.to_string()))?
}

fn export(options: &Options, monitor: &Monitor) -> Result<bool, lexopt::Error> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&options.input)
        .map_err(|e| format!("{}: {e}", options.input.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file())
        .collect();
    paths.sort();

    let mut failed = 0usize;
    let mut inputs = Vec::new();
    for (path, result) in paths
        .iter()
        .zip(parallel(paths.len(), options.threads, |i| read(&paths[i])))
    {
        match result {
            Ok(input) => inputs.push(input),
            Err(e) => {
                failed += 1;
                eprintln!("{}: {e}", path.display());
            }
        }
    }
    let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
    let (version, errors) = {
        let _span = monitor.span("open", "version", "");
        Version::open_indexed(inputs, &cache, &NoBundleIndexes, options.threads)
            .map_err(|e| e.to_string())?
    };
    for e in &errors {
        eprintln!("{}: {}: {}", e.name, e.code, e.message);
    }
    failed += errors.len();

    let store = LocalStore::open(options.output.join("store"))
        .map_err(|e| format!("{}: {e}", options.output.display()))?;
    let derivations = derivations(&options.output, &store).map_err(|e| e.to_string())?;
    let converters = converters(None);
    let evaluation = evaluate_with(
        &version,
        &converters,
        &store,
        derivations.as_ref(),
        monitor,
        &budget::options(options.threads, options.memory, options.plain),
    )
    .map_err(|e| e.to_string())?;

    publish(
        &evaluation,
        "local",
        "local/canonical",
        &options.output,
        options.tree.as_deref(),
        &store,
        monitor,
    )?;
    write_ledger(
        &evaluation,
        &version,
        &converters,
        &options.output,
        &store,
        monitor,
    )?;
    let errored = evaluation.outputs.iter().filter(|o| o.error.is_some());
    println!(
        "bundles {} failed {failed} assets {} errors {} unnamed {} converted {} reused {}",
        version.index().bundles().len(),
        evaluation.outputs.len(),
        errored.count(),
        evaluation.unnamed.len(),
        evaluation.converted,
        evaluation.reused
    );
    Ok(failed == 0)
}

/// The classes and script classes that carry no asset content. `AssetBundle` is the table of
/// contents of a bundle, read when the bundle is indexed. `MonoScript` names the script of other
/// objects. `SoundBundleBuildData` and `MovieBundleBuildData` list the files of a bundle.
const IGNORED: &[&str] = &[
    "AssetBundle",
    "MonoScript",
    "SoundBundleBuildData",
    "MovieBundleBuildData",
];

/// Writes what became of the contents of each bundle to `<output>/ledger.tsv`.
pub(crate) fn write_ledger(
    evaluation: &Evaluation,
    version: &Version<'_>,
    converters: &[Box<dyn Converter>],
    output: &Path,
    store: &LocalStore,
    monitor: &Monitor,
) -> Result<(), lexopt::Error> {
    let _span = monitor.span("publish", "ledger", "");
    let documents = evaluation
        .outputs
        .iter()
        .map(|o| {
            let bytes = store.read(&o.document).map_err(|e| e.to_string())?;
            AssetDocument::from_bytes(&bytes).map_err(|e| e.to_string())
        })
        .collect::<Result<Vec<_>, String>>()?;
    let ledgers = account(version, converters, &documents, IGNORED);
    let mut text = String::from(
        "bundle\tobjects\troots\tread\tignored\tunknown\tresources\tresources_read\tunknown_classes\n",
    );
    for l in &ledgers {
        let classes: Vec<String> = l
            .unknown_classes
            .iter()
            .map(|(class, count)| format!("{class}={count}"))
            .collect();
        text.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            l.bundle,
            l.objects,
            l.roots,
            l.read,
            l.ignored,
            l.unknown,
            l.resources,
            l.resources_read,
            classes.join(",")
        ));
    }
    std::fs::write(output.join("ledger.tsv"), text).map_err(|e| e.to_string())?;
    println!(
        "ledger bundles {} complete {} unknown objects {}",
        ledgers.len(),
        ledgers.iter().filter(|l| l.complete()).count(),
        ledgers.iter().map(|l| l.unknown).sum::<u64>()
    );
    Ok(())
}

/// Lists `evaluation` in `<output>/assets.tsv`, publishes the index under `<output>/published` and,
/// with `tree_root`, a path tree. Returns the pointer and the plan of the publication.
pub(crate) fn publish(
    evaluation: &Evaluation,
    version: &str,
    target: &str,
    output: &Path,
    tree_root: Option<&Path>,
    store: &LocalStore,
    monitor: &Monitor,
) -> Result<(Pointer, Plan), lexopt::Error> {
    let mut listing = String::new();
    for output in &evaluation.outputs {
        listing.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            output.name,
            output.form,
            output.document,
            output.error.as_deref().unwrap_or("-")
        ));
    }
    std::fs::write(output.join("assets.tsv"), listing).map_err(|e| e.to_string())?;
    let bindings: Vec<Binding> = evaluation
        .outputs
        .iter()
        .map(|o| Binding {
            name: o.name.clone(),
            document: o.document.clone(),
        })
        .collect();
    let publishing = monitor.span("publish", "index", "");
    let plan = plan(version, &bindings, store).map_err(|e| e.to_string())?;
    let pointer = index::publish(store, &output.join("published"), target, &plan)
        .map_err(|e| e.to_string())?;
    drop(publishing);
    println!("published revision {}", pointer.revision);
    if let Some(root) = tree_root {
        let _span = monitor.span("publish", "tree", "");
        let report = tree::publish(store, root, &plan).map_err(|e| e.to_string())?;
        println!(
            "tree written {} unchanged {} deleted {}",
            report.written, report.unchanged, report.deleted
        );
    }
    Ok((pointer, plan))
}
