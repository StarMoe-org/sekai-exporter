//! What a publication contains: every file of its assets, with its role and owner.

use std::collections::BTreeMap;
use std::fmt;
use std::io;

use sekai_catalog::store::parse_hex;
use sekai_catalog::{Key, LocalStore};
use sekai_format::AssetDocument;
use sekai_format::publication::{AssetFailure, ListingEntry};
use sha2::{Digest, Sha256};

/// A failure of a publication, with the name of the stage that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishError {
    /// The stage: `config`, `plan`, `documents`, `verify`, `files`, `listing`, `delete` or
    /// `pointer`.
    pub stage: &'static str,
    /// A description for people.
    pub message: String,
}

impl PublishError {
    /// An error of `stage`.
    pub fn new(stage: &'static str, message: impl fmt::Display) -> Self {
        Self {
            stage,
            message: message.to_string(),
        }
    }

    pub(crate) fn wrap(stage: &'static str) -> impl Fn(io::Error) -> Self {
        move |e| Self::new(stage, e)
    }
}

impl fmt::Display for PublishError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "publication failed at {}: {}", self.stage, self.message)
    }
}

impl std::error::Error for PublishError {}

/// An asset to publish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// The name of the asset.
    pub name: String,
    /// The asset document.
    pub document: Key,
}

/// One file of a publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    /// The path of the file.
    pub path: String,
    /// The object in the store.
    pub key: Key,
    /// The size in bytes.
    pub size: u64,
    /// The media type.
    pub media_type: String,
    /// The name of the asset that owns the file.
    pub asset: String,
    /// The role of the file in the asset.
    pub role: String,
}

/// A published asset and its decoded asset document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishedAsset {
    /// The name of the asset.
    pub name: String,
    /// The bundle of the asset's root object, or empty when no source is listed.
    pub bundle: String,
    /// The asset document in the store.
    pub document: Key,
    /// The parsed asset document.
    pub metadata: AssetDocument,
}

/// Everything a publication contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The name of the published version.
    pub version: String,
    /// The files, in path order.
    pub files: Vec<File>,
    /// The assets, in name order.
    pub assets: Vec<PublishedAsset>,
    /// The listing of each asset form, in name order.
    pub listings: BTreeMap<String, Vec<ListingEntry>>,
    /// The assets that could not be produced, in name order.
    pub failures: Vec<AssetFailure>,
    /// The hash of the sorted records of the publication, as lowercase hexadecimal.
    pub revision: String,
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The path of a member: `<name>.<extension>` for an asset of one member, and
/// `<name>/<role>.<extension>` for an asset of several.
fn member_path(name: &str, role: &str, extension: &str, members: usize) -> String {
    if members == 1 {
        format!("{name}.{extension}")
    } else {
        format!("{name}/{role}.{extension}")
    }
}

/// Reads the asset documents of `bindings` and lists what a publication of them contains.
pub fn plan(version: &str, bindings: &[Binding], store: &LocalStore) -> Result<Plan, PublishError> {
    let mut files = Vec::new();
    let mut assets = Vec::with_capacity(bindings.len());
    let mut listings: BTreeMap<String, Vec<ListingEntry>> = BTreeMap::new();
    let mut failures = Vec::new();
    let wrap = PublishError::wrap("plan");
    for binding in bindings {
        let bytes = store.read(&binding.document).map_err(&wrap)?;
        let document = AssetDocument::from_bytes(&bytes)
            .map_err(|e| PublishError::new("plan", format!("{}: {e}", binding.name)))?;
        files.push(File {
            path: format!("{}.asset.json", binding.name),
            key: binding.document.clone(),
            size: bytes.len() as u64,
            media_type: "application/json".to_owned(),
            asset: binding.name.clone(),
            role: "document".to_owned(),
        });
        let bundle = document
            .sources
            .first()
            .map(|source| source.bundle.clone())
            .unwrap_or_default();
        if let Some(error) = &document.error {
            failures.push(AssetFailure {
                name: binding.name.clone(),
                code: error.code.clone(),
            });
            assets.push(PublishedAsset {
                name: binding.name.clone(),
                bundle,
                document: binding.document.clone(),
                metadata: document,
            });
            continue;
        }
        let mut total = 0;
        for member in &document.members {
            let sha256 = parse_hex(&member.sha256).ok_or_else(|| {
                PublishError::new("plan", format!("{}: bad member hash", binding.name))
            })?;
            total += member.size;
            files.push(File {
                path: member_path(
                    &binding.name,
                    &member.role,
                    &member.extension,
                    document.members.len(),
                ),
                key: Key {
                    sha256,
                    extension: member.extension.clone(),
                },
                size: member.size,
                media_type: member.media_type.clone(),
                asset: binding.name.clone(),
                role: member.role.clone(),
            });
        }
        listings
            .entry(document.form.clone())
            .or_default()
            .push(ListingEntry {
                name: binding.name.clone(),
                fingerprint: document.fingerprint.clone(),
                size: total,
            });
        assets.push(PublishedAsset {
            name: binding.name.clone(),
            bundle,
            document: binding.document.clone(),
            metadata: document,
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if let Some(pair) = files.windows(2).find(|pair| pair[0].path == pair[1].path) {
        return Err(PublishError::new(
            "plan",
            format!("two files are published at {}", pair[0].path),
        ));
    }
    assets.sort_by(|a, b| a.name.cmp(&b.name));
    for listing in listings.values_mut() {
        listing.sort_by(|a, b| a.name.cmp(&b.name));
    }
    failures.sort_by(|a, b| a.name.cmp(&b.name));

    let mut records = Sha256::new();
    records.update(format!("publication 1\nversion {version}\n"));
    for file in &files {
        records.update(format!(
            "file {}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            file.path,
            hex(&file.key.sha256),
            file.key.extension,
            file.size,
            file.media_type,
            file.asset,
            file.role
        ));
    }
    for failure in &failures {
        records.update(format!("failure {}\t{}\n", failure.name, failure.code));
    }
    Ok(Plan {
        version: version.to_owned(),
        files,
        assets,
        listings,
        failures,
        revision: hex(&records.finalize()),
    })
}
