//! Derivations: fingerprints, conversion, canonical encoding, the store, and the scheduling of a
//! set of derivations.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::io;

use sekai_catalog::{Derivations, Derived, Key, Store};
use sekai_convert::{Asset, Content, ConvertError, Converter, Cost, CostClass, Encoders, ObjectId};
use sekai_encode::{EncodeError, exr, flac, mp4, webp_lossless};
use sekai_format::asset::{FORMAT, VERSION};
use sekai_format::{AssetDocument, AssetError, MemberFile, SourceObject, SourceResource};
use sekai_unity::ClassLayouts;
use sekai_unity::index::VersionIndex;
use sha2::{Digest, Sha256};

use crate::index_record::StreamDigest;
use crate::metrics::Recorder;
use crate::reads::Reads;
use crate::sched::{Limits, Need, Resident, Scheduler, Slots};
use crate::version::Version;

/// One asset of an evaluated version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// The name of the asset.
    pub name: String,
    /// The bundle of the root object, or empty for an asset built from tables.
    pub bundle: String,
    /// The asset form.
    pub form: &'static str,
    /// The asset document.
    pub document: Key,
    /// The error code of an asset that could not be produced.
    pub error: Option<String>,
}

/// The assets of an evaluation.
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
/// With one thread every job runs on the calling thread and no thread is created.
pub fn parallel<T: Send>(n: usize, threads: usize, job: impl Fn(usize) -> T + Sync) -> Vec<T> {
    let scheduler = Scheduler::new(Limits {
        cpu: threads,
        memory: u64::MAX,
        connections: Vec::new(),
        resident: None,
    });
    scheduler.run(&vec![Need::default(); n], |i, _| job(i))
}

/// How derivations run.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// The CPU slots: how many conversions and encodings run at once.
    pub threads: usize,
    /// The memory budget of the derivations that run at once, in bytes. A derivation whose
    /// estimate exceeds the budget runs alone.
    pub memory: u64,
    /// A limit on the memory of the whole process, which holds back derivations while the
    /// process holds more than their estimates account for.
    pub resident: Option<Resident>,
    /// Plain mode: one derivation after another on the calling thread, each evaluated on its
    /// own, without reading or writing derivation records. It is the reference the other modes
    /// are compared with, and leaves the records of later runs unchanged.
    pub plain: bool,
}

