//! The eager evaluator: names, fingerprints, conversion, canonical encoding and the store.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::io;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use sekai_catalog::{Derivations, Derived, Key, LocalStore};
use sekai_convert::{Content, ConvertError, Converter, ObjectId};
use sekai_encode::{EncodeError, exr, flac, mp4, webp_lossless};
use sekai_format::asset::{FORMAT, VERSION};
use sekai_format::{AssetDocument, AssetError, MemberFile, SourceObject};
use sekai_unity::index::{Resolved, VersionIndex};
use sha2::{Digest, Sha256};

use crate::metrics::Recorder;
use crate::naming::{Candidate, assign, logical_path};
use crate::version::Version;

/// One asset of an evaluated version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The name of the asset.
    pub name: String,
    /// The asset form.
    pub form: &'static str,
    /// The asset document.
    pub document: Key,
    /// The error code of an asset that could not be produced.
    pub error: Option<String>,
}

/// The result of evaluating a version.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Evaluation {
    /// The assets, in name order.
    pub outputs: Vec<Output>,
    /// Roots that cannot be told apart from another root by name, as `(logical path, path ID)`.
    pub unnamed: Vec<(String, i64)>,
    /// The derivations that ran.
    pub converted: usize,
    /// The derivations whose output already existed.
    pub reused: usize,
}

/// Runs `job` for indices `0..n` on `threads` threads and returns the results in index order.
pub fn parallel<T: Send>(n: usize, threads: usize, job: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<T>>> = Mutex::new((0..n).map(|_| None).collect());
    std::thread::scope(|s| {
        for _ in 0..threads.min(n.max(1)) {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= n {
                        break;
                    }
                    let r = job(i);
                    results.lock().unwrap()[i] = Some(r);
                }
            });
        }
    });
    results
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|r| r.expect("every index ran"))
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The root and every object reachable from it through references, root first and the rest in
/// order.
fn closure(index: &VersionIndex, root: ObjectId) -> Vec<ObjectId> {
    let mut seen = BTreeSet::from([root]);
    let mut queue = vec![root];
    while let Some(id) = queue.pop() {
        let Some(position) = index.position(id) else {
            continue;
        };
        for reference in &index.bundles()[id.bundle].files[id.file].objects[position].references {
            if let Ok(Resolved::Object(target)) = index.resolve(id, *reference)
                && index.position(target).is_some()
                && seen.insert(target)
            {
                queue.push(target);
            }
        }
    }
    seen.remove(&root);
    std::iter::once(root).chain(seen).collect()
}

/// A derivation to evaluate.
pub(crate) struct Planned<'a> {
    /// The converter of the asset form.
    pub(crate) converter: &'a dyn Converter,
    /// The name of the asset.
    pub(crate) name: String,
    /// The root and every object it reaches, root first.
    pub(crate) closure: Vec<ObjectId>,
    /// The fingerprint of the derivation.
    pub(crate) fingerprint: [u8; 32],
}

/// The derivations of a version, before any of them runs.
pub(crate) struct Plan<'a> {
    /// The derivations, in name order.
    pub(crate) derivations: Vec<Planned<'a>>,
    /// Roots that cannot be told apart from another root by name, as `(logical path, path ID)`,
    /// in order.
    pub(crate) unnamed: Vec<(String, i64)>,
}

fn fingerprint(
    converter: &dyn Converter,
    name: &str,
    index: &VersionIndex,
    closure: &[ObjectId],
    resources: &[Vec<[u8; 32]>],
) -> [u8; 32] {
    let mut text = format!(
        "fingerprint 1\nform {} {}\nname {name}\nencoder {} {}\nencoder {} {}\nencoder {} {}\nencoder {} {}\n",
        converter.name(),
        converter.version(),
        webp_lossless::NAME,
        webp_lossless::VERSION,
        exr::NAME,
        exr::VERSION,
        flac::NAME,
        flac::VERSION,
        mp4::NAME,
        mp4::VERSION,
    );
    let mut bundles = BTreeSet::new();
    for id in closure {
        bundles.insert(id.bundle);
        if let Some(position) = index.position(*id) {
            let object = &index.bundles()[id.bundle].files[id.file].objects[position];
            text.push_str(&format!("object {}\n", hex(&object.sha256)));
        }
    }
    for bundle in bundles {
        for hash in &resources[bundle] {
            text.push_str(&format!("resource {}\n", hex(hash)));
        }
    }
    Sha256::digest(text.as_bytes()).into()
}

