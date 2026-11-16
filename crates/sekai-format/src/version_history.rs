//! Version histories and version changes, specified in `docs/spec/version-history.md`: the
//! published versions of a view, newest first, and what each of them changed at the paths of
//! the view.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::paths::PathFile;

/// The version of the version history and version changes formats.
pub const VERSION: u32 = 1;

/// The extension of both documents in the store: they are stored gzip-compressed.
pub const EXTENSION: &str = "json.gz";

/// The media type of both documents in the store.
pub const MEDIA_TYPE: &str = "application/gzip";

/// The published versions of a view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionHistory {
    /// Always `"version-history"`.
    pub format: String,
    /// The version of the format.
    pub version: u32,
    /// The view.
    pub view: String,
    /// One entry for each published version, newest first.
    pub entries: Vec<HistoryEntry>,
}

/// A published version, as its last publication made it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// The name of the published version.
    pub published_version: String,
    /// The revision of its publication.
    pub revision: String,
    /// When the publication was made, in UTC as RFC 3339, when that is known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    /// The number of files.
    pub files: u64,
    /// How many files the version added, removed and changed since the version before it, when
    /// that is known.
    #[serde(flatten)]
    pub counts: Option<ChangeCounts>,
}

/// How many files a version added, removed and changed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeCounts {
    /// The files at paths the version before had none at.
    pub added: u64,
    /// The files at paths the version no longer has a file at.
    pub removed: u64,
    /// The files whose bytes differ from those at the same path in the version before.
    pub changed: u64,
}

/// What a published version changed at the paths of a view, since the version before it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionChanges {
    /// Always `"version-changes"`.
    pub format: String,
    /// The version of the format.
    pub version: u32,
    /// The view.
    pub view: String,
    /// The published version before.
    pub from: String,
    /// The published version whose changes these are.
    pub to: String,
    /// The files added, in path order.
    pub added: Vec<AddedFile>,
    /// The files removed, in path order.
    pub removed: Vec<RemovedFile>,
    /// The files changed, in path order.
    pub changed: Vec<ChangedFile>,
}

/// A file at a path the version before had none at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AddedFile {
    /// The path in the view.
    pub path: String,
    /// The size in bytes.
    pub size: u64,
    /// The media type.
    pub media_type: String,
}

/// A file of the version before at a path the version has no file at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedFile {
    /// The path in the view.
    pub path: String,
    /// The size in bytes in the version before.
    pub size: u64,
}

/// A file whose bytes differ from those at the same path in the version before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    /// The path in the view.
    pub path: String,
    /// The size in bytes.
    pub size: u64,
    /// The size in bytes in the version before.
    pub previous_size: u64,
}

/// What became of the file at one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileChange {
    /// A file was put where there was none.
    Added(AddedFile),
    /// The file was taken away.
    Removed(RemovedFile),
    /// The file has other bytes.
    Changed(ChangedFile),
}

impl FileChange {
    /// What became of a path that held `before` and now holds `after`: nothing when both hold
    /// the same bytes or neither holds a file.
    pub fn between(before: Option<&PathFile>, after: Option<&PathFile>) -> Option<Self> {
        match (before, after) {
            (None, Some(after)) => Some(Self::Added(AddedFile {
                path: after.path.clone(),
                size: after.size,
                media_type: after.media_type.clone(),
            })),
            (Some(before), None) => Some(Self::Removed(RemovedFile {
                path: before.path.clone(),
                size: before.size,
            })),
            (Some(before), Some(after)) if before.sha256 != after.sha256 => {
                Some(Self::Changed(ChangedFile {
                    path: after.path.clone(),
                    size: after.size,
                    previous_size: before.size,
                }))
            }
            _ => None,
        }
    }

    /// The path the change is at.
    pub fn path(&self) -> &str {
        match self {
            Self::Added(file) => &file.path,
            Self::Removed(file) => &file.path,
            Self::Changed(file) => &file.path,
        }
    }
}

