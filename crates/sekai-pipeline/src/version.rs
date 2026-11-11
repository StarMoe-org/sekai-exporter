//! The bundles of one version, opened and indexed, as the [`Context`] converters read through.

use std::collections::{BTreeMap, HashMap};
use std::io;
use std::sync::{Arc, Mutex};

use sekai_catalog::{BundleIndexes, NoBundleIndexes};
use sekai_convert::{Chunk, Context, ConvertError, EngineObject, Object, ObjectId, PPtr};
use sekai_unity::index::{
    BundleIndex, FileLayouts, ObjectIndex, OpenFile, Resolved, VersionIndex, index_file,
};
use sekai_unity::{
    BlockCache, Bundle, ByteSource, ClassLayouts, Field, NoClasses, Scalar, SerializedFile,
    Visitor, open_files,
};
use sha2::{Digest, Sha256};

use crate::eval::parallel;
use crate::index_record::{IndexRecord, StreamDigest};

/// The estimated memory of the bundles that stay parsed, in bytes.
const PARSED_BYTES: u64 = 512 << 20;

/// The estimated memory of a parsed serialized file per byte of its metadata: the type tree, the
/// object table and the compiled layouts.
const PARSED_PER_METADATA_BYTE: u64 = 4;

/// One bundle: its container and its serialized files.
struct Opened {
    bundle: Bundle,
    /// Each serialized file with its entry index, in directory order.
    files: Vec<(usize, SerializedFile, FileLayouts)>,
}

impl Opened {
    fn open(
        source: Arc<dyn ByteSource>,
        id: [u8; 32],
        cache: &dyn BlockCache,
        classes: &dyn ClassLayouts,
    ) -> sekai_unity::Result<Self> {
        let bundle = Bundle::open(source, id)?;
        let files = open_files(&bundle, cache, classes)?;
        Ok(Self { bundle, files })
    }

    /// The estimated memory of the parsed form, in bytes.
    fn bytes(&self) -> u64 {
        let metadata: u64 = self
            .files
            .iter()
            .map(|(_, file, _)| file.header.metadata_size)
            .sum();
        metadata.saturating_mul(PARSED_PER_METADATA_BYTE)
    }

    fn record(&self, cache: &dyn BlockCache) -> sekai_unity::Result<IndexRecord> {
        let mut files = Vec::with_capacity(self.files.len());
        let mut streams = Vec::with_capacity(self.files.len());
        for (entry, file, layouts) in &self.files {
            let source = self.bundle.entry(*entry, cache);
            let open = OpenFile {
                path: &self.bundle.entries()[*entry].path,
                file,
                source: &source,
                layouts,
            };
            let indexed = index_file(&open)?;
            streams.push(self.file_streams(&open, cache));
            files.push(indexed);
        }
        Ok(IndexRecord {
            files,
            resources: self.resource_hashes(cache),
            streams,
        })
    }

    fn file_streams(&self, open: &OpenFile<'_>, cache: &dyn BlockCache) -> Vec<StreamDigest> {
        const STREAM_CLASSES: &[i32] = &[28, 43, 83, 89, 117, 187, 329];
        let mut digests = Vec::new();
        for (idx, object) in open.file.objects.iter().enumerate() {
            if !STREAM_CLASSES.contains(&object.class_id) {
                continue;
            }
            let mut visitor = StreamVisitor::default();
            if open.read(idx, &mut visitor).is_err() {
                continue;
            }
            let Some((file, offset, size)) = visitor.finish() else {
                continue;
            };
            let sha256 = self.hash_resource_slice(&file, offset, size, cache);
            digests.push(StreamDigest {
                path_id: object.path_id,
                file,
                offset,
                size,
                sha256,
            });
        }
        digests.sort_by_key(|d| d.path_id);
        digests
    }

    fn hash_resource_slice(
        &self,
        file: &str,
        offset: u64,
        size: u64,
        cache: &dyn BlockCache,
    ) -> Option<[u8; 32]> {
        let entry = self.bundle.entries().iter().position(|e| e.path == file)?;
        let len = usize::try_from(size).ok()?;
        let source = self.bundle.entry(entry, cache);
        let chunk = source.read(offset, len).ok()?;
        Some(Sha256::digest(&*chunk).into())
    }

