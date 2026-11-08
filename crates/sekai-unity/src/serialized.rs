//! Serialized files: the type table, the object table and the list of external files.
//!
//! Format versions 17 to 23 are read, which covers every engine release since 2017.

use std::sync::Arc;

use crate::endian::{Endian, Reader};
use crate::error::{Error, Result};
use crate::source::{ByteSource, Chunk};
use crate::typetree::TypeTree;

const MIN_VERSION: u32 = 17;
const MAX_VERSION: u32 = 23;
/// The class ID of `MonoBehaviour`, whose types also store the script ID.
const MONO_BEHAVIOUR: i32 = 114;

/// The header of a serialized file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileHeader {
    /// The size of the metadata.
    pub metadata_size: u64,
    /// The size of the file.
    pub file_size: u64,
    /// The format version.
    pub version: u32,
    /// The offset of the object data.
    pub data_offset: u64,
    /// The byte order of the metadata and of every object.
    pub endian: Endian,
}

/// One entry of the type table, or of the table of types referenced by `[SerializeReference]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedType {
    /// The class ID.
    pub class_id: i32,
    /// Whether the type was stripped from the build.
    pub stripped: bool,
    /// The index of the script in the file's script table, or -1.
    pub script_type_index: i16,
    /// The script ID, for script types.
    pub script_id: Option<[u8; 16]>,
    /// The hash of the type.
    pub old_type_hash: [u8; 16],
    /// The type tree, when the file stores type trees.
    pub tree: Option<Arc<TypeTree>>,
    /// For referenced types: the class name, namespace and assembly name.
    pub referenced: Option<ReferencedName>,
    /// The indices of referenced types this type depends on.
    pub dependencies: Vec<i32>,
}

/// The name of a type referenced by `[SerializeReference]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ReferencedName {
    /// The class name.
    pub class: String,
    /// The namespace.
    pub namespace: String,
    /// The assembly name.
    pub assembly: String,
}

/// One entry of the object table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectInfo {
    /// The path ID.
    pub path_id: i64,
    /// The offset of the object's bytes in the file.
    pub offset: u64,
    /// The size of the object's bytes.
    pub size: u32,
    /// The index of the object's type in [`SerializedFile::types`].
    pub type_index: usize,
    /// The class ID of the object's type.
    pub class_id: i32,
}

/// One entry of the script table: the object that holds a script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptRef {
    /// The file index, as in a PPtr.
    pub file_index: i32,
    /// The path ID.
    pub path_id: i64,
}

/// One external file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct External {
    /// The path of the external file, such as `archive:/CAB-…/CAB-…`.
    pub path: String,
    /// The GUID of the external asset, zero for files of other bundles.
    pub guid: [u8; 16],
    /// The kind of the external asset.
    pub kind: i32,
}

/// A parsed serialized file. Object bytes are read separately with
/// [`SerializedFile::object_bytes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedFile {
    /// The header.
    pub header: FileHeader,
    /// The engine version string, as stored.
    pub unity_version: String,
    /// The target platform number.
    pub platform: i32,
    /// Whether the file stores type trees.
    pub type_trees: bool,
    /// The type table.
    pub types: Vec<SerializedType>,
    /// The object table, in stored order.
    pub objects: Vec<ObjectInfo>,
    /// The script table.
    pub scripts: Vec<ScriptRef>,
    /// The external files; PPtr file index `n` refers to `externals[n - 1]`.
    pub externals: Vec<External>,
    /// The types referenced by `[SerializeReference]` fields.
    pub referenced_types: Vec<SerializedType>,
    /// The user information string.
    pub user_information: String,
}

