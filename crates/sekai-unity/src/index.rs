//! The index: what each object is and what it refers to, across every bundle of a version.
//!
//! Indexing reads each object once, skipping arrays that contain no references, and records
//! its class, name, script, references, container paths and content hash. A [`VersionIndex`]
//! then resolves external files to bundles and scripts to class names.

use std::collections::HashMap;

use sha2::{Digest, Sha256};

use crate::class_id::class_name;
use crate::error::{Error, Result};
use crate::layout::Layout;
use crate::read::{Field, ReferenceTypes, Visitor, read_object};
use crate::serialized::{External, ReferencedName, SerializedFile};
use crate::source::ByteSource;
use crate::value::{Value, read_value};

const MONO_SCRIPT: i32 = 115;

/// The compiled layouts of one serialized file.
#[derive(Debug)]
pub struct FileLayouts {
    types: Vec<Result<Layout>>,
    referenced: Vec<(ReferencedName, Layout)>,
}

impl FileLayouts {
    /// Compiles the type trees of a file. Types without a tree get [`Error::NoLayout`].
    pub fn new(file: &SerializedFile) -> Self {
        let types = file
            .types
            .iter()
            .map(|t| match &t.tree {
                Some(tree) => Layout::compile(tree),
                None => Err(Error::NoLayout {
                    class_id: t.class_id,
                }),
            })
            .collect();
        let referenced = file
            .referenced_types
            .iter()
            .filter_map(|t| {
                let layout = Layout::compile(t.tree.as_ref()?).ok()?;
                Some((t.referenced.clone()?, layout))
            })
            .collect();
        FileLayouts { types, referenced }
    }

    /// The layout of the type at `type_index`.
    pub fn get(&self, type_index: usize) -> Result<&Layout> {
        self.types[type_index].as_ref().map_err(Clone::clone)
    }
}

impl ReferenceTypes for FileLayouts {
    fn layout(&self, name: &ReferencedName) -> Option<&Layout> {
        self.referenced
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, l)| l)
    }
}

/// An open serialized file: its parsed metadata, its bytes and its layouts.
#[derive(Clone, Copy)]
pub struct OpenFile<'a> {
    /// The entry path of the file in its bundle.
    pub path: &'a str,
    /// The parsed file.
    pub file: &'a SerializedFile,
    /// The file's bytes.
    pub source: &'a dyn ByteSource,
    /// The file's layouts.
    pub layouts: &'a FileLayouts,
}

impl OpenFile<'_> {
    /// Reads object `index` of the file into `visitor`.
    pub fn read(&self, index: usize, visitor: &mut dyn Visitor) -> Result<()> {
        let object = &self.file.objects[index];
        let layout = self.layouts.get(object.type_index)?;
        let data = self.file.object_bytes(self.source, object)?;
        read_object(
            layout,
            &data,
            self.file.header.endian,
            self.layouts,
            visitor,
        )
    }

    /// Reads object `index` of the file into a value tree.
    pub fn value(&self, index: usize) -> Result<Value> {
        let object = &self.file.objects[index];
        let layout = self.layouts.get(object.type_index)?;
        let data = self.file.object_bytes(self.source, object)?;
        read_value(layout, &data, self.file.header.endian, self.layouts)
    }
}

/// An object of a version: its bundle and file, by their positions in the [`VersionIndex`],
/// and its path ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId {
    /// The bundle.
    pub bundle: usize,
    /// The file in the bundle.
    pub file: usize,
    /// The path ID.
    pub path_id: i64,
}

/// A reference as stored: a file index and a path ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PPtr {
    /// 0 for the same file, `n` for `externals[n - 1]`.
    pub file_id: i32,
    /// The path ID.
    pub path_id: i64,
}