    /// The SHA-256 of each resource entry that can be read, in directory order.
    fn resource_hashes(&self, cache: &dyn BlockCache) -> Vec<[u8; 32]> {
        self.bundle
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.is_serialized())
            .filter_map(|(i, entry)| {
                let size = usize::try_from(entry.size).ok()?;
                let source = self.bundle.entry(i, cache);
                let chunk = source.read(0, size).ok()?;
                Some(Sha256::digest(&*chunk).into())
            })
            .collect()
    }
}

#[derive(Default)]
struct StreamVisitor {
    depth: usize,
    in_stream: bool,
    path: Option<String>,
    offset: Option<u64>,
    size: Option<u64>,
}

impl StreamVisitor {
    fn finish(self) -> Option<(String, u64, u64)> {
        let path = self.path.filter(|p| !p.is_empty())?;
        let file = path.rsplit('/').next().unwrap_or(&path).to_owned();
        Some((file, self.offset?, self.size?))
    }
}

impl Visitor for StreamVisitor {
    fn begin_struct(&mut self, field: &Field<'_>) -> sekai_unity::Result<()> {
        self.depth += 1;
        if self.depth == 2 && matches!(field.name, "m_StreamData" | "m_Resource") {
            self.in_stream = true;
        }
        Ok(())
    }

    fn end_struct(&mut self) -> sekai_unity::Result<()> {
        if self.depth == 2 {
            self.in_stream = false;
        }
        self.depth -= 1;
        Ok(())
    }

    fn begin_array(
        &mut self,
        _field: &Field<'_>,
        _len: usize,
        _map: bool,
    ) -> sekai_unity::Result<bool> {
        Ok(false)
    }

    fn string(&mut self, field: &Field<'_>, bytes: &[u8]) -> sekai_unity::Result<()> {
        if self.in_stream && self.depth == 2 && matches!(field.name, "path" | "m_Source") {
            self.path = Some(String::from_utf8_lossy(bytes).into_owned());
        }
        Ok(())
    }

    fn scalar(&mut self, field: &Field<'_>, value: Scalar) -> sekai_unity::Result<()> {
        if !self.in_stream || self.depth != 2 {
            return Ok(());
        }
        let unsigned = match value {
            Scalar::U64(v) => Some(v),
            Scalar::U32(v) => Some(u64::from(v)),
            Scalar::I64(v) => u64::try_from(v).ok(),
            Scalar::I32(v) => u64::try_from(v).ok(),
            _ => None,
        };
        if let Some(v) = unsigned {
            match field.name {
                "offset" | "m_Offset" => self.offset = Some(v),
                "size" | "m_Size" => self.size = Some(v),
                _ => {}
            }
        }
        Ok(())
    }
}

/// The record of a bundle, from `indexes` or else by reading the bundle.
fn record(
    input: &BundleInput,
    cache: &dyn BlockCache,
    classes: &dyn ClassLayouts,
    indexes: &dyn BundleIndexes,
) -> io::Result<sekai_unity::Result<IndexRecord>> {
    if let Some(record) = indexes
        .get(&input.sha256)?
        .as_deref()
        .and_then(IndexRecord::decode)
    {
        return Ok(Ok(record));
    }
    let record = Opened::open(input.source.clone(), input.sha256, cache, classes)
        .and_then(|opened| opened.record(cache));
    if let Some(bytes) = record.as_ref().ok().and_then(IndexRecord::encode) {
        indexes.put(input.sha256, bytes)?;
    }
    Ok(record)
}

/// Each serialized file of a bundle as its path and the paths of its external files.
pub(crate) type BundleFiles = Vec<(String, Vec<String>)>;