impl SerializedFile {
    /// Reads the header and metadata of a serialized file.
    pub fn parse(source: &dyn ByteSource) -> Result<Self> {
        let head = source.read(0, source.len().min(48) as usize)?;
        let mut r = Reader::new(&head, Endian::Big, "serialized file header");
        let metadata_size = r.u32()?;
        let file_size = r.u32()?;
        let version = r.u32()?;
        let data_offset = r.u32()?;
        if !(MIN_VERSION..=MAX_VERSION).contains(&version) {
            return Err(Error::Unsupported {
                what: "serialized file version",
                value: version.into(),
            });
        }
        let endian = if r.u8()? == 0 {
            Endian::Little
        } else {
            Endian::Big
        };
        r.skip(3)?;
        let mut header = FileHeader {
            metadata_size: metadata_size.into(),
            file_size: file_size.into(),
            version,
            data_offset: data_offset.into(),
            endian,
        };
        if version >= 22 {
            header.metadata_size = r.u32()?.into();
            header.file_size = r.u64()?;
            header.data_offset = r.u64()?;
            r.skip(8)?;
        }
        let metadata_start = r.pos();
        if header.data_offset > source.len() {
            return Err(Error::malformed(
                "serialized file header",
                "object data starts after the end of the file",
            ));
        }

        // The metadata lies between the header and the object data. Alignment inside it is
        // relative to the start of the file, so it is read from offset 0.
        let meta = source.read(0, header.data_offset as usize)?;
        let mut r = Reader::new(&meta, endian, "serialized file metadata");
        r.set_pos(metadata_start)?;

        let unity_version = r.cstring()?;
        let platform = r.i32()?;
        let type_trees = r.u8()? != 0;

        let types = (0..count(&mut r)?)
            .map(|_| read_type(&mut r, version, type_trees, false))
            .collect::<Result<Vec<_>>>()?;

        let object_count = count(&mut r)?;
        let mut objects = Vec::with_capacity(object_count.min(r.remaining() / 20));
        for _ in 0..object_count {
            r.align(4)?;
            let path_id = r.i64()?;
            let start = if version >= 22 {
                r.u64()?
            } else {
                r.u32()?.into()
            };
            let size = r.u32()?;
            let type_index = usize::try_from(r.i32()?)
                .ok()
                .filter(|&i| i < types.len())
                .ok_or_else(|| Error::malformed("object table", "type index out of range"))?;
            let offset = header.data_offset + start;
            if offset + u64::from(size) > source.len() {
                return Err(Error::malformed(
                    "object table",
                    format!("object {path_id} lies outside the file"),
                ));
            }
            objects.push(ObjectInfo {
                path_id,
                offset,
                size,
                type_index,
                class_id: types[type_index].class_id,
            });
        }

        let scripts = (0..count(&mut r)?)
            .map(|_| {
                let file_index = r.i32()?;
                r.align(4)?;
                Ok(ScriptRef {
                    file_index,
                    path_id: r.i64()?,
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let externals = (0..count(&mut r)?)
            .map(|_| {
                r.cstr()?;
                let guid = r.array()?;
                let kind = r.i32()?;
                let path = r.cstring()?;
                Ok(External { path, guid, kind })
            })
            .collect::<Result<Vec<_>>>()?;

        let referenced_types = if version >= 20 {
            (0..count(&mut r)?)
                .map(|_| read_type(&mut r, version, type_trees, true))
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };

        let user_information = r.cstring()?;

        Ok(SerializedFile {
            header,
            unity_version,
            platform,
            type_trees,
            types,
            objects,
            scripts,
            externals,
            referenced_types,
            user_information,
        })
    }

    /// The type of an object.
    pub fn object_type(&self, object: &ObjectInfo) -> &SerializedType {
        &self.types[object.type_index]
    }

    /// The stored bytes of an object.
    pub fn object_bytes<'s>(
        &self,
        source: &'s dyn ByteSource,
        object: &ObjectInfo,
    ) -> Result<Chunk<'s>> {
        source.read(object.offset, object.size as usize)
    }
}

fn count(r: &mut Reader<'_>) -> Result<usize> {
    usize::try_from(r.i32()?)
        .map_err(|_| Error::malformed("serialized file metadata", "negative count"))
}

fn read_type(
    r: &mut Reader<'_>,
    version: u32,
    type_trees: bool,
    referenced: bool,
) -> Result<SerializedType> {
    let class_id = r.i32()?;
    let stripped = r.u8()? != 0;
    let script_type_index = r.i16()?;
    let script_id = if (referenced && script_type_index >= 0) || class_id == MONO_BEHAVIOUR {
        Some(r.array()?)
    } else {
        None
    };
    let old_type_hash = r.array()?;
    let mut tree = None;
    let mut name = None;
    let mut dependencies = Vec::new();
    if type_trees {
        let mut has_tree = true;
        if version >= 23 {
            r.skip(16)?;
            has_tree = r.i32()? != 0;
        }
        if has_tree {
            tree = Some(Arc::new(TypeTree::read_blob(r, version)?));
        }
        if version >= 21 {
            if referenced {
                name = Some(ReferencedName {
                    class: r.cstring()?,
                    namespace: r.cstring()?,
                    assembly: r.cstring()?,
                });
            } else {
                dependencies = (0..count(r)?).map(|_| r.i32()).collect::<Result<_>>()?;
            }
        }
    }
    Ok(SerializedType {
        class_id,
        stripped,
        script_type_index,
        script_id,
        old_type_hash,
        tree,
        referenced: name,
        dependencies,
    })
}
