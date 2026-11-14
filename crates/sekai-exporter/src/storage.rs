//! The `storage` view of a region.
//!
//! Every file of the view lies directly in the directory of the bundle that holds the root object
//! of its asset, and that bundle names the unit of the asset. A run therefore changes the view in
//! the directories of the units it touches: each is made again from the assets its unit owns, and
//! compared with what the previous publication of the view has there.

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use sekai_catalog::store::parse_hex;
use sekai_catalog::{Derivations, Derived, Key, LocalStore, Objects, Store};
use sekai_convert::Asset;
use sekai_format::AssetDocument;
use sekai_moesekai::storage_view::{self, Transform, ViewFile};
use sekai_pipeline::isolate::{PANICKED, isolated};
use sekai_pipeline::metrics::Recorder;
use sekai_pipeline::parallel;
use sekai_publish::patch::{Change, Lookup};
use sekai_publish::{Binding, File};
use sha2::{Digest, Sha256};

use crate::monitor::Monitor;

/// One file to make: its asset, its place in the view and the member it comes from.
struct Job {
    asset: String,
    file: ViewFile,
    source: Key,
    size: u64,
    media_type: String,
    role: String,
}

fn fingerprint(transform: Transform, source: &Key) -> [u8; 32] {
    let text = format!(
        "storage transform {}\n{}\nsource {source}\n",
        storage_view::VERSION,
        transform.identity()
    );
    Sha256::digest(text.as_bytes()).into()
}

/// Makes the transformed files of the view from decoded members, so that publishing does not
/// decode them again.
pub(crate) struct Fusion<'a> {
    store: &'a LocalStore,
    derivations: &'a dyn Derivations,
    monitor: &'a Monitor,
}

impl<'a> Fusion<'a> {
    pub(crate) fn new(
        store: &'a LocalStore,
        derivations: &'a dyn Derivations,
        monitor: &'a Monitor,
    ) -> Self {
        Self {
            store,
            derivations,
            monitor,
        }
    }
}

impl sekai_pipeline::Fusion for Fusion<'_> {
    fn extra(&self, asset: &Asset, document: &AssetDocument) -> io::Result<()> {
        for file in storage_view::files(document) {
            let member = &document.members[file.member];
            let Some(sha256) = parse_hex(&member.sha256) else {
                continue;
            };
            let source = Key {
                sha256,
                extension: member.extension.clone(),
            };
            let fingerprint = fingerprint(file.transform, &source);
            if file.transform == Transform::Copy || self.derivations.get(&fingerprint)?.is_some() {
                continue;
            }
            let mut span = self.monitor.span("transform", "storage", &file.path);
            let content = &asset.members[file.member].content;
            // A failure is reported when the view is published.
            let Some(Ok(bytes)) = storage_view::apply_decoded(file.transform, content) else {
                continue;
            };
            let extension = file.transform.extension().unwrap_or(&source.extension);
            let key = self.store.put(&bytes, extension)?;
            span.bytes(0, bytes.len() as u64);
            self.derivations.put(fingerprint, Derived::Document(key))?;
        }
        Ok(())
    }
}

/// Makes a file of the view, or finds it from an earlier run among `objects`.
fn make(
    job: &Job,
    objects: &Objects<'_>,
    derivations: &dyn Derivations,
    monitor: &Monitor,
) -> std::io::Result<Result<(Key, u64, bool), storage_view::ViewError>> {
    let transform = job.file.transform;
    if transform == Transform::Copy {
        return Ok(Ok((job.source.clone(), job.size, false)));
    }
    let fingerprint = fingerprint(transform, &job.source);
    if let Some(Derived::Document(key)) = derivations.get(&fingerprint)?
        && let Some(size) = objects.size(&key)?
    {
        return Ok(Ok((key, size, false)));
    }
    let mut span = monitor.span("transform", "storage", &job.file.path);
    let member = objects.read(&job.source)?;
    let bytes = match storage_view::apply(transform, &member) {
        Ok(bytes) => bytes,
        Err(e) => return Ok(Err(e)),
    };
    let extension = transform.extension().unwrap_or(&job.source.extension);
    let key = objects.put(&bytes, extension)?;
    span.bytes(member.len() as u64, bytes.len() as u64);
    derivations.put(fingerprint, Derived::Document(key.clone()))?;
    Ok(Ok((key, bytes.len() as u64, true)))
}

