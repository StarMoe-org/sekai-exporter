//! The attach points of a piece: where characters stand to use it, as its `FixtureView` stores
//! them.

use sekai_convert::model::{Node, Prefab};
use sekai_convert::{Context, ConvertError, Object, ObjectId, Value};
use serde_json::{Value as Json, json};

use super::nodes::{FIXTURE_VIEW, source_object};
use super::scripts::is_script;
use crate::scripts::malformed;
use crate::stored::{array, field};

/// The class ID of `MeshCollider`.
const MESH_COLLIDER: i32 = 64;

/// The `attach-points` document of `prefab`, whose root carries the `FixtureView`.
pub(super) fn document(prefab: &Prefab, cx: &dyn Context) -> Result<Json, ConvertError> {
    let (root, _) = prefab
        .nodes()
        .split_first()
        .ok_or_else(|| malformed("the prefab has no game object"))?;
    let view = root
        .components()
        .iter()
        .find(|c| is_script(c, FIXTURE_VIEW))
        .ok_or_else(|| malformed("the root game object has no FixtureView"))?;
    let points = array(field(&view.fields, "_attachPoints")?, "_attachPoints")?
        .iter()
        .map(|point| {
            let start = reference(cx, view, field(point, "StartLoc")?, "StartLoc")?
                .ok_or_else(|| malformed("an attach point has no StartLoc"))?;
            let end = reference(cx, view, field(point, "EndLoc")?, "EndLoc")?;
            Ok(json!({
                "start": locator(prefab, start, cx)?,
                "end": end.map(|end| locator(prefab, end, cx)).transpose()?,
            }))
        })
        .collect::<Result<Vec<_>, ConvertError>>()?;
    Ok(json!({
        "root": { "sourceObject": source_object(root, cx)?, "scale": numbers(root.pose().scale)? },
        "collider": collider(prefab, view, cx)?,
        "points": points,
    }))
}

/// The object a reference stored in `view` as `name` points to, or `None` when it is null.
fn reference(
    cx: &dyn Context,
    view: &Object,
    stored: &Value,
    name: &str,
) -> Result<Option<ObjectId>, ConvertError> {
    let pointer = stored
        .as_pptr()
        .ok_or_else(|| malformed(format!("{name} is not a reference")))?;
    cx.resolve(view.id, pointer)
}

/// The locator whose game object is `object`. It is a child of the root, so its local pose is
/// its pose in the frame of the root.
fn locator(prefab: &Prefab, object: ObjectId, cx: &dyn Context) -> Result<Json, ConvertError> {
    let node = prefab
        .node(object)
        .filter(|node| node.object() == object)
        .ok_or_else(|| malformed("a locator is not a game object of the prefab"))?;
    if !is_child_of_root(prefab, node) {
        return Err(ConvertError::new(
            "unsupported",
            format!("the locator {} is not a child of the root", node.name()),
        ));
    }
    let pose = node.pose();
    Ok(json!({
        "name": node.name(),
        "sourceObject": source_object(node, cx)?,
        "position": numbers(pose.translation)?,
        "rotation": numbers(pose.rotation)?,
    }))
}

fn is_child_of_root(prefab: &Prefab, node: &Node) -> bool {
    let nodes = prefab.nodes();
    nodes.first().is_some_and(|root| {
        root.children()
            .iter()
            .any(|&child| nodes[child].object() == node.object())
    })
}

/// The mesh collider the `FixtureView` names as its own, by the node that carries it and its
/// component id, or `null`.
fn collider(prefab: &Prefab, view: &Object, cx: &dyn Context) -> Result<Json, ConvertError> {
    let Some(id) = reference(cx, view, field(&view.fields, "_collider")?, "_collider")? else {
        return Ok(Json::Null);
    };
    let node = prefab
        .nodes()
        .iter()
        .find(|node| {
            node.components()
                .iter()
                .any(|c| c.id == id && c.class_id == MESH_COLLIDER)
        })
        .ok_or_else(|| malformed("_collider is not a mesh collider of the prefab"))?;
    Ok(json!({
        "sourceObject": source_object(node, cx)?,
        "componentId": id.path_id.to_string(),
    }))
}

/// Numbers as the model file writes them: each the shortest decimal that reads back as the same
/// `f32`, so that a pose here and on its node are the same text.
fn numbers<const N: usize>(values: [f32; N]) -> Result<Json, ConvertError> {
    values
        .iter()
        .map(|value| {
            let shortest: f64 = value.to_string().parse().unwrap_or(f64::NAN);
            serde_json::Number::from_f64(shortest)
                .map(Json::Number)
                .ok_or_else(|| malformed("a locator's transform is not finite"))
        })
        .collect()
}
