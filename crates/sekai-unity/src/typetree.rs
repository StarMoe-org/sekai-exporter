//! Type trees: the field layout of a class as a file stores it.

use crate::common_strings::COMMON_STRINGS;
use crate::endian::Reader;
use crate::error::{Error, Result};

/// The meta flag that aligns the stream to four bytes after the field.
pub const ALIGN_BYTES: u32 = 0x4000;

/// One node of a type tree. Nodes are stored in preorder; `level` gives the depth.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeTreeNode {
    /// The version of the type.
    pub version: u16,
    /// The depth of the node; the root has level 0.
    pub level: u8,
    /// Type flags; bit 0 marks the `Array` node of a vector.
    pub type_flags: u8,
    /// The type name, such as `int`, `string`, `vector` or a class name.
    pub type_name: String,
    /// The field name.
    pub name: String,
    /// The stored size, or -1 for variable-size types.
    pub byte_size: i32,
    /// The index of the node in the tree.
    pub index: i32,
    /// Meta flags; see [`ALIGN_BYTES`].
    pub meta_flags: u32,
    /// The hash of the referenced type, for `[SerializeReference]` fields.
    pub ref_type_hash: u64,
}

impl TypeTreeNode {
    /// Whether the stream is aligned to four bytes after this field.
    pub fn aligned(&self) -> bool {
        self.meta_flags & ALIGN_BYTES != 0
    }
}

/// A type tree: the nodes of one class layout in preorder.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TypeTree {
    nodes: Vec<TypeTreeNode>,
}

impl TypeTree {
    /// Creates a tree from nodes in preorder. The first node must have level 0 and every other
    /// node a level between 1 and one more than the level of the node before it.
    pub fn from_nodes(nodes: Vec<TypeTreeNode>) -> Result<Self> {
        let Some(root) = nodes.first() else {
            return Err(Error::invalid_layout("empty type tree"));
        };
        if root.level != 0 {
            return Err(Error::invalid_layout("the root node is not at level 0"));
        }
        for pair in nodes.windows(2) {
            if pair[1].level == 0 || pair[1].level > pair[0].level + 1 {
                return Err(Error::invalid_layout(format!(
                    "node {} skips a level",
                    pair[1].name
                )));
            }
        }
        Ok(TypeTree { nodes })
    }

    /// The nodes in preorder.
    pub fn nodes(&self) -> &[TypeTreeNode] {
        &self.nodes
    }

    /// The type name of the root node: the class name.
    pub fn root_type(&self) -> &str {
        &self.nodes[0].type_name
    }

    /// Reads a tree in the blob format used by serialized file versions 10 and from 12 on.
    pub(crate) fn read_blob(r: &mut Reader<'_>, version: u32) -> Result<Self> {
        if version >= 23 {
            if r.bytes(4)? != b"mhtt" {
                return Err(Error::malformed("type tree", "missing blob signature"));
            }
            r.skip(4)?;
        }
        let count = usize::try_from(r.i32()?)
            .map_err(|_| Error::malformed("type tree", "negative node count"))?;
        let strings_len = usize::try_from(r.i32()?)
            .map_err(|_| Error::malformed("type tree", "negative string buffer size"))?;
        let node_size = if version >= 19 { 32 } else { 24 };
        let records = r.bytes(
            count
                .checked_mul(node_size)
                .ok_or_else(|| Error::malformed("type tree", "node count overflows"))?,
        )?;
        let strings = r.bytes(strings_len)?;

        let mut nr = Reader::new(records, r.endian(), "type tree node");
        let mut nodes = Vec::with_capacity(count);
        for _ in 0..count {
            let node_version = nr.u16()?;
            let level = nr.u8()?;
            let type_flags = nr.u8()?;
            let type_name = blob_string(strings, nr.u32()?)?;
            let name = blob_string(strings, nr.u32()?)?;
            let byte_size = nr.i32()?;
            let index = nr.i32()?;
            let meta_flags = nr.u32()?;
            let ref_type_hash = if version >= 19 { nr.u64()? } else { 0 };
            nodes.push(TypeTreeNode {
                version: node_version,
                level,
                type_flags,
                type_name,
                name,
                byte_size,
                index,
                meta_flags,
                ref_type_hash,
            });
        }
        TypeTree::from_nodes(nodes)
    }
}

/// Resolves a string offset: into the engine's common strings when the high bit is set, into
/// the tree's own buffer otherwise.
fn blob_string(local: &[u8], offset: u32) -> Result<String> {
    let (buffer, at) = if offset & 0x8000_0000 != 0 {
        (COMMON_STRINGS.as_bytes(), (offset & 0x7fff_ffff) as usize)
    } else {
        (local, offset as usize)
    };
    let rest = buffer.get(at..).unwrap_or_default();
    let end = rest
        .iter()
        .position(|&b| b == 0)
        .ok_or_else(|| Error::malformed("type tree", format!("no string at offset {offset:#x}")))?;
    String::from_utf8(rest[..end].to_vec())
        .map_err(|_| Error::malformed("type tree", "a name is not UTF-8"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_strings_start_at_name_boundaries() {
        assert_eq!(blob_string(&[], 0x8000_0000).unwrap(), "AABB");
        assert_eq!(blob_string(&[], 0x8000_0005).unwrap(), "AnimationClip");
        assert!(blob_string(&[], 0x8000_ffff).is_err());
        assert_eq!(blob_string(b"x\0yz\0", 2).unwrap(), "yz");
    }
}
