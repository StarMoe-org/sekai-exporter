//! The keys every node of a MySekai model has: the source identity of its game object, its
//! components and children, its renderer and its collision record.

use std::collections::BTreeMap;

use sekai_convert::model::{Node, Prefab};
use sekai_convert::{Context, ConvertError, Object, ObjectId, Value};
use serde_json::{Value as Json, json};

use super::collision::Collision;
use crate::stored::{array, field, number};

/// The class ID of `MeshRenderer`.
const MESH_RENDERER: i32 = 23;
/// The class ID of `SkinnedMeshRenderer`.
const SKINNED_MESH_RENDERER: i32 = 137;

/// The keys of `node`, a node of `prefab` whose collision `collision` holds.
pub(crate) fn source_keys(
    prefab: &Prefab,
    node: &Node,
    collision: &Collision,
    cx: &dyn Context,
) -> Result<BTreeMap<String, Json>, ConvertError> {
    let nodes = prefab.nodes();
    let children: Vec<ObjectId> = node
        .children()
        .iter()
        .map(|&child| nodes[child].transform())
        .collect();
    let mut keys = BTreeMap::from([
        ("sourceObject".to_owned(), source_object(node, cx)?),
        (
            "sourceComponentIds".to_owned(),
            decimal_ids(node.component_ids()),
        ),
        ("sourceChildOrder".to_owned(), decimal_ids(&children)),
    ]);
    if let Some(renderer) = node.components().iter().find(|c| is_renderer(c)) {
        keys.extend(renderer_keys(renderer)?);
    }
    if let Some(record) = collision.record(node.object()) {
        keys.insert("sourceCollision".to_owned(), record.clone());
    }
    Ok(keys)
}

/// The source identity of `node`: the serialized file of its game object, and the ids of the game
/// object and its transform.
pub(crate) fn source_object(node: &Node, cx: &dyn Context) -> Result<Json, ConvertError> {
    Ok(json!({
        "file": cx.file_name(node.object())?,
        "gameObjectId": node.object().path_id.to_string(),
        "transformId": node.transform().path_id.to_string(),
    }))
}

/// Object ids as decimal strings, in order.
fn decimal_ids(ids: &[ObjectId]) -> Json {
    json!(
        ids.iter()
            .map(|id| id.path_id.to_string())
            .collect::<Vec<_>>()
    )
}

fn is_renderer(component: &Object) -> bool {
    matches!(component.class_id, MESH_RENDERER | SKINNED_MESH_RENDERER)
}

/// The keys of a renderer: the mode it casts shadows in, and whether it draws with a material,
/// that is whether some material slot names a material.
fn renderer_keys(renderer: &Object) -> Result<[(String, Json); 2], ConvertError> {
    let fields = &renderer.fields;
    let shadows = number(field(fields, "m_CastShadows")?, "m_CastShadows")?;
    let materialed = array(field(fields, "m_Materials")?, "m_Materials")?
        .iter()
        .filter_map(Value::as_pptr)
        .any(|material| material.path_id != 0);
    Ok([
        ("shadowCastingMode".to_owned(), shadows),
        ("materialed".to_owned(), json!(materialed)),
    ])
}
