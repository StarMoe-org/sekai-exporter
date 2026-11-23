//! Directory listings: the subdirectories and files right under a directory of a view, gathered
//! from the chunks of its path index.

use std::sync::Arc;

use sekai_format::paths::{Chunk, ChunkFile, PathIndex};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The version of the directory listing format.
pub(crate) const VERSION: u32 = 1;

/// A directory of a view, listed from its path index.
///
/// The paths under a directory are one range of the index, in byte order. The listing looks at
/// that range from its start and, at the first path of a subdirectory, goes on past every path
/// under it, so it reads only the chunks in which a file or a subdirectory of the directory
/// starts.
#[derive(Debug)]
pub(crate) struct Listing {
    /// The directory: empty for the root, and otherwise ending with `/`.
    directory: String,
    /// The first path not looked at yet.
    next: String,
    /// The subdirectories found so far, in path order.
    directories: Vec<String>,
    /// The files found so far, in path order.
    files: Vec<ListedFile>,
    /// Whether every path under the directory has been looked at.
    complete: bool,
}

/// A listing as it is sent, and its `ETag`, kept between requests.
#[derive(Debug)]
pub(crate) struct Listed {
    /// The JSON document.
    pub(crate) document: Arc<[u8]>,
    /// `"<sha256>"` of the document.
    pub(crate) etag: String,
}

impl Listed {
    pub(crate) fn new(document: Vec<u8>) -> Self {
        let etag = format!("\"{}\"", hex(&Sha256::digest(&document)));
        Self {
            document: document.into(),
            etag,
        }
    }

    /// The memory the listing takes.
    pub(crate) fn weight(&self) -> usize {
        size_of::<Self>() + self.document.len() + self.etag.capacity()
    }
}

/// The key under which the listing of `directory` in the path index with the root `root` is
/// kept: a hash of both, which no object of an index is named by.
pub(crate) fn key(root: &[u8; 32], directory: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"directory-listing\n");
    hasher.update(root);
    hasher.update(directory.as_bytes());
    hasher.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A file right under the directory.
#[derive(Debug, Serialize)]
struct ListedFile {
    name: String,
    sha256: String,
    extension: String,
    size: u64,
    media_type: String,
}

impl ListedFile {
    fn new(name: &str, file: ChunkFile<'_>) -> Self {
        Self {
            name: name.to_owned(),
            sha256: hex(&file.sha256),
            extension: file.extension.to_owned(),
            size: file.size,
            media_type: file.media_type.to_owned(),
        }
    }
}

/// A listing as it is sent.
#[derive(Serialize)]
struct Document<'a> {
    format: &'static str,
    version: u32,
    view: &'a str,
    published_version: &'a str,
    revision: &'a str,
    path: &'a str,
    directories: &'a [String],
    files: &'a [ListedFile],
}

impl Listing {
    /// The listing of `directory`, with nothing looked at yet.
    pub(crate) fn new(directory: &str) -> Self {
        Self {
            directory: directory.to_owned(),
            next: directory.to_owned(),
            directories: Vec::new(),
            files: Vec::new(),
            complete: false,
        }
    }

    /// The directory: empty for the root, and otherwise ending with `/`.
    pub(crate) fn directory(&self) -> &str {
        &self.directory
    }

    /// The position of the chunk of `index` that the listing reads next, or `None` once it has
    /// looked at every path under the directory.
    pub(crate) fn wanted(&self, index: &PathIndex) -> Option<usize> {
        if self.complete || index.chunks.is_empty() {
            return None;
        }
        Some(index.chunk_of(&self.next).unwrap_or(0))
    }

    /// Looks at the paths of `chunk`, the chunk of `index` that [`Listing::wanted`] names.
    pub(crate) fn gather(&mut self, index: &PathIndex, chunk: &Chunk) {
        let at = index.chunk_of(&self.next).unwrap_or(0);
        let mut files = chunk.files_from(&self.next);
        while let Some(file) = files.next() {
            let Some(rest) = file.path.strip_prefix(self.directory.as_str()) else {
                self.complete = true;
                return;
            };
            match rest.split_once('/') {
                Some((child, _)) => {
                    // `0` is the character after `/`, so every path under the subdirectory comes
                    // before this one, and every path after them comes after it.
                    self.next = format!("{}{child}0", self.directory);
                    self.directories.push(child.to_owned());
                    files = chunk.files_from(&self.next);
                }
                None => self.files.push(ListedFile::new(rest, file)),
            }
        }
        // The rest of the range is in the chunks after this one, if there are any.
        match index.chunks.get(at + 1) {
            Some(following) if following.first > self.next => {
                self.next.clone_from(&following.first)
            }
            Some(_) => {}
            None => self.complete = true,
        }
    }

    /// Whether the view has the directory: whether something is under it, or it is the root,
    /// which a published view always has.
    pub(crate) fn is_found(&self) -> bool {
        self.directory.is_empty() || !(self.directories.is_empty() && self.files.is_empty())
    }

    /// The listing as it is sent, describing the publication of `index`.
    pub(crate) fn listed(&self, index: &PathIndex) -> Result<Listed, String> {
        let document = Document {
            format: "directory-listing",
            version: VERSION,
            view: &index.view,
            published_version: &index.published_version,
            revision: &index.revision,
            path: &self.directory,
            directories: &self.directories,
            files: &self.files,
        };
        let document = serde_json::to_vec(&document).map_err(|e| e.to_string())?;
        Ok(Listed::new(document))
    }
}