/// The serialized files of a bundle: from the stored index of the bundle when `indexes` holds it,
/// and otherwise from the file headers.
pub(crate) fn bundle_files(
    input: &BundleInput,
    cache: &dyn BlockCache,
    indexes: &dyn BundleIndexes,
) -> io::Result<sekai_unity::Result<BundleFiles>> {
    let paths = |externals: Vec<sekai_unity::serialized::External>| {
        externals
            .into_iter()
            .map(|external| external.path)
            .collect()
    };
    if let Some(record) = indexes
        .get(&input.sha256)?
        .as_deref()
        .and_then(IndexRecord::decode)
    {
        let files = record.files.into_iter();
        return Ok(Ok(files
            .map(|file| (file.path, paths(file.externals)))
            .collect()));
    }
    let read = || -> sekai_unity::Result<_> {
        let bundle = Bundle::open(input.source.clone(), input.sha256)?;
        let mut files = Vec::new();
        for (i, entry) in bundle.entries().iter().enumerate() {
            if entry.is_serialized() {
                let file = SerializedFile::parse(&bundle.entry(i, cache))?;
                files.push((entry.path.clone(), paths(file.externals)));
            }
        }
        Ok(files)
    };
    Ok(read())
}

/// The most recently used parsed bundles, by content hash, within an estimate of their memory.
#[derive(Default)]
struct Recent {
    tick: u64,
    /// The estimated memory of the bundles held.
    bytes: u64,
    bundles: HashMap<[u8; 32], (Arc<Opened>, u64, u64)>,
    order: BTreeMap<u64, [u8; 32]>,
}

impl Recent {
    fn get(&mut self, bundle: &[u8; 32]) -> Option<Arc<Opened>> {
        let tick = self.tick;
        self.tick += 1;
        let (opened, used, _) = self.bundles.get_mut(bundle)?;
        self.order.remove(used);
        self.order.insert(tick, *bundle);
        *used = tick;
        Some(opened.clone())
    }

    /// Holds `opened`, then lets go of the least recently used bundles until the rest fit in
    /// `limit`. The newest bundle stays, whatever its size.
    fn insert(&mut self, bundle: [u8; 32], opened: Arc<Opened>, limit: u64) {
        self.remove(&bundle);
        let tick = self.tick;
        self.tick += 1;
        let bytes = opened.bytes();
        self.bundles.insert(bundle, (opened, tick, bytes));
        self.order.insert(tick, bundle);
        self.bytes += bytes;
        while self.bytes > limit && self.bundles.len() > 1 {
            let Some((_, oldest)) = self.order.pop_first() else {
                break;
            };
            if let Some((_, _, bytes)) = self.bundles.remove(&oldest) {
                self.bytes -= bytes;
            }
        }
    }

    fn remove(&mut self, bundle: &[u8; 32]) {
        if let Some((_, used, bytes)) = self.bundles.remove(bundle) {
            self.order.remove(&used);
            self.bytes -= bytes;
        }
    }
}

/// Parsed bundles, which versions over the same bundles can share. A bundle is held by the hash of
/// its bytes, and the least recently used ones are let go once the estimate of their memory
/// exceeds a limit.
pub(crate) struct Parsed {
    limit: u64,
    recent: Mutex<Recent>,
}

impl Parsed {
    /// Holds parsed bundles within an estimated `limit` bytes.
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            limit,
            recent: Mutex::default(),
        }
    }

    fn get(&self, bundle: &[u8; 32]) -> Option<Arc<Opened>> {
        self.recent.lock().unwrap().get(bundle)
    }

    fn insert(&self, bundle: [u8; 32], opened: Arc<Opened>) {
        self.recent
            .lock()
            .unwrap()
            .insert(bundle, opened, self.limit);
    }

    /// Lets go of the bundle whose bytes hash to `bundle`.
    pub(crate) fn forget(&self, bundle: &[u8; 32]) {
        self.recent.lock().unwrap().remove(bundle);
    }

    /// The estimated memory of the bundles held, in bytes.
    #[cfg(test)]
    fn bytes(&self) -> u64 {
        self.recent.lock().unwrap().bytes
    }
}

/// A bundle given to [`Version::open`].
pub struct BundleInput {
    /// The bundle name.
    pub name: String,
    /// The bundle bytes, already deobfuscated.
    pub source: Arc<dyn ByteSource>,
    /// The SHA-256 of the bytes, which keys the bundle's blocks in the block cache.
    pub sha256: [u8; 32],
}

