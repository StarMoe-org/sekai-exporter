//! Derivations: fingerprints, conversion, canonical encoding, the store, and the scheduling of a
//! set of derivations.

use std::collections::{BTreeMap, BTreeSet};
use std::io;

use sekai_catalog::{Derivations, Derived, Key, Store};
use sekai_convert::{Asset, Content, ConvertError, Converter, Cost, CostClass, ObjectId};
use sekai_encode::mp4;
use sekai_format::asset::{FORMAT, VERSION};
use sekai_format::{AssetDocument, AssetError, MemberFile, SourceObject, SourceResource};
use sekai_unity::ClassLayouts;
use sekai_unity::index::VersionIndex;
use sha2::{Digest, Sha256};

use crate::canonical::Files;
use crate::index_record::StreamDigest;
use crate::isolate::{PANICKED, isolated};
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
    Scheduler::new(Limits::threads(threads)).run(&vec![Need::default(); n], |i, _| job(i))
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

/// The line of a fingerprint that names the video encoder, whose files lose information, when
/// `converter` declares it, and otherwise nothing.
pub(crate) fn lossy_encoder(converter: &dyn Converter) -> String {
    if converter.encoders().video {
        format!("encoder {} {}\n", mp4::NAME, mp4::VERSION)
    } else {
        String::new()
    }
}

/// The fingerprint of a derivation: everything that determines the content of its asset.
///
/// An encoder is part of it only when its files lose information, which is the video encoder,
/// for the forms that declare it. The files of the other encoders decode to the content they
/// were given, whatever their version.
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
    text.push_str(&lossy_encoder(converter));
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
        Content::Audio(sound) => sound.memory(),
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

/// The document of an asset, and where it is stored.
#[derive(Debug)]
pub(crate) struct Stored {
    pub(crate) key: Key,
    pub(crate) document: AssetDocument,
}

impl Stored {
    fn put(store: &dyn Store, document: AssetDocument) -> io::Result<Self> {
        let bytes = document.to_bytes().map_err(io::Error::other)?;
        let key = store.put(&bytes, "json")?;
        Ok(Self { key, document })
    }

    /// The error code of an asset that could not be produced.
    pub(crate) fn error(&self) -> Option<String> {
        self.document.error.as_ref().map(|e| e.code.clone())
    }
}

/// The output of a derivation that ran, and the error code of a failed asset.
pub(crate) fn outcome(stored: Option<&Stored>) -> (Derived, Option<String>) {
    match stored {
        Some(stored) => (Derived::Document(stored.key.clone()), stored.error()),
        None => (Derived::Nothing, None),
    }
}

/// Evaluates one derivation on the calling thread and returns the document of its asset, if it
/// has one.
pub(crate) fn derive(
    plan: &Planned<'_>,
    version: &Version<'_>,
    files: Files<'_>,
    fusion: Option<&dyn Fusion>,
) -> io::Result<Option<Stored>> {
    let cost = plan.cost(version);
    Scheduler::new(Limits::threads(1))
        .run(&[Need::default()], |_, slots| {
            derive_within(plan, cost, slots, version, files, fusion)
        })
        .pop()
        .expect("the derivation ran")
}

