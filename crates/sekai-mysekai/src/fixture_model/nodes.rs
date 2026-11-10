//! The keys of every node of a piece: the keys of every MySekai model, the road cell of a node and
//! the placement root with the collision document of the piece.

use std::collections::BTreeMap;

use sekai_convert::model::{Node, NodeKeys, Prefab};
use sekai_convert::{Context, ConvertError, Object, ObjectId, Value};
use serde_json::{Value as Json, json};

use crate::model_keys::{Collision, is_script, source_keys};
use crate::scripts::malformed;
use crate::stored::{array, field, stored_code};

/// The script class of the behaviour of the game object the game places.
pub(super) const FIXTURE_VIEW: &str = ".FixtureView";
/// The script class of the behaviour of a road.
const ROAD_VIEW: &str = ".RoadView";

/// The keys of the nodes of a piece.
pub(super) struct PieceNodes<'a> {
    prefab: &'a Prefab,
    collision: &'a Collision,
    /// The road cell type of each game object that is a road cell.
    cells: BTreeMap<ObjectId, i64>,
}

impl<'a> PieceNodes<'a> {
    /// The keys of the nodes of `prefab`, whose collision is `collision`.
    pub(super) fn read(
        prefab: &'a Prefab,
        collision: &'a Collision,
        cx: &dyn Context,
    ) -> Result<Self, ConvertError> {
        Ok(Self {
            prefab,
            collision,
            cells: road_cells(prefab, cx)?,
        })
    }
}

impl NodeKeys for PieceNodes<'_> {
    fn keys(
        &self,
        object: ObjectId,
        cx: &dyn Context,
    ) -> Result<BTreeMap<String, Json>, ConvertError> {
        let node = self
            .prefab
            .node(object)
            .ok_or_else(|| malformed("a node is not a game object of the prefab"))?;
        let mut keys = source_keys(self.prefab, node, self.collision, cx)?;
        if let Some(record) = keys.get_mut("sourceCollision") {
            // Every node of a piece is the FixtureView game object or below it.
            record["fixtureNavLayer"] = json!(true);
        }
        if let Some(&cell) = self.cells.get(&object) {
            keys.insert("roadCellType".to_owned(), json!(cell));
        }
        if self.is_root(node) {
            keys.insert("fixtureViewRoot".to_owned(), json!(true));
            keys.insert("fixtureCollision".to_owned(), self.collision.document());
        }
        Ok(keys)
    }
}

impl PieceNodes<'_> {
    fn is_root(&self, node: &Node) -> bool {
        self.prefab
            .nodes()
            .first()
            .is_some_and(|root| root.object() == node.object())
    }
}

/// Fails unless the root game object of `prefab` carries a `FixtureView` and no game object below
/// it does: the root is the game object the game places, and its collision is read from it down.
pub(super) fn check_placement(prefab: &Prefab) -> Result<(), ConvertError> {
    let has_view = |node: &Node| node.components().iter().any(|c| is_script(c, FIXTURE_VIEW));
    match prefab.nodes().split_first() {
        Some((root, _)) if !has_view(root) => {
            Err(malformed("the root game object has no FixtureView"))
        }
        Some((_, below)) if below.iter().any(has_view) => {
            Err(malformed("a FixtureView is below the root game object"))
        }
        Some(_) => Ok(()),
        None => Err(malformed("the prefab has no game object")),
    }
}

/// The road cell type of each game object that is a cell of a `RoadView` of `prefab`. Each entry
/// of a road's list names its cell, the cell names the renderer that draws it, and the renderer's
/// game object is the cell's node.
fn road_cells(prefab: &Prefab, cx: &dyn Context) -> Result<BTreeMap<ObjectId, i64>, ConvertError> {
    let roads = prefab
        .nodes()
        .iter()
        .flat_map(Node::components)
        .filter(|component| is_script(component, ROAD_VIEW));
    let mut cells = BTreeMap::new();
    for road in roads {
        for entry in array(
            field(&road.fields, "_roadViewDataList")?,
            "_roadViewDataList",
        )? {
            let cell = target(cx, road.id, field(entry, "_cell")?, "_cell")?;
            let renderer = target(cx, cell.id, field(&cell.fields, "_renderer")?, "_renderer")?;
            let owner = target(
                cx,
                renderer.id,
                field(&renderer.fields, "m_GameObject")?,
                "m_GameObject",
            )?;
            let cell_type = stored_code(field(entry, "_cellType")?, "_cellType")?;
            cells.insert(owner.id, cell_type);
        }
    }
    Ok(cells)
}

/// The object the reference `stored`, stored in the object `from` as `name`, points to.
fn target(
    cx: &dyn Context,
    from: ObjectId,
    stored: &Value,
    name: &str,
) -> Result<Object, ConvertError> {
    let reference = stored
        .as_pptr()
        .ok_or_else(|| malformed(format!("{name} is not a reference")))?;
    let id = cx
        .resolve(from, reference)?
        .ok_or_else(|| malformed(format!("{name} is null")))?;
    cx.object(id)
}
