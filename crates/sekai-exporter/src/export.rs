//! `sekai-exporter export`: canonical assets for a directory of bundles or a client package.

use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sekai_catalog::{Derivations, LocalStore};
use sekai_convert::Converter;
use sekai_format::publication::Pointer;
use sekai_game::obfuscation::deobfuscate;
use sekai_pipeline::ledger::BundleLedger;
use sekai_pipeline::metrics::Recorder;
use sekai_pipeline::stream::{self, Entry, InBundle, Inputs, Segment, Stream, Streamed};
use sekai_pipeline::{BundleInput, Evaluation, NoBundleIndexes};
use sekai_publish::{Binding, Plan, index, plan, tree};
use sekai_unity::{BoundedBlockCache, ClassLayouts, NoClasses};
use sha2::{Digest, Sha256};

use crate::monitor::Monitor;
use crate::{budget, client};

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

#[cfg(feature = "live2d")]
fn live2d() -> Vec<Box<dyn Converter>> {
    vec![
        Box::new(sekai_convert::live2d::Model),
        Box::new(sekai_convert::live2d::Motions),
    ]
}

#[cfg(not(feature = "live2d"))]
fn live2d() -> Vec<Box<dyn Converter>> {
    use sekai_convert::roots::{Absent, live2d_model, live2d_motions};
    vec![
        Absent::boxed("live2d-model", live2d_model),
        Absent::boxed("live2d-motions", live2d_motions),
    ]
}

#[cfg(feature = "mysekai")]
fn mysekai_objects() -> Vec<Box<dyn Converter>> {
    vec![
        Box::new(sekai_mysekai::emoticon::Emoticon),
        Box::new(sekai_mysekai::facial::FacialTable),
    ]
}

#[cfg(not(feature = "mysekai"))]
fn mysekai_objects() -> Vec<Box<dyn Converter>> {
    use sekai_convert::roots::{Absent, emoticon, facial_table};
    vec![
        Absent::boxed("emoticon", emoticon),
        Absent::boxed("facial-table", facial_table),
    ]
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

#[cfg(feature = "virtual-live")]
fn virtual_live() -> Vec<Box<dyn Converter>> {
    vec![Box::new(sekai_moesekai::virtual_live::VirtualLiveScript)]
}

#[cfg(not(feature = "virtual-live"))]
fn virtual_live() -> Vec<Box<dyn Converter>> {
    use sekai_convert::roots::Absent;
    vec![Absent::boxed(
        "virtual-live-script",
        sekai_moesekai::roots::virtual_live_script,
    )]
}

#[cfg(feature = "episode")]
fn story_data() -> Vec<Box<dyn Converter>> {
    use sekai_moesekai::story_data::{ActionSetForm, ScenarioBackgroundForm};
    vec![Box::new(ActionSetForm), Box::new(ScenarioBackgroundForm)]
}

#[cfg(not(feature = "episode"))]
fn story_data() -> Vec<Box<dyn Converter>> {
    use sekai_convert::roots::Absent;
    use sekai_moesekai::roots::{action_set, scenario_background};
    vec![
        Absent::boxed("action-set", action_set),
        Absent::boxed("scenario-background", scenario_background),
    ]
}

#[cfg(feature = "costume")]
fn costume() -> Vec<Box<dyn Converter>> {
    vec![
        Box::new(sekai_moesekai::costume::CostumeModel),
        Box::new(sekai_moesekai::costume::CostumeMotion),
    ]
}

#[cfg(not(feature = "costume"))]
fn costume() -> Vec<Box<dyn Converter>> {
    use sekai_convert::roots::Absent;
    use sekai_moesekai::roots::{costume_model, costume_motion};
    vec![
        Absent::boxed("costume-model", costume_model),
        Absent::boxed("costume-motion", costume_motion),
    ]
}

/// Every converter of this build. `hca_key` decrypts encrypted HCA audio.
pub(crate) fn converters(hca_key: Option<u64>) -> Vec<Box<dyn Converter>> {
    [
        image(),
        audio(hca_key),
        video(),
        text(),
        live2d(),
        mysekai_objects(),
        scenario(),
        virtual_live(),
        story_data(),
        costume(),
    ]
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
    tables: LayoutTables,
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
    let mut tables = LayoutTables::default();
    let mut memory = None;
    let mut plain = false;
    while let Some(arg) = parser.next()? {
        match arg {
            Long("threads") => threads = parser.value()?.parse()?,
            Long("memory") => memory = Some(budget::parse_bytes(&parser.value()?.string()?)?),
            Long("plain") => plain = true,
            Long("tree") => tree = Some(PathBuf::from(parser.value()?)),
            Long("class-table") => tables.classes = Some(PathBuf::from(parser.value()?)),
            Long("script-table") => tables.scripts = Some(PathBuf::from(parser.value()?)),
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
        tables,
        threads: threads.max(1),
        memory,
        plain,
    })
}

/// The layout tables named on the command line.
#[derive(Debug, Default)]
pub(crate) struct LayoutTables {
    /// A TPK file of engine class layouts.
    pub(crate) classes: Option<PathBuf>,
    /// A table of script layouts.
    pub(crate) scripts: Option<PathBuf>,
}

/// What an export reads.
pub(crate) struct Source<'a> {
    /// The bundles to evaluate.
    pub(crate) manifest: Vec<Entry>,
    /// The bytes of the bundles.
    pub(crate) inputs: Box<dyn Inputs + 'a>,
    /// The layouts of engine classes.
    pub(crate) classes: Box<dyn ClassLayouts>,
}