/// A bundle that could not be opened or indexed, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleError {
    /// The bundle name.
    pub name: String,
    /// The error code of the reader.
    pub code: &'static str,
    /// A description for people.
    pub message: String,
}

/// The bundles of one version, indexed, with references resolved across bundles.
///
/// A bundle is parsed when an object or a resource of it is first read, and only the most recently
/// used bundles stay parsed, within an estimate of their memory.
pub struct Version<'c> {
    /// The bytes and hash of each bundle, in the order of [`VersionIndex::bundles`].
    bundles: Vec<(Arc<dyn ByteSource>, [u8; 32])>,
    /// The hashes of the resource entries of each bundle.
    resources: Vec<Vec<[u8; 32]>>,
    /// The streamed resource slices of each serialized file of each bundle.
    streams: Vec<Vec<Vec<StreamDigest>>>,
    parsed: Arc<Parsed>,
    index: VersionIndex,
    cache: &'c dyn BlockCache,
    classes: &'c dyn ClassLayouts,
}

impl<'c> Version<'c> {
    /// Opens and indexes `inputs`. A bundle that cannot be opened or indexed is left out of the
    /// version and returned with its error. A name names one bundle of a version, so bundles
    /// that share a name are all left out, with the error `duplicate_name`.
    pub fn open(inputs: Vec<BundleInput>, cache: &'c dyn BlockCache) -> (Self, Vec<BundleError>) {
        Self::open_indexed(inputs, cache, &NoClasses, &NoBundleIndexes, 1)
            .expect("keeping no indexes cannot fail")
    }

    /// Opens `inputs` as [`Version::open`] does, on `threads` threads, with the layouts in
    /// `classes` for objects stored without one, reading the index of a bundle from `indexes` when
    /// it is there and adding it when it is not. The result does not depend on `threads` or on
    /// what `indexes` holds. Fails only if `indexes` does.
    pub fn open_indexed(
        mut inputs: Vec<BundleInput>,
        cache: &'c dyn BlockCache,
        classes: &'c dyn ClassLayouts,
        indexes: &dyn BundleIndexes,
        threads: usize,
    ) -> io::Result<(Self, Vec<BundleError>)> {
        // The version index orders bundles by name; opening them in that order keeps the two
        // lists aligned.
        inputs.sort_by(|a, b| a.name.cmp(&b.name));
        let shared: Vec<bool> = (0..inputs.len())
            .map(|i| {
                let name = &inputs[i].name;
                (i > 0 && inputs[i - 1].name == *name)
                    || inputs.get(i + 1).is_some_and(|next| next.name == *name)
            })
            .collect();
        let records = parallel(inputs.len(), threads, |i| {
            (!shared[i])
                .then(|| record(&inputs[i], cache, classes, indexes))
                .transpose()
        });
        indexes.flush()?;
        let mut bundles = Vec::new();
        let mut resources = Vec::new();
        let mut streams = Vec::new();
        let mut bundle_indexes = Vec::new();
        let mut errors = Vec::new();
        for (input, record) in inputs.into_iter().zip(records) {
            let Some(record) = record? else {
                errors.push(BundleError {
                    message: format!("more than one bundle is named {}", input.name),
                    name: input.name,
                    code: "duplicate_name",
                });
                continue;
            };
            match record {
                Ok(record) => {
                    bundle_indexes.push(BundleIndex {
                        name: input.name,
                        files: record.files,
                    });
                    resources.push(record.resources);
                    streams.push(record.streams);
                    bundles.push((input.source, input.sha256));
                }
                Err(e) => errors.push(BundleError {
                    name: input.name,
                    code: e.code(),
                    message: e.to_string(),
                }),
            }
        }
        let version = Self {
            bundles,
            resources,
            streams,
            parsed: Arc::new(Parsed::new(PARSED_BYTES)),
            index: VersionIndex::new(bundle_indexes),
            cache,
            classes,
        };
        Ok((version, errors))
    }