/// What the index records about one object.
#[derive(Debug, Clone)]
pub struct ObjectIndex {
    /// The path ID.
    pub path_id: i64,
    /// The class ID.
    pub class_id: i32,
    /// The class name: the root type of the layout, or the engine's name for the class ID.
    pub class: String,
    /// The top-level `m_Name`, when present.
    pub name: Option<String>,
    /// The top-level `m_Script` reference, when present and not null.
    pub script: Option<PPtr>,
    /// For a `MonoScript`: `"<assembly>/<namespace>.<class>"`.
    pub script_name: Option<String>,
    /// Every non-null reference, in stored order.
    pub references: Vec<PPtr>,
    /// The size of the stored bytes.
    pub size: u32,
    /// The SHA-256 of the stored bytes.
    pub sha256: [u8; 32],
    /// Why the object could not be read exactly, if it could not.
    pub error: Option<Error>,
}

/// The index of one serialized file.
#[derive(Debug, Clone)]
pub struct FileIndex {
    /// The entry path of the file.
    pub path: String,
    /// The file's external files.
    pub externals: Vec<External>,
    /// One record per object, in object table order.
    pub objects: Vec<ObjectIndex>,
    /// The container of the file's `AssetBundle` object: each path and the object it names.
    pub container: Vec<(String, PPtr)>,
}

/// Indexes every object of a file.
///
/// An object that cannot be read exactly is recorded with its error. Only a failure to read the
/// file's bytes fails the whole file.
pub fn index_file(open: &OpenFile<'_>) -> Result<FileIndex> {
    let mut objects = Vec::with_capacity(open.file.objects.len());
    let mut container = Vec::new();
    for (i, object) in open.file.objects.iter().enumerate() {
        let data = open.file.object_bytes(open.source, object)?;
        let sha256 = Sha256::digest(&*data).into();
        let mut v = IndexVisitor::default();
        let (class, error) = match open.layouts.get(object.type_index) {
            Ok(layout) => (layout.root_type().to_string(), open.read(i, &mut v).err()),
            Err(e) => (
                class_name(object.class_id).unwrap_or("").to_string(),
                Some(e),
            ),
        };
        if error.is_some() {
            v = IndexVisitor::default();
        }
        let script_name = (object.class_id == MONO_SCRIPT)
            .then(|| v.script_name())
            .flatten();
        container.append(&mut v.container);
        objects.push(ObjectIndex {
            path_id: object.path_id,
            class_id: object.class_id,
            class,
            name: v.name,
            script: v.script,
            script_name,
            references: v.references,
            size: object.size,
            sha256,
            error,
        });
    }
    Ok(FileIndex {
        path: open.path.to_string(),
        externals: open.file.externals.clone(),
        objects,
        container,
    })
}

#[derive(Default)]
struct IndexVisitor {
    depth: usize,
    /// For each open array: whether it is the top-level `m_Container`.
    arrays: Vec<bool>,
    container_path: Option<String>,
    name: Option<String>,
    script: Option<PPtr>,
    strings: HashMap<&'static str, String>,
    references: Vec<PPtr>,
    container: Vec<(String, PPtr)>,
}

impl IndexVisitor {
    fn in_container(&self) -> bool {
        self.arrays.first() == Some(&true)
    }

    fn script_name(&self) -> Option<String> {
        let class = self.strings.get("m_ClassName")?;
        let namespace = self.strings.get("m_Namespace")?;
        let assembly = self.strings.get("m_AssemblyName")?;
        let assembly = assembly.strip_suffix(".dll").unwrap_or(assembly);
        Some(if namespace.is_empty() {
            format!("{assembly}/{class}")
        } else {
            format!("{assembly}/{namespace}.{class}")
        })
    }
}

impl Visitor for IndexVisitor {
    fn begin_struct(&mut self, _field: &Field<'_>) -> Result<()> {
        self.depth += 1;
        Ok(())
    }

    fn end_struct(&mut self) -> Result<()> {
        self.depth -= 1;
        Ok(())
    }

    fn string(&mut self, field: &Field<'_>, bytes: &[u8]) -> Result<()> {
        if self.depth == 1 && self.arrays.is_empty() {
            let key = match field.name {
                "m_Name" => {
                    self.name = Some(String::from_utf8_lossy(bytes).into_owned());
                    return Ok(());
                }
                "m_ClassName" => "m_ClassName",
                "m_Namespace" => "m_Namespace",
                "m_AssemblyName" => "m_AssemblyName",
                _ => return Ok(()),
            };
            self.strings
                .insert(key, String::from_utf8_lossy(bytes).into_owned());
        } else if self.in_container() && field.name == "first" {
            self.container_path = Some(String::from_utf8_lossy(bytes).into_owned());
        }
        Ok(())
    }

