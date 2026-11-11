//! Layouts: type trees compiled into flat reading programs.
//!
//! A layout is the type tree in preorder with the meaning of each node decided once: which
//! nodes are scalars, strings, byte payloads, arrays, maps, references or structures, which
//! subtrees have a fixed size and which contain references. Every reader in this crate
//! interprets the same layout, so each class layout has one implementation.

use crate::error::{Error, Result};
use crate::typetree::{TypeTree, TypeTreeNode};

/// A fixed-size value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalarKind {
    /// `bool`: one byte.
    Bool,
    /// `SInt8`.
    I8,
    /// `UInt8` and `char`.
    U8,
    /// `SInt16` and `short`.
    I16,
    /// `UInt16` and `unsigned short`.
    U16,
    /// `SInt32` and `int`.
    I32,
    /// `UInt32`, `unsigned int` and `Type*`.
    U32,
    /// `SInt64` and `long long`.
    I64,
    /// `UInt64`, `unsigned long long` and `FileSize`.
    U64,
    /// `float`.
    F32,
    /// `double`.
    F64,
}

impl ScalarKind {
    fn from_type(name: &str) -> Option<Self> {
        Some(match name {
            "bool" => ScalarKind::Bool,
            "SInt8" => ScalarKind::I8,
            "UInt8" | "char" => ScalarKind::U8,
            "SInt16" | "short" => ScalarKind::I16,
            "UInt16" | "unsigned short" => ScalarKind::U16,
            "SInt32" | "int" => ScalarKind::I32,
            "UInt32" | "unsigned int" | "Type*" => ScalarKind::U32,
            "SInt64" | "long long" => ScalarKind::I64,
            "UInt64" | "unsigned long long" | "FileSize" => ScalarKind::U64,
            "float" => ScalarKind::F32,
            "double" => ScalarKind::F64,
            _ => return None,
        })
    }

    /// The stored size in bytes.
    pub fn size(self) -> u32 {
        match self {
            ScalarKind::Bool | ScalarKind::I8 | ScalarKind::U8 => 1,
            ScalarKind::I16 | ScalarKind::U16 => 2,
            ScalarKind::I32 | ScalarKind::U32 | ScalarKind::F32 => 4,
            ScalarKind::I64 | ScalarKind::U64 | ScalarKind::F64 => 8,
        }
    }
}

/// What a layout node reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Scalar(ScalarKind),
    /// An `i32` length and that many bytes, then alignment to four bytes.
    String,
    /// An `i32` length and that many bytes: `TypelessData` and arrays of 8-bit integers.
    Bytes,
    /// An `i32` length and that many elements; the element is the next node.
    Array {
        map: bool,
    },
    /// A file index and a path ID of four or eight bytes.
    PPtr {
        wide_path_id: bool,
    },
    /// The children in order.
    Struct,
    /// A `[SerializeReference]` entry: its `data` child is read with the layout of the type named
    /// by its `type` child.
    ReferencedObject,
    /// The `data` child of a referenced object.
    ReferencedObjectData,
    /// The registry of `[SerializeReference]` entries.
    Registry,
}

#[derive(Debug, Clone)]
pub(crate) struct Op {
    pub(crate) name: String,
    pub(crate) type_name: String,
    pub(crate) kind: Kind,
    /// Align the stream to four bytes after this node.
    pub(crate) align: bool,
    /// The index one past the last op of this node's subtree.
    pub(crate) end: usize,
    /// The size of the subtree when it is the same for every value and contains no alignment.
    pub(crate) fixed_size: Option<u32>,
    /// Whether the subtree contains a reference.
    pub(crate) has_references: bool,
}

/// A compiled class layout.
#[derive(Debug, Clone)]
pub struct Layout {
    pub(crate) ops: Vec<Op>,
}

impl Layout {
    /// Compiles a type tree.
    pub fn compile(tree: &TypeTree) -> Result<Self> {
        let nodes = tree.nodes();
        let mut ops = Vec::with_capacity(nodes.len());
        let next = compile_node(nodes, 0, &mut ops)?;
        if next != nodes.len() {
            return Err(Error::invalid_layout("the tree has more than one root"));
        }
        Ok(Layout { ops })
    }

    /// The class name: the type name of the root.
    pub fn root_type(&self) -> &str {
        &self.ops[0].type_name
    }

    /// Whether the root structure has a field named `name`.
    pub fn has_field(&self, name: &str) -> bool {
        self.children(0).any(|i| self.ops[i].name == name)
    }

    /// The indices of the direct children of op `index`.
    pub(crate) fn children(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        let end = self.ops[index].end;
        let mut i = index + 1;
        std::iter::from_fn(move || {
            (i < end).then(|| {
                let current = i;
                i = self.ops[i].end;
                current
            })
        })
    }
}

/// The index one past the subtree of `nodes[i]`.
fn subtree_end(nodes: &[TypeTreeNode], i: usize) -> usize {
    let level = nodes[i].level;
    nodes[i + 1..]
        .iter()
        .position(|n| n.level <= level)
        .map_or(nodes.len(), |p| i + 1 + p)
}

/// The indices of the direct children of `nodes[i]`.
fn child_nodes(nodes: &[TypeTreeNode], i: usize) -> Vec<usize> {
    let end = subtree_end(nodes, i);
    let mut out = Vec::new();
    let mut c = i + 1;
    while c < end {
        out.push(c);
        c = subtree_end(nodes, c);
    }
    out
}

