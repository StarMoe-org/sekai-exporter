//! The layouts of game scripts, and the layouts of the player data of a client package.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::Arc;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sekai_unity::{ClassLayouts, TypeTree, TypeTreeNode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::TableError;
use crate::table::VersionTable;

const MONO_BEHAVIOUR: i32 = 114;

/// One line of a table: a script with its fields, or without them when they could not be worked
/// out.
#[derive(Serialize, Deserialize)]
struct ScriptRow {
    assembly: String,
    class: String,
    nodes: Option<ScriptFields>,
}

/// One node of the fields of a script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ScriptNode {
    #[serde(rename = "type")]
    type_name: String,
    name: String,
    level: u8,
    byte_size: i32,
    type_flags: u8,
    version: u16,
    meta_flag: u32,
}

impl ScriptNode {
    fn of(node: &TypeTreeNode) -> Self {
        Self {
            type_name: node.type_name.clone(),
            name: node.name.clone(),
            level: node.level,
            byte_size: node.byte_size,
            type_flags: node.type_flags,
            version: node.version,
            meta_flag: node.meta_flags,
        }
    }

    fn tree_node(&self, index: i32) -> TypeTreeNode {
        TypeTreeNode {
            version: self.version,
            level: self.level,
            type_flags: self.type_flags,
            type_name: self.type_name.clone(),
            name: self.name.clone(),
            byte_size: self.byte_size,
            index,
            meta_flags: self.meta_flag,
            ref_type_hash: 0,
        }
    }
}

/// The fields of a script: the nodes of the type tree of its `MonoBehaviour` objects after the
/// engine header, so the top ones have level 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ScriptFields(Vec<ScriptNode>);

impl ScriptFields {
    /// The fields of `tree`, the full type tree of a `MonoBehaviour`, whose engine header is
    /// `header`. `None` when `tree` does not start with `header`, by type, name and level.
    pub fn of_tree(tree: &TypeTree, header: &TypeTree) -> Option<Self> {
        let (head, fields) = tree.nodes().split_at_checked(header.nodes().len())?;
        let same = |(a, b): (&TypeTreeNode, &TypeTreeNode)| {
            (a.level, &a.type_name, &a.name) == (b.level, &b.type_name, &b.name)
        };
        let headed = head.iter().zip(header.nodes()).all(same);
        let top = fields.first().is_none_or(|field| field.level == 1);
        (headed && top).then(|| Self(fields.iter().map(ScriptNode::of).collect()))
    }

    /// The full type tree of a `MonoBehaviour` with these fields: `header`, then the fields.
    pub fn tree(&self, header: &TypeTree) -> Option<TypeTree> {
        let mut nodes = header.nodes().to_vec();
        for field in &self.0 {
            let index = i32::try_from(nodes.len()).ok()?;
            nodes.push(field.tree_node(index));
        }
        TypeTree::from_nodes(nodes).ok()
    }
}

/// The fields of game scripts, by `"<assembly>/<namespace>.<class>"`.
#[derive(Debug)]
pub struct ScriptTable {
    scripts: BTreeMap<String, ScriptFields>,
    sha256: [u8; 32],
}

impl ScriptTable {
    /// Reads a gzipped file with one JSON object per line.
    pub fn from_jsonl_gz(bytes: &[u8]) -> Result<Self, TableError> {
        let mut text = String::new();
        GzDecoder::new(bytes)
            .read_to_string(&mut text)
            .map_err(|e| TableError::new(format!("gzip: {e}")))?;
        let rows = text.lines().map(|line| {
            serde_json::from_str::<ScriptRow>(line).map_err(|e| TableError::new(e.to_string()))
        });
        let mut scripts = BTreeMap::new();
        for row in rows {
            let row = row?;
            if let Some(nodes) = row.nodes {
                scripts.insert(format!("{}/{}", row.assembly, row.class), nodes);
            }
        }
        Ok(Self {
            scripts,
            sha256: Sha256::digest(bytes).into(),
        })
    }

    /// The fields of `script`, when the table has them.
    pub fn get(&self, script: &str) -> Option<&ScriptFields> {
        self.scripts.get(script)
    }

    /// Every script of the table with its fields, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &ScriptFields)> {
        self.scripts
            .iter()
            .map(|(script, fields)| (script.as_str(), fields))
    }
}

/// Writes the fields of `scripts`, each named `"<assembly>/<namespace>.<class>"`, as a table
/// file that [`ScriptTable::from_jsonl_gz`] reads: gzipped JSON, one line per script.
pub fn write_script_table<'a>(
    scripts: impl IntoIterator<Item = (&'a str, &'a ScriptFields)>,
) -> Result<Vec<u8>, TableError> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    for (script, fields) in scripts {
        let (assembly, class) = script
            .split_once('/')
            .ok_or_else(|| TableError::new(format!("{script}: no assembly in the name")))?;
        let row = ScriptRow {
            assembly: assembly.to_owned(),
            class: class.to_owned(),
            nodes: Some(fields.clone()),
        };
        let line = serde_json::to_string(&row).map_err(|e| TableError::new(e.to_string()))?;
        writeln!(encoder, "{line}").map_err(|e| TableError::new(format!("gzip: {e}")))?;
    }
    encoder
        .finish()
        .map_err(|e| TableError::new(format!("gzip: {e}")))
}

/// The layouts of the engine classes and the scripts of the player data of a client package.
#[derive(Debug)]
pub struct PlayerLayouts {
    classes: VersionTable,
    scripts: Option<ScriptTable>,
}

impl PlayerLayouts {
    /// Layouts from `classes` and, for `MonoBehaviour` objects, `scripts`.
    pub fn new(classes: VersionTable, scripts: Option<ScriptTable>) -> Self {
        Self { classes, scripts }
    }
}

impl ClassLayouts for PlayerLayouts {
    fn tree(&self, class_id: i32, unity_version: &str) -> Option<Arc<TypeTree>> {
        self.classes.tree(class_id, unity_version)
    }

    /// The engine header of a `MonoBehaviour`, then the fields of the script.
    fn script_tree(&self, script: &str, unity_version: &str) -> Option<Arc<TypeTree>> {
        let fields = self.scripts.as_ref()?.get(script)?;
        let header = self.classes.tree(MONO_BEHAVIOUR, unity_version)?;
        fields.tree(&header).map(Arc::new)
    }

    fn identity(&self) -> Option<[u8; 32]> {
        let scripts = self.scripts.as_ref().map(|table| table.sha256);
        Some(scripts.map_or(self.classes.sha256(), |scripts| {
            let mut hash = Sha256::new();
            hash.update(self.classes.sha256());
            hash.update(scripts);
            hash.finalize().into()
        }))
    }
}