/// A document that does not follow its format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionHistoryError(String);

impl fmt::Display for VersionHistoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "version history: {}", self.0)
    }
}

impl std::error::Error for VersionHistoryError {}

/// Reads a document of the format `format` as JSON, checking its format and version.
fn from_json<T: DeserializeOwned>(
    bytes: &[u8],
    format: &str,
    header: impl Fn(&T) -> (&str, u32),
) -> Result<T, VersionHistoryError> {
    let document: T =
        serde_json::from_slice(bytes).map_err(|e| VersionHistoryError(e.to_string()))?;
    let (found, version) = header(&document);
    if found != format || version != VERSION {
        return Err(VersionHistoryError(format!(
            "format {found} version {version} is not {format} version {VERSION}"
        )));
    }
    Ok(document)
}

impl VersionHistory {
    /// The history of `view` with `entries`, newest first.
    pub fn new(view: &str, entries: Vec<HistoryEntry>) -> Self {
        Self {
            format: "version-history".to_owned(),
            version: VERSION,
            view: view.to_owned(),
            entries,
        }
    }

    /// Reads a history as JSON, checking its format.
    pub fn from_json(bytes: &[u8]) -> Result<Self, VersionHistoryError> {
        from_json(bytes, "version-history", |history: &Self| {
            (&history.format, history.version)
        })
    }
}

impl VersionChanges {
    /// The changes of `view` from the version `from` to `to`, made of `changes` in path order.
    pub fn new(
        view: &str,
        from: &str,
        to: &str,
        changes: impl IntoIterator<Item = FileChange>,
    ) -> Self {
        let mut document = Self {
            format: "version-changes".to_owned(),
            version: VERSION,
            view: view.to_owned(),
            from: from.to_owned(),
            to: to.to_owned(),
            added: Vec::new(),
            removed: Vec::new(),
            changed: Vec::new(),
        };
        for change in changes {
            match change {
                FileChange::Added(file) => document.added.push(file),
                FileChange::Removed(file) => document.removed.push(file),
                FileChange::Changed(file) => document.changed.push(file),
            }
        }
        document
    }

    /// Reads changes as JSON, checking their format.
    pub fn from_json(bytes: &[u8]) -> Result<Self, VersionHistoryError> {
        from_json(bytes, "version-changes", |changes: &Self| {
            (&changes.format, changes.version)
        })
    }

    /// How many files the changes add, remove and change.
    pub fn counts(&self) -> ChangeCounts {
        ChangeCounts {
            added: self.added.len() as u64,
            removed: self.removed.len() as u64,
            changed: self.changed.len() as u64,
        }
    }

    /// The changes since the same version for the version `to`: these, with the change at each
    /// path of `revised` replaced by the one given with it, or taken out where none is given.
    pub fn revise(
        self,
        to: &str,
        revised: impl IntoIterator<Item = (String, Option<FileChange>)>,
    ) -> Self {
        let added = self.added.into_iter().map(FileChange::Added);
        let removed = self.removed.into_iter().map(FileChange::Removed);
        let changed = self.changed.into_iter().map(FileChange::Changed);
        let mut by_path: BTreeMap<String, FileChange> = added
            .chain(removed)
            .chain(changed)
            .map(|change| (change.path().to_owned(), change))
            .collect();
        for (path, change) in revised {
            match change {
                Some(change) => by_path.insert(path, change),
                None => by_path.remove(&path),
            };
        }
        Self::new(&self.view, &self.from, to, by_path.into_values())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, byte: u8, size: u64) -> PathFile {
        PathFile {
            path: path.to_owned(),
            sha256: [byte; 32],
            extension: "png".to_owned(),
            size,
            media_type: "image/png".to_owned(),
        }
    }