    fn begin_array(&mut self, field: &Field<'_>, _len: usize, _map: bool) -> Result<bool> {
        let container = self.depth == 1 && self.arrays.is_empty() && field.name == "m_Container";
        self.arrays.push(container);
        Ok(field.has_references)
    }

    fn end_array(&mut self) -> Result<()> {
        self.arrays.pop();
        Ok(())
    }

    fn pptr(&mut self, field: &Field<'_>, file_id: i32, path_id: i64) -> Result<()> {
        if file_id == 0 && path_id == 0 {
            return Ok(());
        }
        let p = PPtr { file_id, path_id };
        self.references.push(p);
        if self.depth == 1 && self.arrays.is_empty() && field.name == "m_Script" {
            self.script = Some(p);
        }
        if self.in_container()
            && field.name == "asset"
            && let Some(path) = self.container_path.take()
        {
            self.container.push((path, p));
        }
        Ok(())
    }
}

/// The index of one bundle.
#[derive(Debug, Clone)]
pub struct BundleIndex {
    /// The bundle name.
    pub name: String,
    /// The index of each serialized file, in directory order.
    pub files: Vec<FileIndex>,
}

/// Where a reference points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target<'a> {
    /// Null.
    Null,
    /// An object in a file of an indexed bundle.
    Object {
        /// The bundle.
        bundle: &'a str,
        /// The file in the bundle.
        file: &'a str,
        /// The path ID.
        path_id: i64,
    },
    /// An object in an external file that is in no indexed bundle.
    External {
        /// The external path.
        path: &'a str,
        /// The path ID.
        path_id: i64,
    },
}

/// The object a reference points to, by [`VersionIndex::resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved<'a> {
    /// Null.
    Null,
    /// An object of an indexed file. The file may still lack an object with that path ID.
    Object(ObjectId),
    /// An object in an external file that is in no indexed bundle.
    Missing {
        /// The external path.
        external: &'a str,
        /// The path ID.
        path_id: i64,
    },
}

/// The indexes of every bundle of a version, with references resolved across bundles.
#[derive(Debug)]
pub struct VersionIndex {
    bundles: Vec<BundleIndex>,
    /// File name → (bundle, file). The first bundle in name order wins.
    files: HashMap<String, (usize, usize)>,
    /// (bundle, file, path ID) → object index.
    objects: HashMap<(usize, usize, i64), usize>,
    /// (bundle, file, path ID) → the container paths that name the object.
    containers: HashMap<(usize, usize, i64), Vec<String>>,
}

impl VersionIndex {
    /// Builds the version index. Bundles are sorted by name.
    pub fn new(mut bundles: Vec<BundleIndex>) -> Self {
        bundles.sort_by(|a, b| a.name.cmp(&b.name));
        let mut files = HashMap::new();
        let mut objects = HashMap::new();
        for (b, bundle) in bundles.iter().enumerate() {
            for (f, file) in bundle.files.iter().enumerate() {
                files.entry(file.path.clone()).or_insert((b, f));
                for (o, object) in file.objects.iter().enumerate() {
                    objects.insert((b, f, object.path_id), o);
                }
            }
        }
        let mut index = VersionIndex {
            bundles,
            files,
            objects,
            containers: HashMap::new(),
        };
        let mut containers: HashMap<_, Vec<String>> = HashMap::new();
        for (b, bundle) in index.bundles.iter().enumerate() {
            for (f, file) in bundle.files.iter().enumerate() {
                for (path, p) in &file.container {
                    let target = if p.file_id == 0 {
                        Some((b, f))
                    } else {
                        index.external(b, f, p.file_id as usize)
                    };
                    // Container paths name objects of their own bundle.
                    if let Some((tb, tf)) = target.filter(|&(tb, _)| tb == b) {
                        containers
                            .entry((tb, tf, p.path_id))
                            .or_default()
                            .push(path.clone());
                    }
                }
            }
        }
        index.containers = containers;
        index
    }