/// The source of an export: a client package when a class table comes with it, and otherwise a
/// directory of bundles.
pub(crate) fn source<'a>(
    input: &'a Path,
    tables: &LayoutTables,
) -> Result<Source<'a>, lexopt::Error> {
    match (&tables.classes, &tables.scripts) {
        (Some(classes), scripts) => client::source(input, classes, scripts.as_deref()),
        (None, Some(_)) => Err("--script-table needs --class-table".into()),
        (None, None) => directory(input),
    }
}

fn directory(root: &Path) -> Result<Source<'_>, lexopt::Error> {
    let mut manifest = Vec::new();
    walk(root, "", &mut manifest).map_err(|e| format!("{}: {e}", root.display()))?;
    Ok(Source {
        manifest,
        inputs: Box::new(Directory { root }),
        classes: Box::new(NoClasses),
    })
}

/// The bundles of a directory, by their paths under it with `/` separators.
struct Directory<'a> {
    root: &'a Path,
}

impl Inputs for Directory<'_> {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        let mut bytes = std::fs::read(self.root.join(name))?;
        deobfuscate(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
        Ok(BundleInput {
            name: name.to_owned(),
            sha256: Sha256::digest(&bytes).into(),
            source: Arc::new(Arc::<[u8]>::from(bytes)),
        })
    }
}

/// Every file under `directory`, as an entry named by its path under the root with `/`
/// separators, appended to `out`.
fn walk(directory: &Path, prefix: &str, out: &mut Vec<Entry>) -> io::Result<()> {
    for item in std::fs::read_dir(directory)? {
        let item = item?;
        let name = format!("{prefix}{}", item.file_name().to_string_lossy());
        let path = item.path();
        // Links are followed.
        let metadata = std::fs::metadata(&path)?;
        if metadata.is_dir() {
            walk(&path, &format!("{name}/"), out)?;
        } else if metadata.is_file() {
            out.push(Entry {
                name,
                size: metadata.len(),
                dependencies: Vec::new(),
            });
        }
    }
    Ok(())
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
    let Source {
        mut manifest,
        inputs,
        classes,
    } = source(&options.input, &options.tables)?;
    let inputs = inputs.as_ref();
    let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
    {
        let _span = monitor.span("manifest", "dependencies", "");
        stream::with_dependencies(
            &mut manifest,
            inputs,
            &cache,
            &NoBundleIndexes,
            options.threads,
        )
        .map_err(|e| e.to_string())?;
    }

    let store = LocalStore::open(options.output.join("store"))
        .map_err(|e| format!("{}: {e}", options.output.display()))?;
    let derivations = derivations(&options.output, &store).map_err(|e| e.to_string())?;
    let converters = converters(None);
    let streamed = stream::evaluate(
        &manifest,
        inputs,
        &Stream {
            converters: &converters,
            naming: &InBundle,
            store: &store,
            derivations: derivations.as_ref(),
            indexes: &NoBundleIndexes,
            cache: &cache,
            classes: classes.as_ref(),
            recorder: monitor,
            fusion: None,
            options: budget::options(options.threads, options.memory, options.plain),
            segment: Segment::default(),
            ledger: Some(IGNORED),
            limit: None,
        },
    )
    .map_err(|e| e.to_string())?;
    let failed = report(&streamed);
    let evaluation = &streamed.evaluation;

    publish(
        evaluation,
        "local",
        "local/canonical",
        &options.output,
        options.tree.as_deref(),
        &store,
        monitor,
    )?;
    write_ledger(&streamed.ledgers, &options.output, monitor)?;
    let errored = evaluation.outputs.iter().filter(|o| o.error.is_some());
    println!(
        "bundles {} failed {failed} assets {} errors {} unnamed {} converted {} reused {}",
        manifest.len(),
        evaluation.outputs.len(),
        errored.count(),
        evaluation.unnamed.len(),
        evaluation.converted,
        evaluation.reused
    );
    Ok(failed == 0)
}

/// Prints the bundles that could not be evaluated and the references that leave the window of
/// their unit, and returns the number of such bundles.
pub(crate) fn report(streamed: &Streamed) -> usize {
    for e in &streamed.errors {
        eprintln!("{}: {}: {}", e.name, e.code, e.message);
    }
    for o in &streamed.outside {
        eprintln!(
            "{}: outside_dependencies: object {} of {} refers to object {} of {}",
            o.unit, o.object, o.bundle, o.path_id, o.file
        );
    }
    if !streamed.outside.is_empty() {
        println!("references outside dependencies {}", streamed.outside.len());
    }
    streamed.errors.len()
}

/// The classes and script classes that carry no asset content. `AssetBundle` is the table of
/// contents of a bundle, read when the bundle is indexed. `MonoScript` names the script of other
/// objects. `SoundBundleBuildData` and `MovieBundleBuildData` list the files of a bundle.
pub(crate) const IGNORED: &[&str] = &[
    "AssetBundle",
    "MonoScript",
    "SoundBundleBuildData",
    "MovieBundleBuildData",
];

/// Writes what became of the contents of each bundle to `<output>/ledger.tsv`.
pub(crate) fn write_ledger(
    ledgers: &[BundleLedger],
    output: &Path,
    monitor: &Monitor,
) -> Result<(), lexopt::Error> {
    let _span = monitor.span("publish", "ledger", "");
    let mut text = String::from(
        "bundle\tobjects\troots\tread\tignored\tunknown\tresources\tresources_read\tunknown_classes\n",
    );
    for l in ledgers {
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