/// The jobs of the files of `asset` in the view, from its asset document in `store`.
fn asset_jobs(asset: &Binding, store: &dyn Store) -> Result<Vec<Job>, String> {
    let bytes = store
        .read(&asset.document)
        .map_err(|e| format!("{}: {e}", asset.name))?;
    let document = AssetDocument::from_bytes(&bytes).map_err(|e| format!("{}: {e}", asset.name))?;
    storage_view::files(&document)
        .into_iter()
        .map(|file| {
            let member = &document.members[file.member];
            let sha256 = parse_hex(&member.sha256)
                .ok_or_else(|| format!("{}: bad member hash", asset.name))?;
            Ok(Job {
                asset: asset.name.clone(),
                source: Key {
                    sha256,
                    extension: member.extension.clone(),
                },
                size: member.size,
                media_type: file
                    .transform
                    .media_type()
                    .unwrap_or(&member.media_type)
                    .to_owned(),
                role: member.role.clone(),
                file,
            })
        })
        .collect()
}

/// The jobs of the view of `assets`, in path order, with their asset documents read from `store`
/// on up to `threads` threads. Of the assets that land on one path, the first by name keeps it;
/// the count of the others is returned.
fn jobs(
    assets: &[Binding],
    store: &dyn Store,
    threads: usize,
) -> Result<(Vec<Job>, usize), String> {
    let made = parallel(assets.len(), threads, |i| asset_jobs(&assets[i], store));
    let mut by_path: BTreeMap<String, Job> = BTreeMap::new();
    let mut shadowed = 0;
    for jobs in made {
        for job in jobs? {
            match by_path.get_mut(&job.file.path) {
                Some(kept) if kept.asset <= job.asset => shadowed += 1,
                Some(kept) => {
                    *kept = job;
                    shadowed += 1;
                }
                None => {
                    by_path.insert(job.file.path.clone(), job);
                }
            }
        }
    }
    Ok((by_path.into_values().collect(), shadowed))
}

/// The files of the view of `assets`, by path, made on up to `threads` threads. A file that
/// cannot be made is reported and left out.
fn files(
    assets: &[Binding],
    objects: &Objects<'_>,
    derivations: &dyn Derivations,
    threads: usize,
    monitor: &Monitor,
) -> Result<BTreeMap<String, File>, String> {
    let (jobs, shadowed) = jobs(assets, objects, threads)?;
    let results = parallel(jobs.len(), threads, |i| {
        isolated(|| make(&jobs[i], objects, derivations, monitor)).unwrap_or_else(|message| {
            Ok(Err(storage_view::ViewError {
                code: PANICKED,
                message,
            }))
        })
    });
    derivations.flush().map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    let (mut made, mut failed) = (0, 0);
    for (job, result) in jobs.into_iter().zip(results) {
        match result.map_err(|e| e.to_string())? {
            Ok((key, size, new)) => {
                made += usize::from(new);
                let file = File {
                    path: job.file.path,
                    key,
                    size,
                    media_type: job.media_type,
                    asset: job.asset,
                    role: job.role,
                };
                files.insert(file.path.clone(), file);
            }
            Err(e) => {
                failed += 1;
                eprintln!("{}: {e}", job.file.path);
            }
        }
    }
    println!(
        "storage files {} transformed {made} shadowed {shadowed} failed {failed}",
        files.len()
    );
    Ok(files)
}

/// The change to the storage view in the directories of `units`, which now hold the files of
/// `assets`, every produced asset that those units own. What the directories held before is read
/// from `previous`, the previous publication of the view.
pub(crate) fn change(
    units: &BTreeSet<&str>,
    assets: &[Binding],
    previous: &Lookup<'_>,
    objects: &Objects<'_>,
    derivations: &dyn Derivations,
    threads: usize,
    monitor: &Monitor,
) -> Result<Change, String> {
    let _span = monitor.span("publish", "storage", "");
    let files = files(assets, objects, derivations, threads, monitor)?;
    let outside = files.values().find(|file| {
        let directory = file.path.rsplit_once('/').map(|(directory, _)| directory);
        !directory.is_some_and(|directory| units.contains(directory))
    });
    if let Some(file) = outside {
        return Err(format!(
            "the storage file {} of {} lies outside the directory of its unit",
            file.path, file.asset
        ));
    }
    // Every file of the directories is replaced; a file that stays the same gives the same row.
    let mut change = Change::default();
    for unit in units {
        for old in previous.directory(unit).map_err(|e| e.to_string())? {
            change.remove_file(&old.path);
        }
    }
    for file in files.into_values() {
        change.add_file(file).map_err(|e| e.to_string())?;
    }
    Ok(change)
}