    #[test]
    fn a_path_changes_when_its_bytes_do() {
        let (a, b) = (file("x.png", 1, 3), file("x.png", 2, 5));
        let same_bytes = file("x.png", 1, 3);
        assert_eq!(FileChange::between(Some(&a), Some(&same_bytes)), None);
        assert_eq!(FileChange::between(None, None), None);
        let changed = FileChange::between(Some(&a), Some(&b)).unwrap();
        assert_eq!(
            changed,
            FileChange::Changed(ChangedFile {
                path: "x.png".to_owned(),
                size: 5,
                previous_size: 3,
            })
        );
        let added = FileChange::between(None, Some(&b)).unwrap();
        let removed = FileChange::between(Some(&a), None).unwrap();
        let changes = VersionChanges::new("storage", "1", "2", [added, removed, changed]);
        assert_eq!(
            changes.counts(),
            ChangeCounts {
                added: 1,
                removed: 1,
                changed: 1
            }
        );
    }

    #[test]
    fn revised_changes_replace_the_changes_at_the_paths_given() {
        let (a, b, c, d) = (
            file("a.png", 1, 1),
            file("b.png", 2, 2),
            file("c.png", 3, 3),
            file("d.png", 4, 4),
        );
        let c2 = file("c.png", 5, 5);
        let rows = [
            FileChange::between(None, Some(&a)),
            FileChange::between(Some(&b), None),
            FileChange::between(Some(&c), Some(&c2)),
        ];
        let changes = VersionChanges::new("storage", "1", "2", rows.into_iter().flatten());
        let added_d = FileChange::between(None, Some(&d));
        let revised = changes.revise(
            "2",
            [("a.png".to_owned(), None), ("d.png".to_owned(), added_d)],
        );
        assert_eq!((revised.from.as_str(), revised.to.as_str()), ("1", "2"));
        let added = AddedFile {
            path: "d.png".to_owned(),
            size: 4,
            media_type: "image/png".to_owned(),
        };
        assert_eq!(revised.added, [added]);
        let removed = RemovedFile {
            path: "b.png".to_owned(),
            size: 2,
        };
        assert_eq!(revised.removed, [removed]);
        let changed = ChangedFile {
            path: "c.png".to_owned(),
            size: 5,
            previous_size: 3,
        };
        assert_eq!(revised.changed, [changed]);
    }

    #[test]
    fn an_entry_without_counts_leaves_them_out() {
        let entry = HistoryEntry {
            published_version: "1.0".to_owned(),
            revision: "r".to_owned(),
            published_at: None,
            files: 4,
            counts: None,
        };
        let history = VersionHistory::new("canonical", vec![entry.clone()]);
        let json = serde_json::to_string(&history).unwrap();
        assert_eq!(
            json,
            r#"{"format":"version-history","version":1,"view":"canonical","entries":[{"published_version":"1.0","revision":"r","files":4}]}"#
        );
        assert_eq!(VersionHistory::from_json(json.as_bytes()).unwrap(), history);

        let counted = HistoryEntry {
            published_at: Some("2025-01-02T03:04:05Z".to_owned()),
            counts: Some(ChangeCounts {
                added: 1,
                removed: 0,
                changed: 2,
            }),
            ..entry
        };
        let json = serde_json::to_string(&counted).unwrap();
        assert_eq!(
            json,
            r#"{"published_version":"1.0","revision":"r","published_at":"2025-01-02T03:04:05Z","files":4,"added":1,"removed":0,"changed":2}"#
        );
        assert_eq!(
            serde_json::from_str::<HistoryEntry>(&json).unwrap(),
            counted
        );
    }

    #[test]
    fn a_document_of_another_format_is_refused() {
        let changes = VersionChanges::new("storage", "1", "2", []);
        let json = serde_json::to_vec(&changes).unwrap();
        assert_eq!(VersionChanges::from_json(&json).unwrap(), changes);
        assert!(VersionHistory::from_json(&json).is_err());
    }
}
