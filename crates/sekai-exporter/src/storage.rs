//! Publishes the `storage` view of an evaluation as a path tree.

use std::collections::BTreeMap;
use std::path::Path;

use sekai_catalog::store::parse_hex;
use sekai_catalog::{Derivations, Derived, Key, LocalStore};
use sekai_format::AssetDocument;
use sekai_format::publication::AssetFailure;
use sekai_moesekai::storage_view::{self, Transform, ViewFile};
use sekai_pipeline::metrics::Recorder;
use sekai_pipeline::{Evaluation, parallel};
use sekai_publish::{File, Plan, tree};
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

/// Makes a file of the view, or finds it from an earlier run.
fn make(
    job: &Job,
    store: &LocalStore,
    derivations: &dyn Derivations,
    monitor: &Monitor,
) -> std::io::Result<Result<(Key, u64, bool), storage_view::ViewError>> {
    let transform = job.file.transform;
    if transform == Transform::Copy {
        return Ok(Ok((job.source.clone(), job.size, false)));
    }
    let fingerprint = fingerprint(transform, &job.source);
    if let Some(Derived::Document(key)) = derivations.get(&fingerprint)?
        && store.contains(&key)
    {
        let size = std::fs::metadata(store.path(&key))?.len();
        return Ok(Ok((key, size, false)));
    }
    let mut span = monitor.span("transform", "storage", &job.file.path);
    let member = store.read(&job.source)?;
    let bytes = match storage_view::apply(transform, &member) {
        Ok(bytes) => bytes,
        Err(e) => return Ok(Err(e)),
    };
    let extension = transform.extension().unwrap_or(&job.source.extension);
    let key = store.put(&bytes, extension)?;
    span.bytes(member.len() as u64, bytes.len() as u64);
    derivations.put(fingerprint, Derived::Document(key.clone()))?;
    Ok(Ok((key, bytes.len() as u64, true)))
}

/// The jobs of the view, in path order, and the failures of assets that have no files. Of the
/// assets that land on one path, the first by name keeps it; the count of the others is returned.
fn jobs(
    evaluation: &Evaluation,
    store: &LocalStore,
) -> Result<(Vec<Job>, Vec<AssetFailure>, usize), String> {
    let mut by_path: BTreeMap<String, Job> = BTreeMap::new();
    let mut failures = Vec::new();
    let mut shadowed = 0;
    for output in &evaluation.outputs {
        if let Some(code) = &output.error {
            failures.push(AssetFailure {
                name: output.name.clone(),
                code: code.clone(),
            });
            continue;
        }
        let bytes = store.read(&output.document).map_err(|e| e.to_string())?;
        let document =
            AssetDocument::from_bytes(&bytes).map_err(|e| format!("{}: {e}", output.name))?;
        for file in storage_view::files(&document) {
            let member = &document.members[file.member];
            let sha256 = parse_hex(&member.sha256)
                .ok_or_else(|| format!("{}: bad member hash", output.name))?;
            let job = Job {
                asset: output.name.clone(),
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
            };
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
    Ok((by_path.into_values().collect(), failures, shadowed))
}

/// Publishes the view of `evaluation` at `root`.
pub(crate) fn publish(
    evaluation: &Evaluation,
    version: &str,
    root: &Path,
    store: &LocalStore,
    derivations: &dyn Derivations,
    threads: usize,
    monitor: &Monitor,
) -> Result<(), lexopt::Error> {
    let span = monitor.span("publish", "storage", "");
    let (jobs, mut failures, shadowed) = jobs(evaluation, store)?;
    let results = parallel(jobs.len(), threads, |i| {
        make(&jobs[i], store, derivations, monitor)
    });
    derivations.flush().map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    let mut made = 0;
    for (job, result) in jobs.iter().zip(results) {
        match result.map_err(|e| e.to_string())? {
            Ok((key, size, new)) => {
                made += usize::from(new);
                files.push(File {
                    path: job.file.path.clone(),
                    key,
                    size,
                    media_type: job.media_type.clone(),
                    asset: job.asset.clone(),
                    role: job.role.clone(),
                });
            }
            Err(e) => {
                eprintln!("{}: {e}", job.file.path);
                failures.push(AssetFailure {
                    name: job.asset.clone(),
                    code: e.code.to_owned(),
                });
            }
        }
    }
    failures.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.code.cmp(&b.code)));
    failures.dedup_by(|a, b| a.name == b.name);
    let mut records = Sha256::new();
    for file in &files {
        records.update(format!("{} {}\n", file.path, file.key));
    }
    let revision: [u8; 32] = records.finalize().into();
    let plan = Plan {
        version: version.to_owned(),
        files,
        listings: BTreeMap::new(),
        failures,
        revision: revision.iter().map(|b| format!("{b:02x}")).collect(),
    };
    let report = tree::publish(store, root, &plan).map_err(|e| e.to_string())?;
    drop(span);
    println!(
        "storage files {} transformed {made} shadowed {shadowed} failed {} written {} unchanged {} deleted {} revision {}",
        plan.files.len(),
        plan.failures.len(),
        report.written,
        report.unchanged,
        report.deleted,
        plan.revision
    );
    Ok(())
}