    /// The bundles, in name order.
    pub fn bundles(&self) -> &[BundleIndex] {
        &self.bundles
    }

    /// The (bundle, file) that external `index` (counted from 1) of a file resolves to.
    pub fn external(&self, bundle: usize, file: usize, index: usize) -> Option<(usize, usize)> {
        let external = self.bundles[bundle].files[file]
            .externals
            .get(index.checked_sub(1)?)?;
        let name = external.path.rsplit('/').next()?;
        self.files.get(name).copied()
    }

    /// The file a reference stored in `bundle`/`file` points into: an indexed file, or the
    /// path of an external file that is in no indexed bundle.
    fn file_of(
        &self,
        bundle: usize,
        file: usize,
        file_id: i32,
    ) -> Result<std::result::Result<(usize, usize), &str>> {
        if file_id == 0 {
            return Ok(Ok((bundle, file)));
        }
        let index = usize::try_from(file_id).unwrap_or(usize::MAX);
        let externals = &self.bundles[bundle].files[file].externals;
        let Some(external) = index.checked_sub(1).and_then(|i| externals.get(i)) else {
            return Err(Error::malformed(
                "reference",
                format!("file index {file_id} is not in the list of external files"),
            ));
        };
        Ok(self
            .external(bundle, file, index)
            .ok_or(external.path.as_str()))
    }

    /// Where a reference stored in a file points.
    pub fn target(&self, bundle: usize, file: usize, p: PPtr) -> Result<Target<'_>> {
        if p.file_id == 0 && p.path_id == 0 {
            return Ok(Target::Null);
        }
        Ok(match self.file_of(bundle, file, p.file_id)? {
            Ok((b, f)) => Target::Object {
                bundle: &self.bundles[b].name,
                file: &self.bundles[b].files[f].path,
                path_id: p.path_id,
            },
            Err(path) => Target::External {
                path,
                path_id: p.path_id,
            },
        })
    }

    /// The object a reference stored in object `from` points to. `Ok(None)` is a null
    /// reference; a reference into a file that no indexed bundle contains is
    /// [`Resolved::Missing`].
    pub fn resolve(&self, from: ObjectId, p: PPtr) -> Result<Resolved<'_>> {
        if p.file_id == 0 && p.path_id == 0 {
            return Ok(Resolved::Null);
        }
        Ok(match self.file_of(from.bundle, from.file, p.file_id)? {
            Ok((bundle, file)) => Resolved::Object(ObjectId {
                bundle,
                file,
                path_id: p.path_id,
            }),
            Err(path) => Resolved::Missing {
                external: path,
                path_id: p.path_id,
            },
        })
    }

    /// The position of an object in its file's object table, if the file contains it.
    pub fn position(&self, id: ObjectId) -> Option<usize> {
        self.objects.get(&(id.bundle, id.file, id.path_id)).copied()
    }

    /// The script name of an object that refers to a script: `None` when it has no script,
    /// `Some(None)` when the script does not resolve.
    pub fn script(&self, bundle: usize, file: usize, object: usize) -> Option<Option<&str>> {
        let p = self.bundles[bundle].files[file].objects[object].script?;
        let (b, f) = if p.file_id == 0 {
            (bundle, file)
        } else {
            match self.external(bundle, file, p.file_id as usize) {
                Some(found) => found,
                None => return Some(None),
            }
        };
        Some(
            self.objects
                .get(&(b, f, p.path_id))
                .and_then(|&o| self.bundles[b].files[f].objects[o].script_name.as_deref()),
        )
    }

    /// The container paths of a bundle that name an object, in container order.
    pub fn container_paths(&self, bundle: usize, file: usize, path_id: i64) -> &[String] {
        self.containers
            .get(&(bundle, file, path_id))
            .map_or(&[], Vec::as_slice)
    }
}
