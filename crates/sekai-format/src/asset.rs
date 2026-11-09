//! Asset documents, specified in `docs/spec/asset-documents.md`.

use serde::{Deserialize, Serialize};

/// The value of the `format` key.
pub const FORMAT: &str = "asset";
/// The version of the document format.
pub const VERSION: u32 = 1;

/// What one asset is: its form, its source objects and its files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetDocument {
    /// Always [`FORMAT`].
    pub format: String,
    /// The version of the document format.
    pub version: u32,
    /// The name of the asset.
    pub name: String,
    /// The asset form.
    pub form: String,
    /// The version of the asset form.
    pub form_version: u32,
    /// The fingerprint, as lowercase hexadecimal.
    pub fingerprint: String,
    /// The root object first, then the rest of the closure ordered by bundle and path ID.
    pub sources: Vec<SourceObject>,
    /// The canonical file of each member, in the order the form defines.
    pub members: Vec<MemberFile>,
    /// The names of the assets this asset refers to, in ascending order.
    pub references: Vec<String>,
    /// Why the asset could not be produced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AssetError>,
}

impl AssetDocument {
    /// The bytes of the document.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// Reads a document.
    pub fn from_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

/// An object an asset was made from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceObject {
    /// The bundle name.
    pub bundle: String,
    /// The path ID.
    #[serde(with = "decimal")]
    pub path_id: i64,
    /// The class name.
    pub class: String,
}

/// The canonical file of a member.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberFile {
    /// What the member is to the asset.
    pub role: String,
    /// The SHA-256 of the file, as lowercase hexadecimal.
    pub sha256: String,
    /// The size in bytes.
    pub size: u64,
    /// The file extension, without the dot.
    pub extension: String,
    /// The media type.
    pub media_type: String,
}

/// Why an asset could not be produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetError {
    /// A fixed code.
    pub code: String,
    /// A description for people.
    pub message: String,
}

/// 64-bit integers as decimal strings, so that they survive JSON readers.
mod decimal {
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub(super) fn serialize<S: Serializer>(value: &i64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(value)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> AssetDocument {
        AssetDocument {
            format: FORMAT.to_owned(),
            version: VERSION,
            name: "a/b@-5".to_owned(),
            form: "texture".to_owned(),
            form_version: 1,
            fingerprint: "ab".to_owned(),
            sources: vec![SourceObject {
                bundle: "a".to_owned(),
                path_id: -5,
                class: "Texture2D".to_owned(),
            }],
            members: vec![MemberFile {
                role: "image".to_owned(),
                sha256: "cd".to_owned(),
                size: 3,
                extension: "webp".to_owned(),
                media_type: "image/webp".to_owned(),
            }],
            references: vec![],
            error: None,
        }
    }

    #[test]
    fn the_bytes_are_fixed() {
        assert_eq!(
            String::from_utf8(document().to_bytes().unwrap()).unwrap(),
            r#"{"format":"asset","version":1,"name":"a/b@-5","form":"texture","form_version":1,"fingerprint":"ab","sources":[{"bundle":"a","path_id":"-5","class":"Texture2D"}],"members":[{"role":"image","sha256":"cd","size":3,"extension":"webp","media_type":"image/webp"}],"references":[]}"#
        );
    }

    #[test]
    fn a_document_reads_back() {
        let mut failed = document();
        failed.members.clear();
        failed.error = Some(AssetError {
            code: "unsupported".to_owned(),
            message: "format 1".to_owned(),
        });
        for d in [document(), failed] {
            assert_eq!(
                AssetDocument::from_bytes(&d.to_bytes().unwrap()).unwrap(),
                d
            );
        }
    }

    #[test]
    fn a_path_id_is_a_decimal_string() {
        let text = br#"{"bundle":"a","path_id":"x","class":"c"}"#;
        assert!(serde_json::from_slice::<SourceObject>(text).is_err());
    }
}
