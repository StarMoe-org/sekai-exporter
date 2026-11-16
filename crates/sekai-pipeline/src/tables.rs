//! The table stage: assets built from game data tables and the assets of the version.
//!
//! Each asset is built with a context that notes what it reads. An asset of a reusable plan keeps
//! what the previous run built for it when its reads give what they gave then, and is built again
//! otherwise.

mod trace;

use std::collections::{BTreeMap, HashMap};
use std::io;

use sekai_catalog::store::parse_hex;
use sekai_catalog::{Key, Store};
use sekai_convert::tables::{
    AssetEntry, STORE_FAILED, TableAsset, TableContext, TableConverter, TablePlan,
};
use sekai_convert::{ConvertError, Member};
use sekai_format::asset::{FORMAT, VERSION};
use sekai_format::{AssetDocument, AssetError, MemberFile};
use sha2::{Digest, Sha256};

use crate::canonical::encode;
use crate::eval::{Evaluation, Output, asset_error, hex, parallel};
use crate::metrics::Recorder;
use crate::state::TableOutput;
use trace::{Traced, Values};

pub use trace::{Read, Trace};

/// What the table stage works with.
pub struct TableStage<'a> {
    /// The converters of the build.
    pub converters: &'a [Box<dyn TableConverter>],
    /// Reads a table by name.
    pub read: &'a (dyn Fn(&str) -> io::Result<Vec<u8>> + Sync),
    /// The store of the run.
    pub store: &'a dyn Store,
    /// Receives the work of the stage.
    pub recorder: &'a dyn Recorder,
    /// How many assets are built at once.
    pub threads: usize,
    /// The assets that the previous run built from tables.
    pub previous: &'a [TableOutput],
}

/// What the assets that `converters` build follow from besides their reads: the form and version
/// of each converter, under `table <name>`. An asset is kept from the previous run only while
/// these and its reads are the same.
pub fn evaluation_identity(converters: &[Box<dyn TableConverter>]) -> BTreeMap<String, String> {
    converters
        .iter()
        .map(|converter| {
            let name = converter.name();
            let identity = format!("{name} {}", converter.version());
            (format!("table {name}"), identity)
        })
        .collect()
}

/// An asset built from tables, or kept from the previous run.
#[derive(Debug, Clone)]
pub struct Built {
    /// The asset.
    pub output: Output,
    /// What it read, when its plan is reusable.
    pub trace: Option<Trace>,
    /// Whether it was kept from the previous run.
    pub kept: bool,
}

/// The assets of the version that the table converters read.
pub struct VersionAssets<'a> {
    /// The assets that were produced, by the bundle of their root object, in name order.
    pub by_bundle: BTreeMap<String, Vec<AssetEntry>>,
    /// The asset document of a produced asset, by name.
    pub document: &'a (dyn Fn(&str) -> io::Result<Option<Key>> + Sync),
}

/// The assets of the version and the tables, as the converters read them.
struct Assets<'a> {
    read: &'a (dyn Fn(&str) -> io::Result<Vec<u8>> + Sync),
    store: &'a dyn Store,
    version: &'a VersionAssets<'a>,
    /// How many members [`TableContext::members`] reads at once.
    threads: usize,
}

fn store_failed(e: impl std::fmt::Display) -> ConvertError {
    ConvertError::new(STORE_FAILED, e.to_string())
}

impl Assets<'_> {
    /// The key of the document of the asset `name`.
    fn key(&self, name: &str) -> Result<Key, ConvertError> {
        (self.version.document)(name)
            .map_err(store_failed)?
            .ok_or_else(|| ConvertError::new("missing_asset", format!("no asset {name}")))
    }
}

impl TableContext for Assets<'_> {
    fn table(&self, name: &str) -> Result<Vec<u8>, ConvertError> {
        (self.read)(name).map_err(|e| ConvertError::new("table_unreadable", e.to_string()))
    }

    fn bundles(&self) -> Vec<&str> {
        self.version.by_bundle.keys().map(String::as_str).collect()
    }

    fn assets(&self, bundle: &str) -> &[AssetEntry] {
        self.version
            .by_bundle
            .get(bundle)
            .map_or(&[], Vec::as_slice)
    }

    fn document(&self, name: &str) -> Result<AssetDocument, ConvertError> {
        let bytes = self.store.read(&self.key(name)?).map_err(store_failed)?;
        AssetDocument::from_bytes(&bytes).map_err(store_failed)
    }

    fn identity(&self, name: &str) -> Result<String, ConvertError> {
        Ok(self.key(name)?.to_string())
    }

    fn member(&self, name: &str, role: &str) -> Result<Vec<u8>, ConvertError> {
        let document = self.document(name)?;
        let member = document
            .members
            .iter()
            .find(|member| member.role == role)
            .ok_or_else(|| ConvertError::new("missing_asset", format!("{name} has no {role}")))?;
        let key = Key {
            sha256: parse_hex(&member.sha256)
                .ok_or_else(|| store_failed(format!("{name}: bad member hash")))?,
            extension: member.extension.clone(),
        };
        self.store.read(&key).map_err(store_failed)
    }

    fn members(&self, names: &[&str], role: &str) -> Vec<Result<Vec<u8>, ConvertError>> {
        parallel(names.len(), self.threads, |i| self.member(names[i], role))
    }
}

