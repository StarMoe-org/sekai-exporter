//! The table stage: assets built from game data tables and the assets of the version.

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
}

/// The assets of the version, as the converters read them.
struct Assets<'a> {
    read: &'a (dyn Fn(&str) -> io::Result<Vec<u8>> + Sync),
    store: &'a dyn Store,
    by_bundle: BTreeMap<&'a str, Vec<AssetEntry>>,
    documents: HashMap<&'a str, &'a Key>,
}

fn store_failed(e: impl std::fmt::Display) -> ConvertError {
    ConvertError::new(STORE_FAILED, e.to_string())
}

impl TableContext for Assets<'_> {
    fn table(&self, name: &str) -> Result<Vec<u8>, ConvertError> {
        (self.read)(name).map_err(|e| ConvertError::new("table_unreadable", e.to_string()))
    }

    fn bundles(&self) -> Vec<&str> {
        self.by_bundle.keys().copied().collect()
    }

    fn assets(&self, bundle: &str) -> &[AssetEntry] {
        self.by_bundle.get(bundle).map_or(&[], Vec::as_slice)
    }

    fn document(&self, name: &str) -> Result<AssetDocument, ConvertError> {
        let key = self
            .documents
            .get(name)
            .ok_or_else(|| ConvertError::new("missing_asset", format!("no asset {name}")))?;
        let bytes = self.store.read(key).map_err(store_failed)?;
        AssetDocument::from_bytes(&bytes).map_err(store_failed)
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

fn build(
    stage: &TableStage<'_>,
    converter: &dyn TableConverter,
    plan: &dyn TablePlan,
    index: usize,
) -> io::Result<Output> {
    let name = &plan.names()[index];
    let _span = stage.recorder.span("convert", converter.name(), name);
    let (members, mut references, error) = match plan.convert(index) {
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
    Ok(Output {
        name: name.clone(),
        bundle: String::new(),
        form: converter.name(),
        document: stage.store.put(&bytes, "json")?,
        error: document.error.map(|e| e.code),
    })
}

/// Builds the assets of the table converters and adds them to `evaluation`.
///
/// A table that cannot be read or parsed and a failure of the store fail the run. An asset that
/// cannot be built is an asset error.
pub fn evaluate(stage: &TableStage<'_>, evaluation: &mut Evaluation) -> io::Result<()> {
    let mut by_bundle: BTreeMap<&str, Vec<AssetEntry>> = BTreeMap::new();
    let mut documents = HashMap::with_capacity(evaluation.outputs.len());
    for output in evaluation.outputs.iter().filter(|o| o.error.is_none()) {
        documents.insert(output.name.as_str(), &output.document);
        by_bundle
            .entry(output.bundle.as_str())
            .or_default()
            .push(AssetEntry {
                name: output.name.clone(),
                form: output.form,
            });
    }
    let assets = Assets {
        read: stage.read,
        store: stage.store,
        by_bundle,
        documents,
    };
    let mut built = Vec::new();
    for converter in stage.converters {
        let form = converter.name();
        let plan = {
            let _span = stage.recorder.span("plan", form, "");
            converter
                .plan(&assets)
                .map_err(|e| io::Error::other(format!("{form}: {e}")))?
        };
        let results = parallel(plan.names().len(), stage.threads, |i| {
            build(stage, converter.as_ref(), plan.as_ref(), i)
        });
        for result in results {
            built.push(result?);
        }
    }
    evaluation.converted += built.len();
    evaluation.outputs.extend(built);
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
