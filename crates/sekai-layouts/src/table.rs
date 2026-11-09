//! Type trees of engine classes by class ID and Unity version.

use std::collections::BTreeMap;
use std::sync::Arc;

use sekai_unity::{ClassLayouts, TypeTree, TypeTreeNode};

use crate::error::TpkError;
use crate::tpk;

/// The release type of a version that states none, a final release.
const FINAL_RELEASE: u64 = 3;
/// The release type of a version with a release type that has no number.
const UNKNOWN_RELEASE: u64 = 0xFF;

/// Type trees of engine classes by class ID and Unity version.
#[derive(Debug)]
pub struct VersionTable {
    pub(crate) classes: BTreeMap<i32, Vec<ClassVersion>>,
    pub(crate) nodes: Vec<Node>,
    pub(crate) strings: Vec<String>,
}

/// What a class is from one Unity version on.
#[derive(Debug)]
pub(crate) struct ClassVersion {
    /// The first Unity version the entry applies to.
    pub(crate) from: u64,
    /// The root node of the release layout, absent when the class has none at these versions.
    pub(crate) release_root: Option<u16>,
}

/// One node of a layout. Nodes are shared between layouts and name the nodes below them.
#[derive(Debug)]
pub(crate) struct Node {
    /// The index of the type name in the strings.
    pub(crate) type_name: u16,
    /// The index of the field name in the strings.
    pub(crate) name: u16,
    pub(crate) byte_size: i32,
    pub(crate) version: u16,
    pub(crate) type_flags: u8,
    pub(crate) meta_flags: u32,
    pub(crate) subnodes: Vec<u16>,
}

impl VersionTable {
    /// Reads the type tree information of a TPK file.
    pub fn from_tpk(bytes: &[u8]) -> Result<Self, TpkError> {
        tpk::read(bytes)
    }

    /// The root node of the release layout of `class_id` at `version`, from the last entry that
    /// the version reaches.
    fn release_root(&self, class_id: i32, version: u64) -> Option<u16> {
        self.classes
            .get(&class_id)?
            .iter()
            .take_while(|entry| entry.from <= version)
            .last()?
            .release_root
    }

    fn string(&self, index: u16) -> Option<String> {
        self.strings.get(usize::from(index)).cloned()
    }

    fn tree_node(&self, node: &Node, level: u8, index: i32) -> Option<TypeTreeNode> {
        Some(TypeTreeNode {
            version: node.version,
            level,
            type_flags: node.type_flags,
            type_name: self.string(node.type_name)?,
            name: self.string(node.name)?,
            byte_size: node.byte_size,
            index,
            meta_flags: node.meta_flags,
            ref_type_hash: 0,
        })
    }
}

impl ClassLayouts for VersionTable {
    /// The release layout of the class, its nodes in preorder.
    fn tree(&self, class_id: i32, unity_version: &str) -> Option<Arc<TypeTree>> {
        let version = parse_version(unity_version)?;
        let mut nodes = Vec::new();
        let mut pending = vec![(self.release_root(class_id, version)?, 0u8)];
        while let Some((id, level)) = pending.pop() {
            let node = self.nodes.get(usize::from(id))?;
            let index = i32::try_from(nodes.len()).ok()?;
            nodes.push(self.tree_node(node, level, index)?);
            let below = level.checked_add(1)?;
            pending.extend(node.subnodes.iter().rev().map(|&id| (id, below)));
        }
        TypeTree::from_nodes(nodes).ok().map(Arc::new)
    }
}

/// A Unity version such as `2022.3.62f2` as a number that orders versions: major, minor and build
/// in 16 bits each, then the release type and its number in 8 bits each.
fn parse_version(text: &str) -> Option<u64> {
    let (major, rest) = number(text)?;
    let (minor, rest) = number(rest.strip_prefix('.')?)?;
    let (build, rest) = number(rest.strip_prefix('.')?)?;
    let digits = rest
        .find(|c: char| c.is_ascii_digit())
        .unwrap_or(rest.len());
    let (release, rest) = rest.split_at(digits);
    let release_number = number(rest).map_or(0, |(value, _)| value);
    (major.max(minor).max(build) <= 0xFFFF && release_number <= 0xFF).then_some(
        major << 48 | minor << 32 | build << 16 | release_type(release) << 8 | release_number,
    )
}

/// The digits at the start of `text` as a number, and what follows them.
fn number(text: &str) -> Option<(u64, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let (digits, rest) = text.split_at(end);
    Some((digits.parse().ok()?, rest))
}

fn release_type(text: &str) -> u64 {
    match text.to_ascii_lowercase().as_str() {
        "a" => 0,
        "b" => 1,
        "c" => 2,
        "f" | "" => FINAL_RELEASE,
        "p" => 4,
        "x" => 5,
        _ => UNKNOWN_RELEASE,
    }
}