/// Evaluates one derivation as [`derive`] does, within the CPU slots of a running job.
///
/// `cost` is the estimate the derivation was scheduled with, which the convert span reports
/// beside the decoded size. The conversion and the encoding of the members share one CPU slot of
/// `slots`, so a derivation never waits for a slot while it holds decoded members;
/// [`CostClass::Wide`] work encodes on two slots, which it waits for after converting. The files
/// are stored in `files` once the slots are returned.
pub(crate) fn derive_within(
    plan: &Planned<'_>,
    cost: Cost,
    slots: &Slots<'_>,
    version: &Version<'_>,
    files: Files<'_>,
    fusion: Option<&dyn Fusion>,
) -> io::Result<Option<Stored>> {
    let reads = Reads::new(version, &plan.closure, Some(version.all_streams()));
    let cpu = slots.hold(1);
    let converted = convert(plan, cost, &reads, files.recorder);
    let (objects, resources) = reads.finish();
    let Some(asset) = converted.transpose() else {
        return Ok(None);
    };
    // The slot is returned when the members are encoded, or at once when there are none to
    // encode.
    let encoded = match &asset {
        Ok(asset) => {
            let _cpu = if cost.class == CostClass::Wide {
                drop(cpu);
                slots.hold(2)
            } else {
                cpu
            };
            isolated(|| files.encode(plan.converter, &plan.name, asset)).unwrap_or_else(
                |message| {
                    Ok(Err(AssetError {
                        code: PANICKED.to_owned(),
                        message,
                    }))
                },
            )?
        }
        Err(e) => {
            drop(cpu);
            Err(asset_error(e))
        }
    };
    let (members, error) = match encoded {
        Ok(encoded) => (files.store(&plan.name, encoded)?, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let document = build_document(plan, version, &objects, &resources, members, error);
    let stored = Stored::put(files.store, document)?;
    if let (Some(fusion), Ok(asset)) = (fusion, &asset) {
        // A panic makes nothing more: the view that the extra files are for makes them again
        // when it is published, and reports the asset there.
        slots
            .cpu(1, || isolated(|| fusion.extra(asset, &stored.document)))
            .unwrap_or(Ok(()))?;
    }
    Ok(Some(stored))
}

/// Converts the root of `plan` and reports the conversion to `recorder`.
fn convert(
    plan: &Planned<'_>,
    cost: Cost,
    reads: &Reads<'_>,
    recorder: &dyn Recorder,
) -> Result<Option<Asset>, ConvertError> {
    let mut span = recorder.span("convert", plan.converter.name(), &plan.name);
    let converted = isolated(|| plan.converter.convert(plan.closure[0], reads))
        .unwrap_or_else(|message| Err(ConvertError::new(PANICKED, message)));
    if let Ok(Some(asset)) = &converted {
        span.memory(cost.memory, decoded_size(asset));
    }
    converted
}

/// Stores the document of an asset that could not be produced.
pub(crate) fn document(
    plan: &Planned<'_>,
    version: &Version<'_>,
    store: &dyn Store,
    members: Vec<MemberFile>,
    error: AssetError,
) -> io::Result<Stored> {
    let root = &plan.closure[..1];
    let document = build_document(plan, version, root, &[], members, Some(error));
    Stored::put(store, document)
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
    pub(crate) outputs: Vec<Option<Produced>>,
    /// The derivations that ran.
    pub(crate) converted: usize,
    /// The derivations whose output already existed.
    pub(crate) reused: usize,
}

/// An asset of a run.
pub(crate) struct Produced {
    pub(crate) output: Output,
    /// The document of the asset, when its derivation ran in this run.
    pub(crate) document: Option<AssetDocument>,
}

/// Where the derivations of a run go: the store and the records they are written to, where they
/// are reported, and the scheduler whose CPU slots and memory budget all of them share, however
/// many versions they come from.
pub(crate) struct Runner<'a> {
    store: &'a dyn Store,
    derivations: &'a dyn Derivations,
    recorder: &'a dyn Recorder,
    /// Further files of each converted asset, outside plain mode.
    fusion: Option<&'a dyn Fusion>,
    plain: bool,
    pub(crate) scheduler: Scheduler,
}

impl<'a> Runner<'a> {
    /// A runner within `options`. In plain mode it has one CPU slot and makes no further files.
    pub(crate) fn new(
        store: &'a dyn Store,
        derivations: &'a dyn Derivations,
        recorder: &'a dyn Recorder,
        fusion: Option<&'a dyn Fusion>,
        options: &Options,
    ) -> Self {
        Self {
            store,
            derivations,
            recorder,
            fusion: fusion.filter(|_| !options.plain),
            plain: options.plain,
            scheduler: Scheduler::new(Limits {
                cpu: if options.plain { 1 } else { options.threads },
                memory: options.memory,
                resident: options.resident,
            }),
        }
    }

    /// Runs the derivations of `plans` whose output is not recorded yet, records the new outputs
    /// and reports the asset of each derivation, in order. An asset that cannot be produced is
    /// reported with its error and not recorded; only a failure to read or write the store or
    /// the records fails the run. Other calls may run at the same time, from other threads.
    pub(crate) fn run(&self, plans: &[Planned<'_>], version: &Version<'_>) -> io::Result<Ran> {
        let mut outputs = Vec::with_capacity(plans.len());
        let mut pending = Vec::new();
        for (i, recorded) in self.recorded(plans, version).into_iter().enumerate() {
            outputs.push(match recorded? {
                Ok(Derived::Document(key)) => Some(produced(&plans[i], version, key, None)),
                Ok(Derived::Nothing) => None,
                Err(cost) => {
                    pending.push((i, cost));
                    None
                }
            });
        }
        for derived in self.derive(plans, &pending, version) {
            let (i, stored) = derived?;
            outputs[i] = stored
                .map(|stored| produced(&plans[i], version, stored.key, Some(stored.document)));
        }
        if !self.plain {
            self.derivations.flush()?;
        }
        Ok(Ran {
            outputs,
            converted: pending.len(),
            reused: plans.len() - pending.len(),
        })
    }

    /// The recorded output of each of `plans`, or the cost of deriving it. Plain mode records
    /// nothing, so every plan is derived.
    fn recorded(
        &self,
        plans: &[Planned<'_>],
        version: &Version<'_>,
    ) -> Vec<io::Result<Result<Derived, Cost>>> {
        self.scheduler
            .run(&vec![Need::default(); plans.len()], |i, slots| {
                slots.cpu(1, || {
                    let plan = &plans[i];
                    let recorded = if self.plain {
                        None
                    } else {
                        self.derivations.get(&plan.fingerprint)?
                    };
                    Ok(recorded.ok_or_else(|| plan.cost(version)))
                })
            })
    }

    /// Derives the `pending` plans, each `(plan, cost)`, grouped by the bundles they read, and
    /// records each output that is not a failure. Returns each plan with what it stored.
    fn derive(
        &self,
        plans: &[Planned<'_>],
        pending: &[(usize, Cost)],
        version: &Version<'_>,
    ) -> Vec<io::Result<(usize, Option<Stored>)>> {
        let order = order(plans, pending);
        let needs: Vec<Need> = order
            .iter()
            .map(|&k| Need {
                memory: pending[k].1.memory,
            })
            .collect();
        let files = Files {
            store: self.store,
            records: (!self.plain).then_some(self.derivations),
            recorder: self.recorder,
        };
        self.scheduler.run(&needs, |j, slots| {
            let (i, cost) = pending[order[j]];
            let plan = &plans[i];
            let stored = derive_within(plan, cost, slots, version, files, self.fusion)?;
            // A failed asset has no output, so the next run evaluates it again.
            if let (derived, None) = outcome(stored.as_ref())
                && !self.plain
            {
                self.derivations.put(plan.fingerprint, derived)?;
            }
            Ok((i, stored))
        })
    }
}

/// The asset of the derivation of `plan`: its document is the one stored now, or none for an
/// output that an earlier run recorded.
fn produced(
    plan: &Planned<'_>,
    version: &Version<'_>,
    document: Key,
    stored: Option<AssetDocument>,
) -> Produced {
    let bundle = &version.index().bundles()[plan.closure[0].bundle];
    Produced {
        output: Output {
            name: plan.name.clone(),
            bundle: bundle.name.clone(),
            form: plan.converter.name(),
            document,
            error: stored
                .as_ref()
                .and_then(|stored| Some(stored.error.as_ref()?.code.clone())),
        },
        document: stored,
    }
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
    fn fingerprints_follow_stream_slices_and_lossy_encoders() {
        use sekai_convert::Encoders;
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
        // A lossless encoder leaves the fingerprint as it is; the video encoder is part of it.
        let declaring = |encoders| {
            fingerprint(
                &WithEnc(encoders),
                "x",
                &index,
                &[id_text],
                &streams_v1,
                &NoClasses,
            )
        };
        assert_eq!(declaring(Encoders::NONE), declaring(Encoders::AUDIO));
        assert_ne!(declaring(Encoders::NONE), declaring(Encoders::VIDEO));
    }
}
