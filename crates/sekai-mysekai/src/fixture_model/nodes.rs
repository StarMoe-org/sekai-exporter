//! The keys of every node of a piece or an attachment: the keys of every MySekai model, the road
//! cell of a node and the placement root with the collision document of the prefab.

use std::collections::BTreeMap;

use sekai_convert::model::{Node, NodeKeys, Prefab};
use sekai_convert::{Context, ConvertError, Object, ObjectId, Value};
use serde_json::{Value as Json, json};

use crate::model_keys::{Collision, is_script, source_keys};
use crate::roots::FixturePrefab;
use crate::scripts::malformed;
use crate::stored::{array, field, stored_code};

/// The script class of the behaviour of the game object the game places.
const FIXTURE_VIEW: &str = ".FixtureView";
/// The script class of the behaviour of a road.
const ROAD_VIEW: &str = ".RoadView";

/// The keys of the nodes of a piece or an attachment.
pub(super) struct PieceNodes<'a> {
    prefab: &'a Prefab,
    collision: &'a Collision,
    /// Whether the root carries a `FixtureView`, which puts every node on the fixture layer.
    fixture_view: bool,
    /// The road cell type of each game object that is a road cell.
    cells: BTreeMap<ObjectId, i64>,
}

impl<'a> PieceNodes<'a> {
    /// The keys of the nodes of `prefab`, whose collision is `collision` and whose root carries
    /// a `FixtureView` when `fixture_view` is set.
    pub(super) fn read(
        prefab: &'a Prefab,
        collision: &'a Collision,
        fixture_view: bool,
        cx: &dyn Context,
    ) -> Result<Self, ConvertError> {
        Ok(Self {
            prefab,
            collision,
            fixture_view,
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
            // Every node is the root or below it, so it is on the fixture layer when the root
            // carries the FixtureView.
            record["fixtureNavLayer"] = json!(self.fixture_view);
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

/// The `FixtureView` of the root game object of `prefab`, a prefab of the kind `kind`. The root is
/// the game object the game places, and its collision is read from it down, so a `FixtureView`
/// below the root fails, and so does a piece whose root has none; an attachment is placed by its
/// root either way.
pub(super) fn placement(
    prefab: &Prefab,
    kind: FixturePrefab,
) -> Result<Option<&Object>, ConvertError> {
    let (root, below) = prefab
        .nodes()
        .split_first()
        .ok_or_else(|| malformed("the prefab has no game object"))?;
    let view = fixture_view(root);
    if view.is_none() && kind == FixturePrefab::Piece {
        return Err(malformed("the root game object has no FixtureView"));
    }
    if below.iter().any(|node| fixture_view(node).is_some()) {
        return Err(malformed("a FixtureView is below the root game object"));
    }
    Ok(view)
}

/// The `FixtureView` of the game object of `node`, if it has one.
fn fixture_view(node: &Node) -> Option<&Object> {
    node.components()
        .iter()
        .find(|component| is_script(component, FIXTURE_VIEW))
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