    /// Keeps parsed bundles in `parsed`, which other versions may share, instead of in a set of
    /// its own.
    pub(crate) fn sharing(mut self, parsed: Arc<Parsed>) -> Self {
        self.parsed = parsed;
        self
    }

    /// The parsed form of a bundle.
    fn opened(&self, bundle: usize) -> Result<Arc<Opened>, ConvertError> {
        let (source, sha256) = &self.bundles[bundle];
        if let Some(opened) = self.parsed.get(sha256) {
            return Ok(opened);
        }
        let opened = Arc::new(Opened::open(
            source.clone(),
            *sha256,
            self.cache,
            self.classes,
        )?);
        self.parsed.insert(*sha256, opened.clone());
        Ok(opened)
    }

    fn missing(&self, id: ObjectId) -> ConvertError {
        let bundle = &self.index.bundles()[id.bundle];
        ConvertError::new(
            "missing_object",
            format!(
                "object {} of {} in {}, which the file does not contain",
                id.path_id, bundle.files[id.file].path, bundle.name
            ),
        )
    }

    /// The SHA-256 of each resource entry of a bundle that can be read, in directory order.
    pub fn resource_hashes(&self, bundle: usize) -> Vec<[u8; 32]> {
        self.resources[bundle].clone()
    }

    /// The streamed resource slice digests of every bundle of the version.
    pub(crate) fn all_streams(&self) -> &[Vec<Vec<StreamDigest>>] {
        &self.streams
    }

    /// The index of the version.
    pub fn index(&self) -> &VersionIndex {
        &self.index
    }

    /// Every object of the version with its index record and container paths, in
    /// (bundle, file, object table) order.
    pub fn objects(&self) -> impl Iterator<Item = (ObjectId, &ObjectIndex, &[String])> {
        self.index
            .bundles()
            .iter()
            .enumerate()
            .flat_map(move |(b, bundle)| {
                bundle.files.iter().enumerate().flat_map(move |(f, file)| {
                    file.objects.iter().map(move |object| {
                        let id = ObjectId {
                            bundle: b,
                            file: f,
                            path_id: object.path_id,
                        };
                        let paths = self.index.container_paths(b, f, object.path_id);
                        (id, object, paths)
                    })
                })
            })
    }
}

