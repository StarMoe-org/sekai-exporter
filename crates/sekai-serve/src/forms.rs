//! Form listings: the forms of a publication and the assets of a form, gathered from the index
//! root of the publication and the chunks of its listings table.

use std::io;
use std::mem::size_of;
use std::sync::Arc;

use sekai_format::publication::{
    IndexChunk, IndexRoot, ListingEntry, VERSION as PUBLICATION_VERSION,
};
use serde::{Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::listing::Listed;
use crate::resolver::{INDEX_BYTES, parse_sha256};

/// The version of the `form-listing` and `form-listings` response formats.
pub(crate) const VERSION: u32 = 1;

/// The most bytes a gathered form listing may take, in memory or as JSON.
const MAX_LISTING_BYTES: usize = INDEX_BYTES as usize;

/// The form listings of a publication's index root, without its directory shards or failures.
#[derive(Debug)]
pub(crate) struct FormIndex {
    /// The name of the published version.
    pub(crate) published_version: String,
    /// The revision of the publication.
    pub(crate) revision: String,
    /// The listings, in strictly increasing byte order of `form`.
    forms: Vec<FormTable>,
}

/// One form in [`FormIndex`].
#[derive(Debug)]
pub(crate) struct FormTable {
    /// The asset form.
    pub(crate) form: String,
    /// Total number of assets across its chunks.
    pub(crate) assets: u64,
    /// The chunks of [`ListingEntry`] rows, in name order.
    pub(crate) chunks: Vec<FormChunkRef>,
}

/// A chunk of a form's listing, as [`FormIndex`] keeps it.
#[derive(Debug)]
pub(crate) struct FormChunkRef {
    /// The name of the first asset in the chunk.
    pub(crate) first: String,
    /// The SHA-256 of the chunk object (`<aa>/<sha256>.json`).
    pub(crate) sha256: [u8; 32],
    /// The number of rows in the chunk.
    pub(crate) rows: u64,
}

impl FormIndex {
    /// Decodes an index root, checking its format, version, ordering and listing chunk hashes.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let root: IndexRoot = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if root.format != "index-root" || root.version != PUBLICATION_VERSION {
            return Err(format!(
                "format {} version {} is not index-root version {PUBLICATION_VERSION}",
                root.format, root.version
            ));
        }
        if root
            .listings
            .windows(2)
            .any(|pair| pair[0].form >= pair[1].form)
        {
            return Err("listings are not in form order".to_owned());
        }
        let mut forms = Vec::with_capacity(root.listings.len());
        for table in root.listings {
            if table.chunks.is_empty() {
                return Err(format!("listing of {:?} has no chunks", table.form));
            }
            if table
                .chunks
                .windows(2)
                .any(|pair| pair[0].first >= pair[1].first)
            {
                return Err(format!("chunks of {:?} are not in name order", table.form));
            }
            let mut assets = 0u64;
            let mut chunks = Vec::with_capacity(table.chunks.len());
            for (at, chunk) in table.chunks.into_iter().enumerate() {
                if chunk.rows == 0 {
                    return Err(format!("chunk {at} of {:?} has no rows", table.form));
                }
                let sha256 = parse_sha256(&chunk.sha256).ok_or_else(|| {
                    format!("chunk {at} of {:?} is not named by a SHA-256", table.form)
                })?;
                assets = assets
                    .checked_add(chunk.rows)
                    .ok_or_else(|| format!("asset count of {:?} overflows", table.form))?;
                chunks.push(FormChunkRef {
                    first: chunk.first,
                    sha256,
                    rows: chunk.rows,
                });
            }
            forms.push(FormTable {
                form: table.form,
                assets,
                chunks,
            });
        }
        Ok(Self {
            published_version: root.published_version,
            revision: root.revision,
            forms,
        })
    }

    /// The position of the table of `form`, if the publication lists it.
    fn position(&self, form: &str) -> Option<usize> {
        self.forms
            .binary_search_by(|table| table.form.as_str().cmp(form))
            .ok()
    }

    /// The memory the decoded index takes.
    pub(crate) fn weight(&self) -> usize {
        let tables: usize = self
            .forms
            .iter()
            .map(|table| {
                table.form.capacity()
                    + table.chunks.capacity() * size_of::<FormChunkRef>()
                    + table
                        .chunks
                        .iter()
                        .map(|chunk| chunk.first.capacity())
                        .sum::<usize>()
            })
            .sum();
        size_of::<Self>()
            + self.published_version.capacity()
            + self.revision.capacity()
            + self.forms.capacity() * size_of::<FormTable>()
            + tables
    }

    /// The list of forms as it is sent, describing `view`.
    pub(crate) fn listed_forms(&self, view: &str) -> Result<Listed, String> {
        self.listed_forms_within(view, MAX_LISTING_BYTES)
    }

    fn listed_forms_within(&self, view: &str, limit: usize) -> Result<Listed, String> {
        let forms: Vec<ListedForm<'_>> = self
            .forms
            .iter()
            .map(|table| ListedForm {
                form: &table.form,
                assets: table.assets,
            })
            .collect();
        let document = FormsDocument {
            format: "form-listings",
            version: VERSION,
            view,
            published_version: &self.published_version,
            revision: &self.revision,
            forms: &forms,
        };
        let document = serialize_within(&document, limit)?;
        Ok(Listed::new(document))
    }
}

