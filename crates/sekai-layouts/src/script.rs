//! The layouts of game scripts, and the layouts of the player data of a client package.

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use flate2::read::GzDecoder;
use sekai_unity::{ClassLayouts, TypeTree, TypeTreeNode};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::TableError;
use crate::table::VersionTable;

const MONO_BEHAVIOUR: i32 = 114;

/// One line of a table: a script with its fields, or without them when they could not be worked
/// out.
#[derive(Deserialize)]
struct ScriptRow {
    assembly: String,
    class: String,
    nodes: Option<Vec<ScriptNode>>,
}

/// One node of the fields of a script. The fields follow the engine header of a `MonoBehaviour`,
/// so the top ones have level 1.
#[derive(Debug, Deserialize)]
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

/// The fields of game scripts, by `"<assembly>/<namespace>.<class>"`.
#[derive(Debug)]
pub struct ScriptTable {
    scripts: HashMap<String, Vec<ScriptNode>>,
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
        let mut scripts = HashMap::new();
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
        let fields = self.scripts.as_ref()?.scripts.get(script)?;
        let header = self.classes.tree(MONO_BEHAVIOUR, unity_version)?;
        let mut nodes = header.nodes().to_vec();
        for field in fields {
            let index = i32::try_from(nodes.len()).ok()?;
            nodes.push(field.tree_node(index));
        }
        TypeTree::from_nodes(nodes).ok().map(Arc::new)
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