impl Context for Version<'_> {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        let position = self.index.position(id).ok_or_else(|| self.missing(id))?;
        let record = &self.index.bundles()[id.bundle].files[id.file].objects[position];
        if let Some(e) = &record.error {
            return Err(ConvertError::from(e.clone()));
        }
        let opened = self.opened(id.bundle)?;
        let (entry, file, layouts) = &opened.files[id.file];
        let fields = OpenFile {
            path: &opened.bundle.entries()[*entry].path,
            file,
            source: &opened.bundle.entry(*entry, self.cache),
            layouts,
        }
        .value(position)?;
        let script = self
            .index
            .script(id.bundle, id.file, position)
            .map(|s| s.map(str::to_owned))
            .unwrap_or(None);
        Ok(Object {
            id,
            class_id: record.class_id,
            class: record.class.clone(),
            script,
            fields,
        })
    }

    fn resolve(&self, from: ObjectId, reference: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        match self.index.resolve(from, reference)? {
            Resolved::Null => Ok(None),
            Resolved::Object(id) if self.index.position(id).is_some() => Ok(Some(id)),
            Resolved::Object(id) => Err(self.missing(id)),
            Resolved::Missing { external, path_id } => Err(ConvertError::new(
                "missing_object",
                format!("object {path_id} of {external}, which no bundle of the version contains"),
            )),
        }
    }

    fn resource(
        &self,
        from: ObjectId,
        path: &str,
        offset: u64,
        size: u64,
    ) -> Result<Chunk<'_>, ConvertError> {
        let opened = self.opened(from.bundle)?;
        let bundle = &opened.bundle;
        // A stored path has the form `archive:/<file>/<resource file>`; the resource file is an
        // entry of the object's own bundle.
        let name = path.rsplit('/').next().unwrap_or(path);
        let entry = bundle
            .entries()
            .iter()
            .position(|e| e.path == name)
            .ok_or_else(|| {
                ConvertError::new(
                    "missing_resource",
                    format!(
                        "{path} is not an entry of {}",
                        self.index.bundles()[from.bundle].name
                    ),
                )
            })?;
        let size = usize::try_from(size)
            .map_err(|_| ConvertError::new("malformed", format!("a resource of {size} bytes")))?;
        let source = bundle.entry(entry, self.cache);
        let chunk = source.read(offset, size)?;
        // The chunk borrows the entry source made for this read; keep only what outlives it.
        Ok(match chunk {
            Chunk::Borrowed(bytes) => Chunk::Owned(bytes.to_vec()),
            Chunk::Shared { block, start, end } => Chunk::Shared { block, start, end },
            Chunk::Owned(bytes) => Chunk::Owned(bytes),
        })
    }

    fn container(&self, from: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        let mut entries = Vec::new();
        for (file, index) in self.index.bundles()[from.bundle].files.iter().enumerate() {
            let holder = ObjectId {
                bundle: from.bundle,
                file,
                path_id: 0,
            };
            for (path, reference) in &index.container {
                if let Resolved::Object(id) = self.index.resolve(holder, *reference)?
                    && id.bundle == from.bundle
                {
                    entries.push((path.clone(), id));
                }
            }
        }
        Ok(entries)
    }

    fn file_name(&self, from: ObjectId) -> Result<&str, ConvertError> {
        self.index
            .bundles()
            .get(from.bundle)
            .and_then(|bundle| bundle.files.get(from.file))
            .map(|file| file.path.as_str())
            .ok_or_else(|| self.missing(from))
    }

    fn engine_object(
        &self,
        from: ObjectId,
        reference: PPtr,
    ) -> Result<Option<EngineObject>, ConvertError> {
        Ok(match self.index.resolve(from, reference)? {
            Resolved::Missing { external, path_id }
                if ENGINE_FILES
                    .iter()
                    .any(|file| file.eq_ignore_ascii_case(external)) =>
            {
                Some(EngineObject {
                    file: external.to_owned(),
                    path_id,
                })
            }
            _ => None,
        })
    }
}

/// The files of the engine's own resources, which the player holds and no bundle does.
const ENGINE_FILES: &[&str] = &[
    "Library/unity default resources",
    "Resources/unity_builtin_extra",
];

#[cfg(test)]
mod tests {
    use std::path::Path;

    use sekai_unity::NoBlockCache;

    use super::*;

    fn opened(file: &str) -> ([u8; 32], Arc<Opened>) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../sekai-unity/tests/fixtures/lz4")
            .join(file);
        let bytes = std::fs::read(path).unwrap();
        let sha256: [u8; 32] = Sha256::digest(&bytes).into();
        let source: Arc<dyn ByteSource> = Arc::new(Arc::<[u8]>::from(bytes));
        let opened = Opened::open(source, sha256, &NoBlockCache, &NoClasses).unwrap();
        (sha256, Arc::new(opened))
    }

    #[test]
    fn parsed_bundles_stay_within_their_estimated_memory() {
        let (a, first) = opened("fixture_scene");
        let (b, second) = opened("fixture_data");
        let (size_a, size_b) = (first.bytes(), second.bytes());
        assert!(size_a > 0 && size_b > 0);

        let one = Parsed::new(size_a.max(size_b));
        one.insert(a, first.clone());
        one.insert(b, second.clone());
        assert!(one.get(&a).is_none());
        assert!(one.get(&b).is_some());
        assert_eq!(one.bytes(), size_b);

        let both = Parsed::new(size_a + size_b);
        both.insert(a, first.clone());
        both.insert(b, second);
        assert!(both.get(&a).is_some());
        assert_eq!(both.bytes(), size_a + size_b);
        both.forget(&a);
        assert!(both.get(&a).is_none());
        assert_eq!(both.bytes(), size_b);

        let none = Parsed::new(0);
        none.insert(a, first);
        assert!(none.get(&a).is_some(), "the newest bundle stays");
    }
}