/// A decoded chunk of a form's listing.
#[derive(Debug)]
pub(crate) struct FormChunk {
    /// The rows, in strictly increasing byte order of `name`.
    pub(crate) rows: Vec<ListingEntry>,
}

impl FormChunk {
    /// Decodes a listing chunk, checking its format, version and row order.
    pub(crate) fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let mut chunk: IndexChunk<ListingEntry> =
            serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if chunk.format != "index-chunk" || chunk.version != PUBLICATION_VERSION {
            return Err(format!(
                "format {} version {} is not index-chunk version {PUBLICATION_VERSION}",
                chunk.format, chunk.version
            ));
        }
        if chunk.rows.is_empty() {
            return Err("listing chunk has no rows".to_owned());
        }
        if chunk
            .rows
            .windows(2)
            .any(|pair| pair[0].name >= pair[1].name)
        {
            return Err("rows are not in name order".to_owned());
        }
        chunk.rows.shrink_to_fit();
        Ok(Self { rows: chunk.rows })
    }

    /// The memory the decoded chunk takes.
    pub(crate) fn weight(&self) -> usize {
        let rows: usize = self
            .rows
            .iter()
            .map(|row| row.name.capacity() + row.fingerprint.capacity())
            .sum();
        size_of::<Self>() + self.rows.capacity() * size_of::<ListingEntry>() + rows
    }
}

/// The estimated in-memory size of one listing row.
fn row_bytes(row: &ListingEntry) -> usize {
    size_of::<ListingEntry>() + row.name.len() + row.fingerprint.len()
}

/// The listing of one form, while its chunks are gathered as shared [`Arc<FormChunk>`] handles.
#[derive(Debug)]
pub(crate) struct FormGathering {
    index: Arc<FormIndex>,
    /// The position of the table of the form in `index`.
    at: usize,
    /// The chunks gathered so far, in name order.
    chunks: Vec<Arc<FormChunk>>,
    /// Estimated row bytes gathered so far, bounded by [`MAX_LISTING_BYTES`].
    bytes: usize,
}

impl FormGathering {
    /// Starts gathering the listing of `form` from `index`.
    ///
    /// Returns `Ok(None)` when the publication does not list `form`, or `Err` when the declared
    /// asset count cannot fit within [`MAX_LISTING_BYTES`] even at the minimum size per entry.
    pub(crate) fn new(index: Arc<FormIndex>, form: &str) -> Result<Option<Self>, String> {
        let Some(at) = index.position(form) else {
            return Ok(None);
        };
        if index.forms[at]
            .assets
            .saturating_mul(size_of::<ListingEntry>() as u64)
            > MAX_LISTING_BYTES as u64
        {
            return Err(format!(
                "listing of {form:?} exceeds {MAX_LISTING_BYTES} bytes"
            ));
        }
        Ok(Some(Self {
            index,
            at,
            chunks: Vec::new(),
            bytes: 0,
        }))
    }

    /// The table of the form being listed.
    fn table(&self) -> &FormTable {
        &self.index.forms[self.at]
    }