impl Options {
    /// `threads` CPU slots with no memory limit.
    pub fn threads(threads: usize) -> Self {
        Self {
            threads,
            memory: u64::MAX,
            resident: None,
            plain: false,
        }
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Further files of an asset, made while its members are still decoded. They must not change any
/// canonical output.
pub trait Fusion: Sync {
    /// Makes the files for `asset`, whose members are those of `document`, in the same order.
    fn extra(&self, asset: &Asset, document: &AssetDocument) -> io::Result<()>;
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

impl Planned<'_> {
    /// The estimated cost of evaluating this derivation, from the object headers of its root.
    pub(crate) fn cost(&self, version: &Version<'_>) -> Cost {
        let index = version.index();
        let root = self.closure[0];
        match index.position(root) {
            Some(position) => {
                let object = &index.bundles()[root.bundle].files[root.file].objects[position];
                self.converter.cost(root, object, version)
            }
            None => Cost::light(0),
        }
    }

    /// The bundles the closure reads from, in order.
    fn bundles(&self) -> Vec<usize> {
        let bundles: BTreeSet<usize> = self.closure.iter().map(|id| id.bundle).collect();
        bundles.into_iter().collect()
    }
}

pub(crate) fn fingerprint(
    converter: &dyn Converter,
    name: &str,
    index: &VersionIndex,
    closure: &[ObjectId],
    streams: &[Vec<Vec<StreamDigest>>],
    classes: &dyn ClassLayouts,
) -> [u8; 32] {
    let mut text = format!(
        "fingerprint 2\nform {} {}\nname {name}\n",
        converter.name(),
        converter.version(),
    );
    let encoders = converter.encoders();
    if encoders.image {
        text.push_str(&format!(
            "encoder {} {}\n",
            webp_lossless::NAME,
            webp_lossless::VERSION
        ));
    }
    if encoders.hdr_image {
        text.push_str(&format!("encoder {} {}\n", exr::NAME, exr::VERSION));
    }
    if encoders.audio {
        text.push_str(&format!("encoder {} {}\n", flac::NAME, flac::VERSION));
    }
    if encoders.video {
        text.push_str(&format!("encoder {} {}\n", mp4::NAME, mp4::VERSION));
    }
    if let Some(identity) = classes.identity() {
        text.push_str(&format!("classes {}\n", hex(&identity)));
    }
    for id in closure {
        if let Some(position) = index.position(*id) {
            let object = &index.bundles()[id.bundle].files[id.file].objects[position];
            text.push_str(&format!("object {}\n", hex(&object.sha256)));
        }
        if let Some(digest) = streams
            .get(id.bundle)
            .and_then(|b| b.get(id.file))
            .and_then(|list| {
                list.binary_search_by_key(&id.path_id, |d| d.path_id)
                    .ok()
                    .map(|pos| &list[pos])
            })
        {
            match digest.sha256 {
                Some(sha) => text.push_str(&format!("stream {}\n", hex(&sha))),
                None => text.push_str("stream missing\n"),
            }
        }
    }
    Sha256::digest(text.as_bytes()).into()
}

/// The canonical file of a member.
pub(crate) struct Encoded<'a> {
    pub(crate) bytes: Cow<'a, [u8]>,
    pub(crate) extension: &'a str,
    pub(crate) media_type: &'static str,
}

pub(crate) fn encode(content: &Content) -> Result<Encoded<'_>, EncodeError> {
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

pub(crate) fn asset_error(e: &ConvertError) -> AssetError {
    AssetError {
        code: e.code.to_owned(),
        message: e.message.clone(),
    }
}

/// The size in memory of the decoded members of an asset.
fn decoded_size(asset: &Asset) -> u64 {
    let size = |content: &Content| match content {
        Content::Image(surface) => surface.pixels().len(),
        Content::HdrImage(surface) => surface.pixels().len() * 2,
        Content::Audio(pcm) => pcm.samples().len() * 2,
        Content::Video(movie) => {
            movie.stream.len()
                + movie
                    .audio
                    .as_ref()
                    .map_or(0, |pcm| pcm.samples().len() * 2)
        }
        Content::File { bytes, .. } => bytes.len(),
    };
    asset.members.iter().map(|m| size(&m.content) as u64).sum()
}

/// The estimated peak memory of a derivation.
pub(crate) fn estimated_memory(plan: &Planned<'_>, version: &Version<'_>) -> u64 {
    plan.cost(version).memory
}

/// Evaluates one derivation on the calling thread and returns its output and the error code of a
/// failed asset.
pub(crate) fn derive(
    plan: &Planned<'_>,
    version: &Version<'_>,
    store: &dyn Store,
    recorder: &dyn Recorder,
    fusion: Option<&dyn Fusion>,
) -> io::Result<(Derived, Option<String>)> {
    let cost = plan.cost(version);
    let alone = Scheduler::new(Limits {
        cpu: 1,
        memory: u64::MAX,
        connections: Vec::new(),
        resident: None,
    });
    alone
        .run(&[Need::default()], |_, slots| {
            derive_within(plan, cost, slots, version, store, recorder, fusion)
        })
        .pop()
        .expect("the derivation ran")
}

fn check_declared_encoders(
    declared: Encoders,
    asset: &Asset,
    form: &str,
) -> Result<(), ConvertError> {
    for member in &asset.members {
        let missing = match &member.content {
            Content::Image(_) if !declared.image => Some(webp_lossless::NAME),
            Content::HdrImage(_) if !declared.hdr_image => Some(exr::NAME),
            Content::Audio(_) if !declared.audio => Some(flac::NAME),
            Content::Video(_) if !declared.video => Some(mp4::NAME),
            _ => None,
        };
        if let Some(encoder) = missing {
            return Err(ConvertError::new(
                "undeclared_encoder",
                format!(
                    "converter {form} produced member {} using encoder {encoder} without declaring it",
                    member.role
                ),
            ));
        }
    }
    Ok(())
}

/// Evaluates one derivation as [`derive`] does, within the CPU slots of a running job.
///
/// `cost` is the estimate the derivation was scheduled with, which the convert span reports
/// beside the decoded size. The conversion takes one CPU slot of `slots` and the encoding of each
/// member takes one more, two for [`CostClass::Wide`] work.
pub(crate) fn derive_within(
    plan: &Planned<'_>,
    cost: Cost,
    slots: &Slots<'_>,
    version: &Version<'_>,
    store: &dyn Store,
    recorder: &dyn Recorder,
    fusion: Option<&dyn Fusion>,
) -> io::Result<(Derived, Option<String>)> {
    let form = plan.converter.name();
    let reads = Reads::new(version, &plan.closure, Some(version.all_streams()));
    let result = slots.cpu(1, || {
        let mut span = recorder.span("convert", form, &plan.name);
        let result = plan.converter.convert(plan.closure[0], &reads);
        if let Ok(Some(asset)) = &result {
            span.memory(cost.memory, decoded_size(asset));
        }
        result
    });
    let result = match result {
        Ok(Some(asset)) => {
            check_declared_encoders(plan.converter.encoders(), &asset, form).map(|()| Some(asset))
        }
        other => other,
    };
    let (objects, resources) = reads.finish();
    let encoders = if cost.class == CostClass::Wide { 2 } else { 1 };
    let mut members = Vec::new();
    let mut decoded = None;
    let error = match result {
        Ok(None) => return Ok((Derived::Nothing, None)),
        Ok(Some(asset)) => {
            let mut failed = None;
            for member in &asset.members {
                let encoded = slots.cpu(encoders, || {
                    let mut span = recorder.span("encode", form, &plan.name);
                    let encoded = encode(&member.content);
                    if let Ok(file) = &encoded {
                        span.bytes(0, file.bytes.len() as u64);
                    }
                    encoded
                });
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
                        let code = match e {
                            EncodeError::Unavailable { .. } => "encoder_unavailable",
                            _ => "encode_failed",
                        };
                        failed = Some(AssetError {
                            code: code.to_owned(),
                            message: e.to_string(),
                        });
                        break;
                    }
                }
            }
            decoded = Some(asset);
            failed
        }
        Err(e) => Some(asset_error(&e)),
    };
    let code = error.as_ref().map(|e| e.code.clone());
    let document = build_document(plan, version, &objects, &resources, members, error);
    let key = store_document(store, &document)?;
    if let (Some(fusion), Some(asset)) = (fusion, &decoded) {
        slots.cpu(1, || fusion.extra(asset, &document))?;
    }
    Ok((Derived::Document(key), code))
}