/// The canonical file of a member.
struct Encoded<'a> {
    bytes: Cow<'a, [u8]>,
    extension: &'a str,
    media_type: &'static str,
}

fn encode(content: &Content) -> Result<Encoded<'_>, EncodeError> {
    let encoded = |bytes: Vec<u8>, extension, media_type| Encoded {
        bytes: bytes.into(),
        extension,
        media_type,
    };
    Ok(match content {
        Content::Image(surface) => encoded(webp_lossless::encode(surface)?, "webp", "image/webp"),
        Content::HdrImage(surface) => encoded(exr::encode(surface)?, "exr", "image/x-exr"),
        Content::Audio(pcm) => encoded(flac::encode(pcm)?, "flac", "audio/flac"),
        Content::Video(movie) => encoded(mp4::encode(movie)?, "mp4", "video/mp4"),
        Content::File { extension, bytes } => Encoded {
            bytes: bytes.into(),
            extension,
            media_type: media_type(extension),
        },
    })
}

fn media_type(extension: &str) -> &'static str {
    match extension {
        "json" => "application/json",
        "txt" => "text/plain",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "glb" => "model/gltf-binary",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

fn asset_error(e: &ConvertError) -> AssetError {
    AssetError {
        code: e.code.to_owned(),
        message: e.message.clone(),
    }
}

/// Evaluates one derivation and returns its output and the error code of a failed asset.
pub(crate) fn derive(
    plan: &Planned<'_>,
    version: &Version<'_>,
    store: &LocalStore,
    recorder: &dyn Recorder,
) -> io::Result<(Derived, Option<String>)> {
    let index = version.index();
    let form = plan.converter.name();
    let result = {
        let _span = recorder.span("convert", form, &plan.name);
        plan.converter.convert(plan.closure[0], version)
    };
    let mut members = Vec::new();
    let error = match result {
        Ok(None) => return Ok((Derived::Nothing, None)),
        Ok(Some(asset)) => {
            let mut failed = None;
            for member in &asset.members {
                let encoded = {
                    let mut span = recorder.span("encode", form, &plan.name);
                    let encoded = encode(&member.content);
                    if let Ok(file) = &encoded {
                        span.bytes(0, file.bytes.len() as u64);
                    }
                    encoded
                };
                match encoded {
                    Ok(file) => {
                        let mut span = recorder.span("store", file.extension, &plan.name);
                        span.bytes(0, file.bytes.len() as u64);
                        let key = store.put(&file.bytes, file.extension)?;
                        drop(span);
                        members.push(MemberFile {
                            role: member.role.clone(),
                            sha256: hex(&key.sha256),
                            size: file.bytes.len() as u64,
                            extension: file.extension.to_owned(),
                            media_type: file.media_type.to_owned(),
                        });
                    }
                    Err(e) => {
                        members.clear();
                        failed = Some(AssetError {
                            code: "encode_failed".to_owned(),
                            message: e.to_string(),
                        });
                        break;
                    }
                }
            }
            failed
        }
        Err(e) => Some(asset_error(&e)),
    };
    let sources = plan
        .closure
        .iter()
        .filter_map(|id| {
            let bundle = &index.bundles()[id.bundle];
            let position = index.position(*id)?;
            Some(SourceObject {
                bundle: bundle.name.clone(),
                path_id: id.path_id,
                class: bundle.files[id.file].objects[position].class.clone(),
            })
        })
        .collect();
    let code = error.as_ref().map(|e| e.code.clone());
    let document = AssetDocument {
        format: FORMAT.to_owned(),
        version: VERSION,
        name: plan.name.clone(),
        form: plan.converter.name().to_owned(),
        form_version: plan.converter.version(),
        fingerprint: hex(&plan.fingerprint),
        sources,
        members,
        references: Vec::new(),
        error,
    };
    let bytes = document.to_bytes().map_err(io::Error::other)?;
    Ok((Derived::Document(store.put(&bytes, "json")?), code))
}

/// Evaluates every asset of `version`. An asset that cannot be produced is recorded with its
/// error and the run continues; only a failure to write the store fails the run.
pub fn evaluate(
    version: &Version<'_>,
    converters: &[Box<dyn Converter>],
    store: &LocalStore,
    derivations: &dyn Derivations,
    recorder: &dyn Recorder,
    threads: usize,
) -> io::Result<Evaluation> {
    let plan = plan(version, converters, recorder, threads);
    run(&plan, version, store, derivations, recorder, threads)
}

/// Chooses the roots of `version`, names them and computes the closure and fingerprint of each
/// derivation. Nothing is converted and the store is not read.
pub(crate) fn plan<'a>(
    version: &Version<'_>,
    converters: &'a [Box<dyn Converter>],
    recorder: &dyn Recorder,
    threads: usize,
) -> Plan<'a> {
    let index = version.index();
    let _span = recorder.span("plan", "assets", "");
    let mut roots = Vec::new();
    for (id, object, paths) in version.objects() {
        let Some(path) = paths.iter().min() else {
            continue;
        };
        let script = index
            .position(id)
            .and_then(|position| index.script(id.bundle, id.file, position))
            .flatten();
        for converter in converters {
            if converter.is_root(object, script, paths) {
                roots.push((converter.as_ref(), id, object, logical_path(path)));
            }
        }
    }
    let candidates: Vec<Candidate<'_>> = roots
        .iter()
        .map(|(_, id, object, logical)| Candidate {
            logical,
            class_id: object.class_id,
            class: &object.class,
            bundle: &index.bundles()[id.bundle].name,
            path_id: id.path_id,
        })
        .collect();
    let names = assign(&candidates);

    let resources = parallel(index.bundles().len(), threads, |b| {
        let _span = recorder.span("hash", "resources", &index.bundles()[b].name);
        version.resource_hashes(b)
    });
    let mut unnamed = Vec::new();
    let mut derivations = Vec::new();
    for ((converter, id, _, logical), name) in roots.iter().zip(names) {
        let Some(name) = name else {
            unnamed.push(((*logical).to_owned(), id.path_id));
            continue;
        };
        let closure = closure(index, *id);
        let fingerprint = fingerprint(*converter, &name, index, &closure, &resources);
        derivations.push(Planned {
            converter: *converter,
            name,
            closure,
            fingerprint,
        });
    }
    unnamed.sort();
    derivations.sort_by(|a, b| a.name.cmp(&b.name));
    Plan {
        derivations,
        unnamed,
    }
}

