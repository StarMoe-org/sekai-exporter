//! Index shards, listings, index roots and pointers, specified in `docs/spec/publication.md`.

use serde::{Deserialize, Serialize};

use crate::chunks::{self, Row};

/// The version of these document formats.
pub const VERSION: u32 = 3;

/// A chunk of a table of the index root ends after a row whose key hashes to a multiple of this,
/// so chunks hold this many rows on average.
const AVERAGE: u32 = 512;

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

/// A shard named by its directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShardReference {
    /// The directory, empty for the top level.
    pub directory: String,
    /// The SHA-256 of the shard, as lowercase hexadecimal.
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

/// A chunk of a table, as the index root lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableChunk {
    /// The key of the first row: a directory, or an asset name.
    pub first: String,
    /// The SHA-256 of the chunk, as lowercase hexadecimal.
    pub sha256: String,
    /// The number of rows.
    pub rows: u64,
}

/// A chunk of the rows of a table, in key order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexChunk<R> {
    /// Always `"index-chunk"`.
    pub format: String,
    /// The version of the format.
    pub version: u32,
    /// The rows.
    pub rows: Vec<R>,
}

/// The listing of a form, as the chunks of its entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListingTable {
    /// The asset form.
    pub form: String,
    /// The chunks of [`ListingEntry`] rows, in name order.
    pub chunks: Vec<TableChunk>,
}

/// Everything a publication contains.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRoot {
    /// Always `"index-root"`.
    pub format: String,
    /// The version of the format.
    pub version: u32,
    /// The name of the published version.
    pub published_version: String,
    /// The revision of the publication.
    pub revision: String,
    /// The chunks of [`ShardReference`] rows, in directory order.
    pub directories: Vec<TableChunk>,
    /// The listing of each form, in form order.
    pub listings: Vec<ListingTable>,
    /// The chunks of [`AssetFailure`] rows, in name order.
    pub failures: Vec<TableChunk>,
}

/// Makes each type a row of a table of the index root, keyed by the field given with it.
macro_rules! index_rows {
    ($($row:ty => $key:ident),* $(,)?) => {$(
        impl Row for $row {
            type Key = String;
            const MOST: usize = 4 * AVERAGE as usize;

            fn key(&self) -> &String {
                &self.$key
            }

            fn ends_chunk(&self) -> bool {
                chunks::ends_after(self.$key.as_bytes(), AVERAGE)
            }
        }
    )*};
}

index_rows!(ShardReference => directory, ListingEntry => name, AssetFailure => name);

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