/// Stores the document of an asset that could not be produced.
pub(crate) fn document(
    plan: &Planned<'_>,
    version: &Version<'_>,
    store: &dyn Store,
    members: Vec<MemberFile>,
    error: AssetError,
) -> io::Result<Key> {
    let root = &plan.closure[..1];
    store_document(
        store,
        &build_document(plan, version, root, &[], members, Some(error)),
    )
}

fn store_document(store: &dyn Store, document: &AssetDocument) -> io::Result<Key> {
    let bytes = document.to_bytes().map_err(io::Error::other)?;
    store.put(&bytes, "json")
}

fn build_document(
    plan: &Planned<'_>,
    version: &Version<'_>,
    objects: &[ObjectId],
    resources: &[(usize, String)],
    members: Vec<MemberFile>,
    error: Option<AssetError>,
) -> AssetDocument {
    let index = version.index();
    let resources = resources
        .iter()
        .map(|(bundle, file)| SourceResource {
            bundle: index.bundles()[*bundle].name.clone(),
            file: file.clone(),
        })
        .collect();
    let sources = objects
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
    AssetDocument {
        format: FORMAT.to_owned(),
        version: VERSION,
        name: plan.name.clone(),
        form: plan.converter.name().to_owned(),
        form_version: plan.converter.version(),
        fingerprint: hex(&plan.fingerprint),
        sources,
        resources,
        members,
        references: Vec::new(),
        error,
    }
}

/// An estimate of the time a derivation takes, in arbitrary units.
fn effort(cost: &Cost) -> u64 {
    let weight = match cost.class {
        CostClass::Light => 1,
        CostClass::Heavy => 4,
        CostClass::Wide => 8,
    };
    cost.memory.saturating_mul(weight)
}