/// The fingerprint of an asset built from tables: everything that determines its document.
fn fingerprint(
    converter: &dyn TableConverter,
    name: &str,
    members: &[MemberFile],
    references: &[String],
    error: Option<&AssetError>,
) -> String {
    let mut text = format!(
        "table asset 1\nform {} {}\nname {name}\n",
        converter.name(),
        converter.version()
    );
    for member in members {
        text.push_str(&format!(
            "member {} {} {}\n",
            member.role, member.sha256, member.extension
        ));
    }
    for reference in references {
        text.push_str(&format!("reference {reference}\n"));
    }
    if let Some(error) = error {
        text.push_str(&format!("error {} {}\n", error.code, error.message));
    }
    hex(&Sha256::digest(text.as_bytes()))
}

/// Stores the members of an asset. The error is that of a member that could not be encoded.
fn store_members(
    members: &[Member],
    store: &dyn Store,
) -> io::Result<Result<Vec<MemberFile>, AssetError>> {
    let mut files = Vec::with_capacity(members.len());
    for member in members {
        let file = match encode(&member.content) {
            Ok(file) => file,
            Err(e) => {
                return Ok(Err(AssetError {
                    code: "encode_failed".to_owned(),
                    message: e.to_string(),
                }));
            }
        };
        let key = store.put(&file.bytes, file.extension)?;
        files.push(MemberFile {
            role: member.role.clone(),
            sha256: hex(&key.sha256),
            size: file.bytes.len() as u64,
            extension: file.extension.to_owned(),
            media_type: file.media_type.to_owned(),
        });
    }
    Ok(Ok(files))
}

/// What the previous run built for the asset of `job`, when its plan is reusable and its reads
/// give what they gave then.
fn kept(
    stage: &TableStage<'_>,
    job: Job<'_>,
    previous: &HashMap<&str, &TableOutput>,
    values: &Values<'_>,
) -> io::Result<Option<Built>> {
    let name = &job.plan.names()[job.index];
    let Some(old) = previous.get(name.as_str()).filter(|_| job.plan.reusable()) else {
        return Ok(None);
    };
    let (Some(trace), Ok(document)) = (&old.trace, old.document.parse()) else {
        return Ok(None);
    };
    let _span = stage.recorder.span("keep", job.converter.name(), name);
    if job.digest(values, &trace.reads)?.as_ref() != Some(&trace.digest) {
        return Ok(None);
    }
    Ok(Some(Built {
        output: Output {
            name: name.clone(),
            bundle: String::new(),
            form: job.converter.name(),
            document,
            error: old.error.clone(),
        },
        trace: Some(trace.clone()),
        kept: true,
    }))
}

/// Builds the asset of `job` with a context on `assets` that notes its reads.
fn build_asset(
    stage: &TableStage<'_>,
    assets: &Assets<'_>,
    values: &Values<'_>,
    job: Job<'_>,
) -> io::Result<Built> {
    let Job {
        converter,
        plan,
        index,
    } = job;
    let name = &plan.names()[index];
    let _span = stage.recorder.span("convert", converter.name(), name);
    let traced = Traced::new(assets);
    let (members, mut references, error) = match plan.convert(index, &traced) {
        Err(e) if e.code == STORE_FAILED => return Err(io::Error::other(e.message)),
        Err(e) => (Vec::new(), Vec::new(), Some(asset_error(&e))),
        Ok(TableAsset { asset, references }) => match store_members(&asset.members, stage.store)? {
            Ok(files) => (files, references, None),
            Err(error) => (Vec::new(), Vec::new(), Some(error)),
        },
    };
    references.sort();
    references.dedup();
    let document = AssetDocument {
        format: FORMAT.to_owned(),
        version: VERSION,
        name: name.clone(),
        form: converter.name().to_owned(),
        form_version: converter.version(),
        fingerprint: fingerprint(converter, name, &members, &references, error.as_ref()),
        sources: Vec::new(),
        resources: Vec::new(),
        members,
        references,
        error,
    };
    let bytes = document.to_bytes().map_err(io::Error::other)?;
    let output = Output {
        name: name.clone(),
        bundle: String::new(),
        form: converter.name(),
        document: stage.store.put(&bytes, "json")?,
        error: document.error.map(|e| e.code),
    };
    let trace = if plan.reusable() {
        let reads = traced.reads();
        let digest = job.digest(values, &reads)?;
        digest.map(|digest| Trace { reads, digest })
    } else {
        None
    };
    Ok(Built {
        output,
        trace,
        kept: false,
    })
}

