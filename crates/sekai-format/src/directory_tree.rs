//! Directory trees, specified in `docs/spec/directory-tree.md`: every directory and file of a
//! publication of a view in one document, for readers that browse a whole view, such as file
//! browsers.

use std::collections::HashMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::paths::PathFile;

/// The version of the directory tree format.
pub const VERSION: u32 = 1;

/// The extension of a directory tree in the store: the document is stored gzip-compressed.
pub const EXTENSION: &str = "json.gz";

/// The media type of a directory tree in the store.
pub const MEDIA_TYPE: &str = "application/gzip";

/// A file of a view, as a directory tree has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFile {
    /// The path in the view, without a leading `/`.
    pub path: String,
    /// The size in bytes.
    pub size: u64,
    /// The media type.
    pub media_type: String,
}

impl From<&PathFile> for TreeFile {
    fn from(file: &PathFile) -> Self {
        Self {
            path: file.path.clone(),
            size: file.size,
            media_type: file.media_type.clone(),
        }
    }
}

/// The directory tree of a publication of a view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryTree {
    /// Always `"directory-tree"`.
    pub format: String,
    /// The version of the format.
    pub version: u32,
    /// The view.
    pub view: String,
    /// The name of the published version.
    pub published_version: String,
    /// The revision of the publication.
    pub revision: String,
    /// The number of files.
    pub files: u64,
    /// The media types of the files, in order of first use.
    pub media_types: Vec<String>,
    /// The root directory, whose name is empty.
    pub root: Directory,
}

/// A directory: what is right under it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Directory {
    /// The name, which is empty for the root.
    pub name: String,
    /// The subdirectories, in byte order of their names. Left out when there are none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub directories: Vec<Directory>,
    /// The files. Left out when there are none.
    #[serde(default, skip_serializing_if = "Files::is_empty")]
    pub files: Files,
}

/// The files right under a directory, one column per field, in byte order of their names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Files {
    /// The names.
    pub names: Vec<String>,
    /// The sizes in bytes.
    pub sizes: Vec<u64>,
    /// The position of each media type in [`DirectoryTree::media_types`].
    pub media_types: Vec<usize>,
}

impl Files {
    fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}

/// A directory tree that does not follow the format, or files that cannot make one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectoryTreeError(String);

impl fmt::Display for DirectoryTreeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "directory tree: {}", self.0)
    }
}

impl std::error::Error for DirectoryTreeError {}

fn invalid(message: impl Into<String>) -> DirectoryTreeError {
    DirectoryTreeError(message.into())
}

impl DirectoryTree {
    /// The tree of `files`, which are in strictly increasing byte order of their paths, as the
    /// publication of `view` named `published_version` with `revision` has them.
    pub fn new(
        view: &str,
        published_version: &str,
        revision: &str,
        files: impl IntoIterator<Item = TreeFile>,
    ) -> Result<Self, DirectoryTreeError> {
        let mut builder = Builder::default();
        for file in files {
            builder.add(file)?;
        }
        Ok(Self {
            format: "directory-tree".to_owned(),
            version: VERSION,
            view: view.to_owned(),
            published_version: published_version.to_owned(),
            revision: revision.to_owned(),
            files: builder.count,
            media_types: builder.media_types,
            root: builder.open.finish(),
        })
    }

    /// Reads a tree as JSON, checking its format.
    pub fn from_json(bytes: &[u8]) -> Result<Self, DirectoryTreeError> {
        let tree: Self = serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
        if tree.format != "directory-tree" || tree.version != VERSION {
            return Err(invalid(format!(
                "format {} version {} is not directory-tree version {VERSION}",
                tree.format, tree.version
            )));
        }
        Ok(tree)
    }

    /// The files of the tree, in byte order of their paths. Fails when the tree does not hold
    /// what the format says it does.
    pub fn files(&self) -> Result<Vec<TreeFile>, DirectoryTreeError> {
        if !self.root.name.is_empty() {
            return Err(invalid("the root has a name"));
        }
        let mut files = Vec::new();
        self.root.collect("", &self.media_types, &mut files)?;
        files.sort_unstable_by(|a, b| a.path.cmp(&b.path));
        if let Some(pair) = files.windows(2).find(|pair| pair[0].path == pair[1].path) {
            return Err(invalid(format!("{} is listed twice", pair[0].path)));
        }
        if files.len() as u64 != self.files {
            return Err(invalid(format!(
                "it counts {} files and lists {}",
                self.files,
                files.len()
            )));
        }
        Ok(files)
    }
}