    /// The SHA-256 of the next chunk to read, or `None` once every chunk has been gathered.
    pub(crate) fn wanted(&self) -> Option<[u8; 32]> {
        let next = self.table().chunks.get(self.chunks.len())?;
        Some(next.sha256)
    }

    /// Appends `chunk`, the chunk that [`FormGathering::wanted`] named, checking that it matches
    /// the table entry, stays in strictly increasing name order and fits within
    /// [`MAX_LISTING_BYTES`].
    pub(crate) fn push(&mut self, chunk: Arc<FormChunk>) -> Result<(), String> {
        self.push_within(chunk, MAX_LISTING_BYTES)
    }

    fn push_within(&mut self, chunk: Arc<FormChunk>, limit: usize) -> Result<(), String> {
        let expected = self
            .table()
            .chunks
            .get(self.chunks.len())
            .ok_or_else(|| "unexpected listing chunk".to_owned())?;
        if chunk.rows.len() as u64 != expected.rows {
            return Err(format!(
                "it has {} rows and the index root says {}",
                chunk.rows.len(),
                expected.rows
            ));
        }
        let first = chunk
            .rows
            .first()
            .ok_or_else(|| "listing chunk has no rows".to_owned())?;
        if first.name != expected.first {
            return Err(format!(
                "its first row is {:?} and the index root says {:?}",
                first.name, expected.first
            ));
        }
        if let Some(last) = self.chunks.last().and_then(|prev| prev.rows.last())
            && last.name >= first.name
        {
            return Err("chunks are not in name order".to_owned());
        }
        let added = chunk
            .rows
            .iter()
            .try_fold(0usize, |acc, row| acc.checked_add(row_bytes(row)))
            .and_then(|added| self.bytes.checked_add(added));
        match added {
            Some(total) if total <= limit => self.bytes = total,
            _ => return Err(format!("listing is longer than {limit} bytes")),
        }
        self.chunks.push(chunk);
        Ok(())
    }

    /// The form listing as it is sent, describing `view`.
    pub(crate) fn listed(&self, view: &str) -> Result<Listed, String> {
        self.listed_within(view, MAX_LISTING_BYTES)
    }

    fn listed_within(&self, view: &str, limit: usize) -> Result<Listed, String> {
        let document = FormDocument {
            format: "form-listing",
            version: VERSION,
            view,
            published_version: &self.index.published_version,
            revision: &self.index.revision,
            form: &self.table().form,
            assets: ChunkAssets(&self.chunks),
        };
        let document = serialize_within(&document, limit)?;
        Ok(Listed::new(document))
    }
}

/// Serializes `value` as compact JSON into a buffer that never grows beyond `limit` bytes.
fn serialize_within(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, String> {
    let mut writer = LimitedWriter::new(limit);
    serde_json::to_writer(&mut writer, value).map_err(|e| e.to_string())?;
    Ok(writer.finish())
}

/// A writer backed by a `Vec<u8>` that stops with an error before exceeding `limit` bytes and
/// never reserves past `limit`.
struct LimitedWriter {
    bytes: Vec<u8>,
    limit: usize,
}

impl LimitedWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::new(),
            limit,
        }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

impl io::Write for LimitedWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let total = self
            .bytes
            .len()
            .checked_add(buf.len())
            .filter(|&len| len <= self.limit)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("is longer than {} bytes", self.limit),
                )
            })?;
        if self.bytes.capacity() < total {
            let needed = total - self.bytes.len();
            let remaining = self.limit - self.bytes.len();
            let doubled = self.bytes.len().min(remaining);
            self.bytes.reserve_exact(needed.max(doubled));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The key under which the listing of `form` in `view`, or the list of the forms of `view` when
/// `form` is `None`, is kept. The index root `root` is described by `published_version` and
/// `revision`, as the pointer that names it gives them.
pub(crate) fn key(
    root: &[u8; 32],
    view: &str,
    published_version: &str,
    revision: &str,
    form: Option<&str>,
) -> [u8; 32] {
    let tag: &[u8] = match form {
        Some(_) => b"form-listing\n",
        None => b"form-listings\n",
    };
    let mut hasher = Sha256::new();
    hasher.update(tag);
    hasher.update(root);
    for part in [view, published_version, revision].into_iter().chain(form) {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hasher.finalize().into()
}

#[derive(Serialize)]
struct ListedForm<'a> {
    form: &'a str,
    assets: u64,
}

#[derive(Serialize)]
struct FormsDocument<'a> {
    format: &'static str,
    version: u32,
    view: &'a str,
    published_version: &'a str,
    revision: &'a str,
    forms: &'a [ListedForm<'a>],
}

