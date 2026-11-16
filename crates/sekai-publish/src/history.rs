//! The version history of a view and the changes of each version, written with every
//! publication, as `docs/spec/version-history.md` specifies them.
//!
//! The history of a publication is that of the previous publication with one entry for its
//! version: a new entry first, or, when the version is published again under its name, in place
//! of the newest one. The changes of a new version are what the change applied to the previous
//! publication did at its paths. The changes of a version published again stay those since the
//! version before it: the ones the previous publication recorded, with each path that the change
//! touched looked up again in the version before.

use sekai_catalog::Store;
use sekai_format::paths::{PathFile, PathIndex};
use sekai_format::publication::{ObjectReference, Pointer};
use sekai_format::version_history::{
    self, FileChange, HistoryEntry, VersionChanges, VersionHistory,
};
use serde::Serialize;

use crate::patch::{Lookup, Written, read_gzip, read_path_index};
use crate::plan::PublishError;

/// The pointers of the published versions of a view.
pub trait Versions {
    /// The pointer of the published version `version`, or `None` when it is not published.
    fn pointer(&self, version: &str) -> Result<Option<Pointer>, PublishError>;
}

/// No published versions, for a view that has no publication yet.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoVersions;

impl Versions for NoVersions {
    fn pointer(&self, _version: &str) -> Result<Option<Pointer>, PublishError> {
        Ok(None)
    }
}

/// The publication that the history of a view continues from, as its pointer names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Last {
    /// The name of its published version.
    pub published_version: String,
    /// Its revision.
    pub revision: String,
    /// Its number of files, when its pointer names a path index.
    pub files: Option<u64>,
    /// Its version history.
    pub history: Option<ObjectReference>,
    /// The changes of its version.
    pub changes: Option<ObjectReference>,
}

impl Last {
    /// The publication that `pointer` names, whose path index is `paths`.
    pub fn of(pointer: &Pointer, paths: Option<&PathIndex>) -> Self {
        Self {
            published_version: pointer.published_version.clone(),
            revision: pointer.revision.clone(),
            files: paths.map(|paths| paths.files),
            history: pointer.history.clone(),
            changes: pointer.changes.clone(),
        }
    }
}

/// The files at a path before and after a change, which hold other bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PathDiff {
    pub(crate) path: String,
    pub(crate) before: Option<PathFile>,
    pub(crate) after: Option<PathFile>,
}

/// The publication that a history gains its entry for.
pub(crate) struct Made<'a> {
    pub(crate) view: &'a str,
    pub(crate) version: &'a str,
    pub(crate) revision: &'a str,
    pub(crate) files: u64,
    /// When it was made, as RFC 3339.
    pub(crate) published_at: Option<String>,
}

/// The documents a pointer names the history and the changes by.
pub(crate) struct Recorded {
    pub(crate) history: ObjectReference,
    pub(crate) changes: Option<ObjectReference>,
}

/// Writes the history of `made` and the changes of its version, which follows `last`. `diffs`
/// are what the change from `last` did at the paths of the view, when it was applied to that
/// publication. The documents of earlier publications are read from `store`, and their pointers
/// from `versions`.
pub(crate) fn record(
    store: &dyn Store,
    written: &mut Written<'_>,
    made: &Made<'_>,
    last: Option<&Last>,
    diffs: Option<Vec<PathDiff>>,
    versions: &dyn Versions,
) -> Result<Recorded, PublishError> {
    let Some(last) = last else {
        return write(written, made, [], None);
    };
    let again = last.published_version == made.version;
    if again
        && last.revision == made.revision
        && let Some(history) = &last.history
    {
        // The same publication again: its history and changes stay as they were.
        return Ok(Recorded {
            history: history.clone(),
            changes: last.changes.clone(),
        });
    }
    let changes = match diffs {
        None => None,
        Some(diffs) if again => recompose(store, made.version, last, diffs, versions)?,
        Some(diffs) => {
            let changes = diffs
                .iter()
                .filter_map(|diff| FileChange::between(diff.before.as_ref(), diff.after.as_ref()));
            let from = &last.published_version;
            Some(VersionChanges::new(made.view, from, made.version, changes))
        }
    };
    // A version published again takes the place of its own entry, the newest.
    let earlier = entries(store, last)?.into_iter().skip(usize::from(again));
    write(written, made, earlier, changes)
}

/// The entries of the history up to `last`. A publication without a history starts one with an
/// entry of its own, when its number of files is known.
fn entries(store: &dyn Store, last: &Last) -> Result<Vec<HistoryEntry>, PublishError> {
    if let Some(history) = &last.history {
        let json = read_gzip(store, history, version_history::EXTENSION)?;
        let history = VersionHistory::from_json(&json).map_err(previous)?;
        return Ok(history.entries);
    }
    let entry = last.files.map(|files| HistoryEntry {
        published_version: last.published_version.clone(),
        revision: last.revision.clone(),
        published_at: None,
        files,
        counts: None,
    });
    Ok(entry.into_iter().collect())
}

/// The changes of `version`, published again after `last` with `diffs`: those that
/// `last` recorded since the version before, with each path of `diffs` compared again with that
/// version. None when `last` recorded no changes, or the version before has no path index.
fn recompose(
    store: &dyn Store,
    version: &str,
    last: &Last,
    diffs: Vec<PathDiff>,
    versions: &dyn Versions,
) -> Result<Option<VersionChanges>, PublishError> {
    let Some(changes) = &last.changes else {
        return Ok(None);
    };
    let json = read_gzip(store, changes, version_history::EXTENSION)?;
    let recorded = VersionChanges::from_json(&json).map_err(previous)?;
    let paths = versions.pointer(&recorded.from)?.and_then(|p| p.paths);
    let Some(paths) = paths else {
        return Ok(None);
    };
    let index = read_path_index(store, &paths)?;
    let before = Lookup::new(store, Some(&index));
    let revised = diffs
        .into_iter()
        .map(|diff| {
            let was = before.file(&diff.path)?;
            let change = FileChange::between(was.as_ref(), diff.after.as_ref());
            Ok((diff.path, change))
        })
        .collect::<Result<Vec<_>, PublishError>>()?;
    Ok(Some(recorded.revise(version, revised)))
}

/// Writes `changes`, and the history of `made`: its entry, then `earlier`.
fn write(
    written: &mut Written<'_>,
    made: &Made<'_>,
    earlier: impl IntoIterator<Item = HistoryEntry>,
    changes: Option<VersionChanges>,
) -> Result<Recorded, PublishError> {
    let counts = changes.as_ref().map(VersionChanges::counts);
    let changes = changes.map(|c| put(written, &c)).transpose()?;
    let entry = HistoryEntry {
        published_version: made.version.to_owned(),
        revision: made.revision.to_owned(),
        published_at: made.published_at.clone(),
        files: made.files,
        counts,
    };
    let entries = std::iter::once(entry).chain(earlier).collect();
    let history = put(written, &VersionHistory::new(made.view, entries))?;
    Ok(Recorded { history, changes })
}

/// Stores `document` as a document of the version history format.
fn put(
    written: &mut Written<'_>,
    document: &impl Serialize,
) -> Result<ObjectReference, PublishError> {
    written.put_gzip(
        document,
        version_history::EXTENSION,
        version_history::MEDIA_TYPE,
    )
}

fn previous(error: impl std::fmt::Display) -> PublishError {
    PublishError::new("previous", error)
}