impl Directory {
    fn named(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..Self::default()
        }
    }

    /// Adds the files under the directory, whose path is `prefix`, to `files`.
    fn collect(
        &self,
        prefix: &str,
        media_types: &[String],
        files: &mut Vec<TreeFile>,
    ) -> Result<(), DirectoryTreeError> {
        let Files {
            names,
            sizes,
            media_types: types,
        } = &self.files;
        if sizes.len() != names.len() || types.len() != names.len() {
            return Err(invalid(format!(
                "the columns of {prefix:?} differ in length"
            )));
        }
        for ((name, &size), &media_type) in names.iter().zip(sizes).zip(types) {
            check_name(name)?;
            let media_type = media_types
                .get(media_type)
                .ok_or_else(|| invalid(format!("{prefix}{name} has no media type {media_type}")))?;
            files.push(TreeFile {
                path: format!("{prefix}{name}"),
                size,
                media_type: media_type.clone(),
            });
        }
        for directory in &self.directories {
            check_name(&directory.name)?;
            let prefix = format!("{prefix}{}/", directory.name);
            directory.collect(&prefix, media_types, files)?;
        }
        Ok(())
    }
}

/// Fails unless `name` can be a segment of a path: not empty and without `/`.
fn check_name(name: &str) -> Result<(), DirectoryTreeError> {
    if name.is_empty() || name.contains('/') {
        return Err(invalid(format!("{name:?} is not a name")));
    }
    Ok(())
}

/// Builds a tree from files in path order. The files under a directory are contiguous in that
/// order, so a directory is finished when the first path outside it arrives.
#[derive(Default)]
struct Builder {
    media_types: Vec<String>,
    /// The position of each media type in `media_types`.
    positions: HashMap<String, usize>,
    open: OpenDirectories,
    count: u64,
    last: Option<String>,
}

impl Builder {
    fn add(&mut self, file: TreeFile) -> Result<(), DirectoryTreeError> {
        if self.last.as_ref().is_some_and(|last| *last >= file.path) {
            return Err(invalid(format!("{} is out of path order", file.path)));
        }
        let (directory, name) = file.path.rsplit_once('/').unwrap_or(("", &file.path));
        let segments: Vec<&str> = match directory {
            "" => Vec::new(),
            directory => directory.split('/').collect(),
        };
        check_name(name)?;
        for segment in &segments {
            check_name(segment)?;
        }
        let media_type = self.media_type(&file.media_type);
        let files = &mut self.open.enter(&segments).files;
        files.names.push(name.to_owned());
        files.sizes.push(file.size);
        files.media_types.push(media_type);
        self.count += 1;
        self.last = Some(file.path);
        Ok(())
    }

    /// The position of `media_type`, which is added when it is new.
    fn media_type(&mut self, media_type: &str) -> usize {
        if let Some(&position) = self.positions.get(media_type) {
            return position;
        }
        let position = self.media_types.len();
        self.media_types.push(media_type.to_owned());
        self.positions.insert(media_type.to_owned(), position);
        position
    }
}

/// The root and the directories from it down to the one that the last file was added to.
#[derive(Default)]
struct OpenDirectories {
    root: Directory,
    /// The open directories below the root, each the parent of the next.
    below: Vec<Directory>,
}

impl OpenDirectories {
    /// The directory at `segments` below the root: the directories that are open and not on the
    /// way to it are finished, and the missing ones opened.
    fn enter(&mut self, segments: &[&str]) -> &mut Directory {
        let shared = self
            .below
            .iter()
            .zip(segments)
            .take_while(|(directory, segment)| directory.name == **segment)
            .count();
        self.close_to(shared);
        let opened = segments[shared..]
            .iter()
            .map(|segment| Directory::named(segment));
        self.below.extend(opened);
        self.innermost()
    }

    fn innermost(&mut self) -> &mut Directory {
        self.below.last_mut().unwrap_or(&mut self.root)
    }

    /// Finishes the open directories below the first `depth`, each into its parent.
    fn close_to(&mut self, depth: usize) {
        let mut finished = None;
        for mut directory in self.below.split_off(depth).into_iter().rev() {
            directory.directories.extend(finished);
            directory.sort();
            finished = Some(directory);
        }
        self.innermost().directories.extend(finished);
    }

    /// The root, with every directory finished.
    fn finish(mut self) -> Directory {
        self.close_to(0);
        self.root.sort();
        self.root
    }
}

impl Directory {
    /// Puts the subdirectories in byte order of their names. They are finished in the order of
    /// their paths, where `a-b/` comes before `a/`.
    fn sort(&mut self) {
        self.directories.sort_by(|a, b| a.name.cmp(&b.name));
    }
}
