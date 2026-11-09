//! The bundles of one version, opened and indexed, as the [`Context`] converters read through.

use std::sync::Arc;

use sekai_convert::{Chunk, Context, ConvertError, Object, ObjectId, PPtr};
use sekai_unity::index::{
    BundleIndex, FileLayouts, ObjectIndex, OpenFile, Resolved, VersionIndex, index_file,
};
use sekai_unity::{BlockCache, Bundle, ByteSource, SerializedFile};
use sha2::{Digest, Sha256};

/// One bundle: its container and its serialized files.
struct Opened {
    bundle: Bundle,
    /// Each serialized file with its entry index, in directory order.
    files: Vec<(usize, SerializedFile, FileLayouts)>,
}

impl Opened {
    fn open(bytes: Arc<[u8]>, id: [u8; 32], cache: &dyn BlockCache) -> sekai_unity::Result<Self> {
        let source: Arc<dyn ByteSource> = Arc::new(bytes);
        let bundle = Bundle::open(source, id)?;
        let mut files = Vec::new();
        for (i, entry) in bundle.entries().iter().enumerate() {
            if entry.is_serialized() {
                let file = SerializedFile::parse(&bundle.entry(i, cache))?;
                let layouts = FileLayouts::new(&file);
                files.push((i, file, layouts));
            }
        }
        Ok(Self { bundle, files })
    }

    fn index(&self, name: String, cache: &dyn BlockCache) -> sekai_unity::Result<BundleIndex> {
        let files = self
            .files
            .iter()
            .map(|(entry, file, layouts)| {
                index_file(&OpenFile {
                    path: &self.bundle.entries()[*entry].path,
                    file,
                    source: &self.bundle.entry(*entry, cache),
                    layouts,
                })
            })
            .collect::<sekai_unity::Result<_>>()?;
        Ok(BundleIndex { name, files })
    }
}

/// A bundle given to [`Version::open`].
pub struct BundleInput {
    /// The bundle name.
    pub name: String,
    /// The bundle bytes, already deobfuscated.
    pub bytes: Arc<[u8]>,
    /// The SHA-256 of `bytes`, which keys the bundle's blocks in the block cache.
    pub sha256: [u8; 32],
}

/// A bundle that could not be opened or indexed, with the reason.
#[derive(Debug, Clone)]
pub struct BundleError {
    /// The bundle name.
    pub name: String,
    /// The error code of the reader.
    pub code: &'static str,
    /// A description for people.
    pub message: String,
}

/// The bundles of one version, opened and indexed, with references resolved across bundles.
///
/// The bundles stay in memory while the version is open.
pub struct Version<'c> {
    /// In the order of [`VersionIndex::bundles`].
    bundles: Vec<Opened>,
    index: VersionIndex,
    cache: &'c dyn BlockCache,
}

impl<'c> Version<'c> {
    /// Opens and indexes `inputs`. A bundle that cannot be opened or indexed is left out of the
    /// version and returned with its error. A name names one bundle of a version, so bundles
    /// that share a name are all left out, with the error `duplicate_name`.
    pub fn open(
        mut inputs: Vec<BundleInput>,
        cache: &'c dyn BlockCache,
    ) -> (Self, Vec<BundleError>) {
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
        let mut bundles = Vec::new();
        let mut indexes = Vec::new();
        let mut errors = Vec::new();
        for (input, shared) in inputs.into_iter().zip(shared) {
            if shared {
                errors.push(BundleError {
                    message: format!("more than one bundle is named {}", input.name),
                    name: input.name,
                    code: "duplicate_name",
                });
                continue;
            }
            let opened = Opened::open(input.bytes, input.sha256, cache)
                .and_then(|o| Ok((o.index(input.name.clone(), cache)?, o)));
            match opened {
                Ok((index, opened)) => {
                    indexes.push(index);
                    bundles.push(opened);
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
            index: VersionIndex::new(indexes),
            cache,
        };
        (version, errors)
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
        let bundle = &self.bundles[bundle].bundle;
        bundle
            .entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| !entry.is_serialized())
            .filter_map(|(i, entry)| {
                let size = usize::try_from(entry.size).ok()?;
                let source = bundle.entry(i, self.cache);
                let chunk = source.read(0, size).ok()?;
                Some(Sha256::digest(&*chunk).into())
            })
            .collect()
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
        let opened = &self.bundles[id.bundle];
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
        let bundle = &self.bundles[from.bundle].bundle;
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
}