/// Compiles `nodes[i]` and its subtree, appending ops. Returns the index of the next node.
fn compile_node(nodes: &[TypeTreeNode], i: usize, ops: &mut Vec<Op>) -> Result<usize> {
    let node = &nodes[i];
    let end = subtree_end(nodes, i);
    let children = child_nodes(nodes, i);
    let at = ops.len();
    ops.push(Op {
        name: node.name.clone(),
        type_name: node.type_name.clone(),
        kind: Kind::Struct,
        align: node.aligned(),
        end: 0,
        fixed_size: None,
        has_references: false,
    });

    let invalid =
        |what: &str| Error::invalid_layout(format!("{} {}: {what}", node.type_name, node.name));

    let kind = if let Some(scalar) = ScalarKind::from_type(&node.type_name) {
        Kind::Scalar(scalar)
    } else if node.type_name == "string" {
        Kind::String
    } else if node.type_name == "TypelessData" {
        Kind::Bytes
    } else if node.type_name == "ReferencedObject" {
        Kind::ReferencedObject
    } else if node.type_name == "ReferencedObjectData" {
        Kind::ReferencedObjectData
    } else if let Some(&array) = children.first().filter(|&&c| nodes[c].type_name == "Array") {
        // A vector, map or other container: one `Array` child with a size and an element.
        let parts = child_nodes(nodes, array);
        let [_, element] = parts[..] else {
            return Err(invalid("the array has no size and element"));
        };
        // Elements are stored packed. An alignment flag on the `Array` node or on the element
        // node aligns once, after the last element.
        ops[at].align |= nodes[array].aligned() || nodes[element].aligned();
        if matches!(
            ScalarKind::from_type(&nodes[element].type_name),
            Some(ScalarKind::U8 | ScalarKind::I8)
        ) {
            Kind::Bytes
        } else {
            compile_node(nodes, element, ops)?;
            Kind::Array {
                map: node.type_name == "map",
            }
        }
    } else if node.type_name.starts_with("PPtr<") {
        let path_id = children
            .iter()
            .find(|&&c| nodes[c].name == "m_PathID")
            .ok_or_else(|| invalid("no m_PathID"))?;
        Kind::PPtr {
            wide_path_id: matches!(nodes[*path_id].type_name.as_str(), "SInt64" | "long long"),
        }
    } else if node.type_name == "ManagedReferencesRegistry" {
        for &c in &children {
            compile_node(nodes, c, ops)?;
        }
        Kind::Registry
    } else {
        for &c in &children {
            compile_node(nodes, c, ops)?;
        }
        Kind::Struct
    };
    if matches!(kind, Kind::ReferencedObject) {
        for &c in &children {
            compile_node(nodes, c, ops)?;
        }
    }

    let end_op = ops.len();
    let child_ops = || {
        let mut out = Vec::new();
        let mut c = at + 1;
        while c < end_op {
            out.push(c);
            c = ops[c].end;
        }
        out
    };
    let (fixed_size, has_references) = match kind {
        Kind::Scalar(s) => (Some(s.size()), false),
        Kind::PPtr { wide_path_id } => (Some(if wide_path_id { 12 } else { 8 }), true),
        Kind::String | Kind::Bytes => (None, false),
        Kind::Array { .. } => (None, ops[at + 1].has_references),
        Kind::Struct => {
            let kids = child_ops();
            let fixed = kids.iter().try_fold(0u32, |sum, &c| {
                let op = &ops[c];
                if op.align {
                    None
                } else {
                    op.fixed_size.map(|s| sum + s)
                }
            });
            (fixed, kids.iter().any(|&c| ops[c].has_references))
        }
        Kind::ReferencedObject | Kind::ReferencedObjectData | Kind::Registry => (None, true),
    };
    let op = &mut ops[at];
    op.kind = kind;
    op.end = end_op;
    op.fixed_size = if op.align { None } else { fixed_size };
    op.has_references = has_references;
    Ok(end)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a tree from `(level, type, name, aligned)` rows.
    pub(crate) fn tree(rows: &[(u8, &str, &str, bool)]) -> TypeTree {
        TypeTree::from_nodes(
            rows.iter()
                .enumerate()
                .map(|(i, &(level, type_name, name, aligned))| TypeTreeNode {
                    version: 1,
                    level,
                    type_flags: u8::from(type_name == "Array"),
                    type_name: type_name.into(),
                    name: name.into(),
                    byte_size: -1,
                    index: i as i32,
                    meta_flags: if aligned {
                        crate::typetree::ALIGN_BYTES
                    } else {
                        0
                    },
                    ref_type_hash: 0,
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn classifies_containers() {
        let t = tree(&[
            (0, "Thing", "Base", false),
            (1, "string", "m_Name", false),
            (2, "Array", "Array", true),
            (3, "int", "size", false),
            (3, "char", "data", false),
            (1, "vector", "m_Bytes", false),
            (2, "Array", "Array", true),
            (3, "int", "size", false),
            (3, "UInt8", "data", false),
            (1, "vector", "m_Points", false),
            (2, "Array", "Array", false),
            (3, "int", "size", false),
            (3, "Vector2f", "data", false),
            (4, "float", "x", false),
            (4, "float", "y", false),
            (1, "PPtr<Object>", "m_Target", false),
            (2, "int", "m_FileID", false),
            (2, "SInt64", "m_PathID", false),
        ]);
        let layout = Layout::compile(&t).unwrap();
        let kinds: Vec<_> = layout.children(0).map(|i| layout.ops[i].kind).collect();
        assert_eq!(
            kinds,
            [
                Kind::String,
                Kind::Bytes,
                Kind::Array { map: false },
                Kind::PPtr { wide_path_id: true }
            ]
        );
        let points = layout.children(0).nth(2).unwrap();
        assert_eq!(layout.ops[points + 1].fixed_size, Some(8));
        assert!(layout.ops[0].has_references);
        assert!(!layout.ops[points].has_references);
        assert!(layout.has_field("m_Target"));
    }
}
