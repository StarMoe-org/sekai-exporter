//! Publishes the `storage` view of an evaluation: its path index and pointer, and a path tree when
//! one is asked for.

use std::collections::BTreeMap;
use std::io;

use sekai_catalog::store::parse_hex;
use sekai_catalog::{Derivations, Derived, Key, LocalStore, Objects, Store};
use sekai_convert::Asset;
use sekai_format::AssetDocument;
use sekai_format::publication::AssetFailure;
use sekai_moesekai::storage_view::{self, Transform, ViewFile};
use sekai_pipeline::isolate::{PANICKED, isolated};
use sekai_pipeline::metrics::Recorder;
use sekai_pipeline::parallel;
use sekai_publish::{File, Layout, Plan, tree};
use sha2::{Digest, Sha256};

use crate::export::Destination;
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

/// The jobs of the view of the assets of `index`, the plan of the index publication, in path
/// order. Of the assets that land on one path, the first by name keeps it; the count of the
/// others is returned.
fn jobs(index: &Plan) -> Result<(Vec<Job>, usize), String> {
    let mut by_path: BTreeMap<String, Job> = BTreeMap::new();
    let mut shadowed = 0;
    for asset in &index.assets {
        let document = &asset.metadata;
        for file in storage_view::files(document) {
            let member = &document.members[file.member];
            let sha256 = parse_hex(&member.sha256)
                .ok_or_else(|| format!("{}: bad member hash", asset.name))?;
            let job = Job {
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
    Ok((by_path.into_values().collect(), shadowed))
}

/// Publishes the view of the assets of `index`, the plan of the index publication, as `target`
/// (`<region>/storage`) under `<output>/published`, with a path tree when `destination` has one,
/// and returns the plan of the view. The assets that could not be produced are its failures.
pub(crate) fn publish(
    index: &Plan,
    target: &str,
    destination: &Destination<'_>,
    objects: &Objects<'_>,
    derivations: &dyn Derivations,
    threads: usize,
    monitor: &Monitor,
) -> Result<Option<Plan>, lexopt::Error> {
    let span = monitor.span("publish", "storage", "");
    let (jobs, shadowed) = jobs(index)?;
    let mut failures = index.failures.clone();
    let results = parallel(jobs.len(), threads, |i| {
        isolated(|| make(&jobs[i], objects, derivations, monitor)).unwrap_or_else(|message| {
            Ok(Err(storage_view::ViewError {
                code: PANICKED,
                message,
            }))
        })
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
        version: index.version.clone(),
        files,
        assets: Vec::new(),
        listings: BTreeMap::new(),
        failures,
        revision: revision.iter().map(|b| format!("{b:02x}")).collect(),
    };
    sekai_publish::publish(
        objects,
        &destination.output.join("published"),
        target,
        &plan,
        Layout::Paths,
        threads,
    )
    .map_err(|e| e.to_string())?;
    println!(
        "storage files {} transformed {made} shadowed {shadowed} failed {} revision {}",
        plan.files.len(),
        plan.failures.len(),
        plan.revision
    );
    if let Some(root) = destination.tree {
        let report =
            tree::publish(objects.local, root, &plan, threads).map_err(|e| e.to_string())?;
        println!(
            "storage tree written {} unchanged {} deleted {}",
            report.written, report.unchanged, report.deleted
        );
    }
    drop(span);
    Ok(Some(plan))
}