/// The order to run `pending` in, as indices into it. Derivations do not depend on each other,
/// so they form one layer. Derivations that read the same set of bundles run together, so the
/// bundles are read while they are cached; the groups and the derivations within each group
/// run longest first, so the last to finish are short.
fn order(plans: &[Planned<'_>], pending: &[(usize, Cost)]) -> Vec<usize> {
    let mut groups: BTreeMap<Vec<usize>, Vec<usize>> = BTreeMap::new();
    for (k, (i, _)) in pending.iter().enumerate() {
        groups.entry(plans[*i].bundles()).or_default().push(k);
    }
    let longest = |k: &usize| std::cmp::Reverse(effort(&pending[*k].1));
    let mut groups: Vec<Vec<usize>> = groups.into_values().collect();
    for group in &mut groups {
        group.sort_by_key(longest);
    }
    groups.sort_by_key(|group| longest(&group[0]));
    groups.concat()
}

/// The outcome of [`run_each`].
pub(crate) struct Ran {
    /// The asset of each derivation, in order; `None` for a derivation with no asset.
    pub(crate) outputs: Vec<Option<Output>>,
    /// The derivations that ran.
    pub(crate) converted: usize,
    /// The derivations whose output already existed.
    pub(crate) reused: usize,
}

/// Runs the derivations of `plans` whose output is not recorded in `derivations` yet, records the
/// new outputs and reports the asset of each derivation, in order. An asset that cannot be
/// produced is reported with its error and not recorded; only a failure to read or write the
/// store or the records fails the run.
pub(crate) fn run_each(
    plans: &[Planned<'_>],
    version: &Version<'_>,
    store: &dyn Store,
    derivations: &dyn Derivations,
    recorder: &dyn Recorder,
    options: &Options,
    fusion: Option<&dyn Fusion>,
) -> io::Result<Ran> {
    let threads = if options.plain { 1 } else { options.threads };
    let recorded = parallel(plans.len(), threads, |i| -> io::Result<_> {
        let plan = &plans[i];
        if !options.plain
            && let Some(derived) = derivations.get(&plan.fingerprint)?
        {
            return Ok(Ok(derived));
        }
        Ok(Err(plan.cost(version)))
    });
    let mut results: Vec<Option<(Derived, Option<String>, bool)>> = vec![None; plans.len()];
    let mut pending = Vec::new();
    for (i, result) in recorded.into_iter().enumerate() {
        match result? {
            Ok(derived) => results[i] = Some((derived, None, true)),
            Err(cost) => pending.push((i, cost)),
        }
    }
    let order = order(plans, &pending);
    let needs: Vec<Need> = order
        .iter()
        .map(|k| Need {
            memory: pending[*k].1.memory,
            remote: None,
        })
        .collect();
    let scheduler = Scheduler::new(Limits {
        cpu: threads,
        memory: options.memory,
        connections: Vec::new(),
        resident: options.resident,
    });
    let derived = scheduler.run(&needs, |j, slots| -> io::Result<_> {
        let (i, cost) = pending[order[j]];
        let plan = &plans[i];
        let (derived, error) = derive_within(plan, cost, slots, version, store, recorder, fusion)?;
        // A failed asset has no output, so the next run evaluates it again.
        if !options.plain && error.is_none() {
            derivations.put(plan.fingerprint, derived.clone())?;
        }
        Ok((i, derived, error))
    });
    for result in derived {
        let (i, derived, error) = result?;
        results[i] = Some((derived, error, false));
    }
    if !options.plain {
        derivations.flush()?;
    }
    let mut ran = Ran {
        outputs: Vec::with_capacity(plans.len()),
        converted: 0,
        reused: 0,
    };
    for (plan, result) in plans.iter().zip(results) {
        let (derived, error, reused) = result.expect("every derivation ran or was recorded");
        if reused {
            ran.reused += 1;
        } else {
            ran.converted += 1;
        }
        ran.outputs.push(match derived {
            Derived::Document(document) => Some(Output {
                name: plan.name.clone(),
                bundle: version.index().bundles()[plan.closure[0].bundle]
                    .name
                    .clone(),
                form: plan.converter.name(),
                document,
                error,
            }),
            Derived::Nothing => None,
        });
    }
    Ok(ran)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sekai_convert::{Asset, Context, ConvertError, ObjectIndex};
    use sekai_unity::{NoClasses, TypeTree};

    use super::*;

    struct Form;

    impl Converter for Form {
        fn name(&self) -> &'static str {
            "form"
        }

        fn version(&self) -> u32 {
            1
        }

        fn is_root(&self, _: &ObjectIndex, _: Option<&str>, _: &[String]) -> bool {
            false
        }

        fn convert(&self, _: ObjectId, _: &dyn Context) -> Result<Option<Asset>, ConvertError> {
            Ok(None)
        }
    }

    /// A table of class layouts with the identity `.0`.
    struct Table([u8; 32]);

    impl ClassLayouts for Table {
        fn tree(&self, _class_id: i32, _unity_version: &str) -> Option<Arc<TypeTree>> {
            None
        }

        fn identity(&self) -> Option<[u8; 32]> {
            Some(self.0)
        }
    }

    #[test]
    fn the_fingerprint_follows_the_table_of_class_layouts() {
        let index = VersionIndex::new(Vec::new());
        let of = |classes: &dyn ClassLayouts| fingerprint(&Form, "name", &index, &[], &[], classes);
        assert_eq!(of(&Table([1; 32])), of(&Table([1; 32])));
        assert_ne!(of(&Table([1; 32])), of(&Table([2; 32])));
        assert_ne!(of(&Table([1; 32])), of(&NoClasses));
    }

    #[test]
    fn fingerprint_scopes_encoders_and_stream_slices_to_declared_closure() {
        use sekai_unity::index::{BundleIndex, FileIndex};

        struct WithEnc(Encoders);
        impl Converter for WithEnc {
            fn name(&self) -> &'static str {
                "form"
            }
            fn version(&self) -> u32 {
                1
            }
            fn encoders(&self) -> Encoders {
                self.0
            }
            fn is_root(&self, _: &ObjectIndex, _: Option<&str>, _: &[String]) -> bool {
                false
            }
            fn convert(&self, _: ObjectId, _: &dyn Context) -> Result<Option<Asset>, ConvertError> {
                Ok(None)
            }
        }

        let obj_a = ObjectIndex {
            path_id: 1,
            class_id: 49,
            class: "TextAsset".to_owned(),
            name: None,
            script: None,
            script_name: None,
            references: Vec::new(),
            size: 4,
            sha256: [0xaa; 32],
            error: None,
        };
        let obj_b = ObjectIndex {
            path_id: 2,
            class_id: 28,
            class: "Texture2D".to_owned(),
            name: None,
            script: None,
            script_name: None,
            references: Vec::new(),
            size: 4,
            sha256: [0xbb; 32],
            error: None,
        };
        let index = VersionIndex::new(vec![BundleIndex {
            name: "b".to_owned(),
            files: vec![FileIndex {
                path: "CAB-1".to_owned(),
                externals: Vec::new(),
                objects: vec![obj_a, obj_b],
                container: Vec::new(),
            }],
        }]);

        let id_text = ObjectId {
            bundle: 0,
            file: 0,
            path_id: 1,
        };
        let id_tex = ObjectId {
            bundle: 0,
            file: 0,
            path_id: 2,
        };
        let streams_v1 = vec![vec![vec![StreamDigest {
            path_id: 2,
            file: "CAB-1.resS".to_owned(),
            offset: 0,
            size: 64,
            sha256: Some([0x11; 32]),
        }]]];
        let streams_v2 = vec![vec![vec![StreamDigest {
            path_id: 2,
            file: "CAB-1.resS".to_owned(),
            offset: 0,
            size: 64,
            sha256: Some([0x22; 32]),
        }]]];

        // Sibling TextAsset in the same bundle is unaffected when Texture2D's `.resS` slice changes.
        assert_eq!(
            fingerprint(&Form, "txt", &index, &[id_text], &streams_v1, &NoClasses),
            fingerprint(&Form, "txt", &index, &[id_text], &streams_v2, &NoClasses),
        );
        // Texture2D itself tracks its own `.resS` slice digest.
        assert_ne!(
            fingerprint(
                &WithEnc(Encoders::IMAGE),
                "tex",
                &index,
                &[id_tex],
                &streams_v1,
                &NoClasses
            ),
            fingerprint(
                &WithEnc(Encoders::IMAGE),
                "tex",
                &index,
                &[id_tex],
                &streams_v2,
                &NoClasses
            ),
        );
        // Different declared encoder capabilities produce distinct fingerprints.
        assert_ne!(
            fingerprint(
                &WithEnc(Encoders::NONE),
                "x",
                &index,
                &[id_text],
                &streams_v1,
                &NoClasses
            ),
            fingerprint(
                &WithEnc(Encoders::AUDIO),
                "x",
                &index,
                &[id_text],
                &streams_v1,
                &NoClasses
            ),
        );
    }
}
