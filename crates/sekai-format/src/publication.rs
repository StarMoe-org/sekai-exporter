//! Index shards, listings, index roots and pointers, specified in `docs/spec/publication.md`.

use serde::{Deserialize, Serialize};

/// The version of these document formats.
pub const VERSION: u32 = 2;

/// Whether `text` can name a region, a view or a published version in a path: ASCII letters,
/// digits, `.`, `_` and `-`, and not only dots.
pub fn is_name(text: &str) -> bool {
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-');
    !text.is_empty() && text.chars().all(allowed) && !text.chars().all(|c| c == '.')
}

/// One published file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexEntry {
    /// The file name within its directory.
    pub name: String,
    /// The SHA-256 of the file, as lowercase hexadecimal.
    pub sha256: String,
    /// The file extension in the store, without the dot.
    pub extension: String,
    /// The size in bytes.
    pub size: u64,
    /// The media type.
    pub media_type: String,
    /// The name of the asset that owns the file.
    pub asset: String,
    /// The role of the file in the asset.
    pub role: String,
}

/// The published files of one directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexShard {
    /// Always `"index-shard"`.
    pub format: String,
    /// The version of the format.
    pub version: u32,
    /// The directory, empty for the top level.
    pub directory: String,
    /// The files, in name order.
    pub entries: Vec<IndexEntry>,
}

/// One asset in a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingEntry {
    /// The name of the asset.
    pub name: String,
    /// The fingerprint, as lowercase hexadecimal.
    pub fingerprint: String,
    /// The total size of the members in bytes.
    pub size: u64,
}

/// Every asset of one form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    /// Always `"listing"`.
    pub format: String,
    /// The version of the format.
    pub version: u32,
    /// The asset form.
    pub form: String,
    /// The assets, in name order.
    pub entries: Vec<ListingEntry>,
}

/// A shard named by its directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardReference {
    /// The directory, empty for the top level.
    pub directory: String,
    /// The SHA-256 of the shard, as lowercase hexadecimal.
    pub sha256: String,
}

/// A listing named by its form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingReference {
    /// The asset form.
    pub form: String,
    /// The SHA-256 of the listing, as lowercase hexadecimal.
    pub sha256: String,
}

/// An asset that could not be produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetFailure {
    /// The name of the asset.
    pub name: String,
    /// The error code.
    pub code: String,
}

/// Everything a publication contains.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRoot {
    /// Always `"index-root"`.
    pub format: String,
    /// The version of the format.
    pub version: u32,
    /// The name of the published version.
    pub published_version: String,
    /// The revision: the hash of the sorted records of the publication.
    pub revision: String,
    /// The shards, in directory order.
    pub shards: Vec<ShardReference>,
    /// The listings, in form order.
    pub listings: Vec<ListingReference>,
    /// The assets that could not be produced, in name order.
    pub failures: Vec<AssetFailure>,
}

/// The current publication of a region and view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pointer {
    /// Always `"pointer"`.
    pub format: String,
    /// The version of the format.
    pub version: u32,
    /// The name of the published version.
    pub published_version: String,
    /// The revision of the publication.
    pub revision: String,
    /// The SHA-256 of the index root, as lowercase hexadecimal. A view without index shards has
    /// none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// The SHA-256 of the SQLite index (`store/<aa>/<sha256>.sqlite`), as lowercase hexadecimal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sqlite: Option<String>,
    /// The SHA-256 of the root of the path index (`store/<aa>/<sha256>.json`), as lowercase
    /// hexadecimal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paths: Option<String>,
}

/// One file of a path tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    /// The path of the file.
    pub path: String,
    /// The SHA-256 of the file, as lowercase hexadecimal.
    pub sha256: String,
    /// The name of the asset that owns the file.
    pub asset: String,
}

/// The files of a path tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeListing {
    /// Always `"tree-listing"`.
    pub format: String,
    /// The version of the format.
    pub version: u32,
    /// The name of the published version.
    pub published_version: String,
    /// The files, in path order.
    pub entries: Vec<TreeEntry>,
}
