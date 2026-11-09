//! The manifest document.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::crypto::Keys;
use super::{BundleEntry, SourceError};

#[derive(Deserialize)]
#[serde(untagged)]
enum Scalar {
    Text(String),
    Unsigned(u64),
    Signed(i64),
}

impl Scalar {
    fn text(self) -> String {
        match self {
            Scalar::Text(text) => text,
            Scalar::Unsigned(value) => value.to_string(),
            Scalar::Signed(value) => value.to_string(),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    #[serde(default)]
    hash: Option<Scalar>,
    #[serde(default)]
    crc: Option<Scalar>,
    #[serde(default)]
    download_path: Option<String>,
    #[serde(default)]
    file_size: u64,
    #[serde(default)]
    dependencies: Vec<String>,
}

#[derive(Deserialize)]
struct Document {
    bundles: BTreeMap<String, Row>,
}

/// Which field of a row holds the declared hash.
#[derive(Clone, Copy)]
pub(super) enum Declared {
    Hash,
    Crc,
}

/// Reads an encrypted manifest, in name order.
pub(super) fn read(
    keys: &Keys,
    body: &[u8],
    declared: Declared,
) -> Result<Vec<BundleEntry>, SourceError> {
    let document: Document = keys.decode(body)?;
    Ok(document
        .bundles
        .into_iter()
        .map(|(name, row)| BundleEntry {
            name,
            declared: match declared {
                Declared::Hash => row.hash,
                Declared::Crc => row.crc,
            }
            .map(Scalar::text)
            .unwrap_or_default(),
            download_path: row.download_path,
            size: row.file_size,
            dependencies: row.dependencies,
        })
        .collect())
}