/// Builds the assets of the table converters and adds them to `evaluation`, whose produced
/// assets the converters read. Fails if a built asset has the name of another asset.
///
/// A table that cannot be read or parsed and a failure of the store fail the run. An asset that
/// cannot be built is an asset error.
pub fn evaluate(stage: &TableStage<'_>, evaluation: &mut Evaluation) -> io::Result<()> {
    let produced = || evaluation.outputs.iter().filter(|o| o.error.is_none());
    let mut by_bundle: BTreeMap<String, Vec<AssetEntry>> = BTreeMap::new();
    for output in produced() {
        by_bundle
            .entry(output.bundle.clone())
            .or_default()
            .push(AssetEntry {
                name: output.name.clone(),
                form: output.form,
            });
    }
    let documents: HashMap<&str, &Key> = produced()
        .map(|output| (output.name.as_str(), &output.document))
        .collect();
    let document = |name: &str| Ok(documents.get(name).map(|key| (*key).clone()));
    let version = VersionAssets {
        by_bundle,
        document: &document,
    };
    let built = build(stage, &version)?;
    evaluation.converted += built.iter().filter(|built| !built.kept).count();
    evaluation.reused += built.iter().filter(|built| built.kept).count();
    evaluation
        .outputs
        .extend(built.into_iter().map(|built| built.output));
    evaluation.outputs.sort_by(|a, b| a.name.cmp(&b.name));
    match evaluation
        .outputs
        .windows(2)
        .find(|o| o[0].name == o[1].name)
    {
        Some(same) => Err(io::Error::other(format!(
            "two assets are named {}",
            same[0].name
        ))),
        None => Ok(()),
    }
}

/// Builds the assets of the table converters from the tables and the assets of `version`, or
/// keeps those of the previous run whose reads give what they gave then, and returns them in name
/// order.
///
/// A table that cannot be read or parsed and a failure of the store fail the run. An asset that
/// cannot be built is an asset error.
pub fn build(stage: &TableStage<'_>, version: &VersionAssets<'_>) -> io::Result<Vec<Built>> {
    let assets = Assets {
        read: stage.read,
        store: stage.store,
        version,
        threads: stage.threads,
    };
    let plans = stage.converters.iter().map(|converter| {
        let form = converter.name();
        let _span = stage.recorder.span("plan", form, "");
        let plan = converter.plan(&assets);
        plan.map_err(|e| io::Error::other(format!("{form}: {e}")))
    });
    let plans = plans.collect::<io::Result<Vec<_>>>()?;
    let jobs = jobs(stage.converters, &plans);
    let previous: HashMap<&str, &TableOutput> = stage
        .previous
        .iter()
        .map(|output| (output.name.as_str(), output))
        .collect();
    let values = Values::new(&assets);
    let built = parallel(jobs.len(), stage.threads, |i| {
        let job = jobs[i];
        match kept(stage, job, &previous, &values)? {
            Some(kept) => Ok(kept),
            None => build_asset(stage, &assets, &values, job),
        }
    });
    let mut built = built.into_iter().collect::<io::Result<Vec<_>>>()?;
    built.sort_by(|a, b| a.output.name.cmp(&b.output.name));
    Ok(built)
}

/// The asset at `index` of the plan of a converter.
#[derive(Clone, Copy)]
struct Job<'p> {
    converter: &'p dyn TableConverter,
    plan: &'p dyn TablePlan,
    index: usize,
}

impl Job<'_> {
    /// The digest of the form of the asset and of what `reads` give for it.
    fn digest(&self, values: &Values<'_>, reads: &[Read]) -> io::Result<Option<String>> {
        let converter = self.converter;
        values.digest(converter.name(), converter.version(), reads, self.plan)
    }
}

/// Every asset of `plans`, the plans of `converters`, to be built on one pool of threads. The
/// assets of a plan with fewer assets come first: each of them is built from more of the
/// version, and starting them first keeps them from holding up the end of the stage.
fn jobs<'p>(
    converters: &'p [Box<dyn TableConverter>],
    plans: &'p [Box<dyn TablePlan + '_>],
) -> Vec<Job<'p>> {
    let mut planned: Vec<(&dyn TableConverter, &dyn TablePlan)> = converters
        .iter()
        .map(Box::as_ref)
        .zip(plans.iter().map(Box::as_ref))
        .collect();
    planned.sort_by_key(|(_, plan)| plan.names().len());
    let jobs = planned.into_iter().flat_map(|(converter, plan)| {
        (0..plan.names().len()).map(move |index| Job {
            converter,
            plan,
            index,
        })
    });
    jobs.collect()
}