/// Runs the derivations of `plan` whose output is not recorded in `derivations` yet and records
/// the new outputs.
pub(crate) fn run(
    plan: &Plan<'_>,
    version: &Version<'_>,
    store: &LocalStore,
    derivations: &dyn Derivations,
    recorder: &dyn Recorder,
    threads: usize,
) -> io::Result<Evaluation> {
    let plans = &plan.derivations;
    let results = parallel(plans.len(), threads, |i| -> io::Result<_> {
        let plan = &plans[i];
        if let Some(derived) = derivations.get(&plan.fingerprint)? {
            return Ok((derived, None, true));
        }
        let (derived, error) = derive(plan, version, store, recorder)?;
        // A failed asset has no output, so the next run evaluates it again.
        if error.is_none() {
            derivations.put(plan.fingerprint, derived.clone())?;
        }
        Ok((derived, error, false))
    });
    derivations.flush()?;
    let mut evaluation = Evaluation {
        unnamed: plan.unnamed.clone(),
        ..Evaluation::default()
    };
    for (plan, result) in plans.iter().zip(results) {
        let (derived, error, reused) = result?;
        if reused {
            evaluation.reused += 1;
        } else {
            evaluation.converted += 1;
        }
        if let Derived::Document(document) = derived {
            evaluation.outputs.push(Output {
                name: plan.name.clone(),
                form: plan.converter.name(),
                document,
                error,
            });
        }
    }
    Ok(evaluation)
}