/// Serializes the rows across all gathered chunks without cloning [`ListingEntry`] values.
struct ChunkAssets<'a>(&'a [Arc<FormChunk>]);

impl Serialize for ChunkAssets<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.0.iter().flat_map(|chunk| chunk.rows.iter()))
    }
}

#[derive(Serialize)]
struct FormDocument<'a> {
    format: &'static str,
    version: u32,
    view: &'a str,
    published_version: &'a str,
    revision: &'a str,
    form: &'a str,
    assets: ChunkAssets<'a>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_index() -> Arc<FormIndex> {
        Arc::new(FormIndex {
            published_version: "1".to_owned(),
            revision: "r1".to_owned(),
            forms: vec![FormTable {
                form: "live2d-model".to_owned(),
                assets: 2,
                chunks: vec![
                    FormChunkRef {
                        first: "a".to_owned(),
                        sha256: [1; 32],
                        rows: 1,
                    },
                    FormChunkRef {
                        first: "b".to_owned(),
                        sha256: [2; 32],
                        rows: 1,
                    },
                ],
            }],
        })
    }

    #[test]
    fn gathering_refuses_listings_past_its_byte_limit() {
        let index = sample_index();
        let chunk_a = Arc::new(FormChunk {
            rows: vec![ListingEntry {
                name: "a".to_owned(),
                fingerprint: "0".repeat(64),
                size: 1,
            }],
        });
        let chunk_b = Arc::new(FormChunk {
            rows: vec![ListingEntry {
                name: "b".to_owned(),
                fingerprint: "1".to_owned(),
                size: 2,
            }],
        });
        let one_row = row_bytes(&chunk_a.rows[0]);
        let mut gathering = FormGathering::new(index, "live2d-model").unwrap().unwrap();
        assert!(gathering.push_within(chunk_a, one_row).is_ok());
        assert!(gathering.push_within(chunk_b, one_row).is_err());
    }

    #[test]
    fn gathering_fails_fast_when_declared_assets_exceed_limit() {
        let too_many = (MAX_LISTING_BYTES as u64 / size_of::<ListingEntry>() as u64) + 1;
        let index = Arc::new(FormIndex {
            published_version: "1".to_owned(),
            revision: "r1".to_owned(),
            forms: vec![FormTable {
                form: "huge".to_owned(),
                assets: too_many,
                chunks: vec![FormChunkRef {
                    first: "a".to_owned(),
                    sha256: [1; 32],
                    rows: too_many,
                }],
            }],
        });
        assert!(FormGathering::new(index, "huge").is_err());
    }

    #[test]
    fn bounded_serialization_rejects_oversized_form_documents() {
        let index = sample_index();
        let full_forms = index.listed_forms("canonical").unwrap();
        assert!(
            index
                .listed_forms_within("canonical", full_forms.document.len())
                .is_ok()
        );
        assert!(
            index
                .listed_forms_within("canonical", full_forms.document.len() - 1)
                .is_err()
        );

        let chunk_a = Arc::new(FormChunk {
            rows: vec![ListingEntry {
                name: "a".to_owned(),
                fingerprint: "0".repeat(64),
                size: 1,
            }],
        });
        let chunk_b = Arc::new(FormChunk {
            rows: vec![ListingEntry {
                name: "b".to_owned(),
                fingerprint: "1".repeat(64),
                size: 2,
            }],
        });
        let mut gathering = FormGathering::new(index, "live2d-model").unwrap().unwrap();
        gathering.push(chunk_a).unwrap();
        gathering.push(chunk_b).unwrap();
        let full = gathering.listed("canonical").unwrap();
        assert!(
            gathering
                .listed_within("canonical", full.document.len())
                .is_ok()
        );
        assert!(
            gathering
                .listed_within("canonical", full.document.len() - 1)
                .is_err()
        );
    }
}
