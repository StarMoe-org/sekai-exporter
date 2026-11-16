//! The `tree` layout: a path tree at fixed paths.
//!
//! A tree cannot be switched atomically, so a publication writes every new file, then the listing,
//! then deletes the files that the previous publication stopped using. A file leaves the tree one
//! publication after it leaves the listing, so readers of the previous version can finish.
//! Running a publication again after an interruption converges to the same tree.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;

use sekai_catalog::LocalStore;
use sekai_format::publication::{TreeEntry, TreeListing, VERSION};
use serde::{Deserialize, Serialize};

use crate::parallel;
use crate::plan::{Plan, PublishError, hex};

const LISTING: &str = "_listing.json";
const STATE: &str = "_state.json";

/// What publications have left to delete.
#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    pending_delete: Vec<String>,
    /// Files that the current publication is retiring, kept on disk until the next one finishes
    /// even if this run is interrupted after writing `_listing.json`.
    #[serde(default)]
    retiring: Vec<String>,
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> io::Result<Option<T>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(io::Error::other)?;
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)
}

/// What a tree publication did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TreeReport {
    /// The files written.
    pub written: usize,
    /// The files that were already up to date.
    pub unchanged: usize,
    /// The files deleted.
    pub deleted: usize,
}

/// Publishes `plan` as a tree at `root`, writing files on up to `threads` threads. The files of
/// an asset that could not be produced keep their previous bytes until a conversion succeeds.
pub fn publish(
    store: &LocalStore,
    root: &Path,
    plan: &Plan,
    threads: usize,
) -> Result<TreeReport, PublishError> {
    let wrap = PublishError::wrap("files");
    std::fs::create_dir_all(root).map_err(&wrap)?;
    let previous: Option<TreeListing> = read_json(&root.join(LISTING)).map_err(&wrap)?;
    let previous = previous.map_or_else(Vec::new, |l| l.entries);
    let state: State = read_json(&root.join(STATE))
        .map_err(&wrap)?
        .unwrap_or_default();

    let failed: BTreeSet<&str> = plan.failures.iter().map(|f| f.name.as_str()).collect();
    let mut entries: BTreeMap<String, TreeEntry> = BTreeMap::new();
    for entry in previous
        .iter()
        .filter(|e| failed.contains(e.asset.as_str()))
    {
        entries.insert(entry.path.clone(), entry.clone());
    }
    for file in &plan.files {
        // The document of a failed asset is not a reason to replace what readers have.
        if failed.contains(file.asset.as_str()) && entries.contains_key(&file.path) {
            continue;
        }
        entries.insert(
            file.path.clone(),
            TreeEntry {
                path: file.path.clone(),
                sha256: hex(&file.key.sha256),
                asset: file.asset.clone(),
            },
        );
    }

    let to_delete: BTreeSet<String> = state
        .pending_delete
        .into_iter()
        .filter(|path| !entries.contains_key(path))
        .collect();
    let retiring: Vec<String> = state
        .retiring
        .into_iter()
        .chain(previous.iter().map(|e| e.path.clone()))
        .filter(|path| !entries.contains_key(path) && !to_delete.contains(path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    write_json(
        &root.join(STATE),
        &State {
            pending_delete: to_delete.iter().cloned().collect(),
            retiring: retiring.clone(),
        },
    )
    .map_err(&wrap)?;

    let known: BTreeMap<&str, &str> = previous
        .iter()
        .map(|e| (e.path.as_str(), e.sha256.as_str()))
        .collect();
    let mut report = TreeReport::default();
    let mut writes = Vec::new();
    for file in &plan.files {
        let Some(entry) = entries.get(&file.path) else {
            continue;
        };
        if entry.sha256 != hex(&file.key.sha256) {
            continue;
        }
        let destination = root.join(&file.path);
        if known.get(file.path.as_str()) == Some(&entry.sha256.as_str()) && destination.is_file() {
            report.unchanged += 1;
            continue;
        }
        clear_conflicting_paths(root, &file.path, &entries).map_err(&wrap)?;
        writes.push((&file.key, destination));
    }
    parallel::map(&writes, threads, |(key, destination)| {
        store.materialize(key, destination)
    })
    .into_iter()
    .collect::<io::Result<()>>()
    .map_err(&wrap)?;
    report.written = writes.len();

    let listing = TreeListing {
        format: "tree-listing".to_owned(),
        version: VERSION,
        published_version: plan.version.clone(),
        entries: entries.values().cloned().collect(),
    };
    write_json(&root.join(LISTING), &listing).map_err(PublishError::wrap("listing"))?;

    let wrap = PublishError::wrap("delete");
    for path in &to_delete {
        let full = root.join(path);
        match std::fs::remove_file(&full) {
            Ok(()) => {
                report.deleted += 1;
                prune_empty_parents(root, &full);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(wrap(e)),
        }
    }
    write_json(
        &root.join(STATE),
        &State {
            pending_delete: retiring,
            retiring: Vec::new(),
        },
    )
    .map_err(&wrap)?;
    Ok(report)
}

/// Removes a retired file on an ancestor of `rel_path`, or an empty retired directory at
/// `rel_path` itself, when it would prevent writing `rel_path`.
fn clear_conflicting_paths(
    root: &Path,
    rel_path: &str,
    entries: &BTreeMap<String, TreeEntry>,
) -> io::Result<()> {
    let mut prefix = String::new();
    if let Some((parent, _)) = rel_path.rsplit_once('/') {
        for segment in parent.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            if !entries.contains_key(&prefix) {
                let candidate = root.join(&prefix);
                if candidate.is_file() {
                    std::fs::remove_file(candidate)?;
                }
            }
        }
    }
    let target = root.join(rel_path);
    if target.is_dir() {
        let prefix = format!("{rel_path}/");
        let live = entries
            .range(prefix.clone()..)
            .next()
            .is_some_and(|(k, _)| k.starts_with(&prefix));
        if !live {
            std::fs::remove_dir_all(target)?;
        }
    }
    Ok(())
}

fn prune_empty_parents(root: &Path, removed: &Path) {
    let mut current = removed.parent();
    while let Some(dir) = current {
        if dir == root {
            break;
        }
        if std::fs::remove_dir(dir).is_err() {
            break;
        }
        current = dir.parent();
    }
}
