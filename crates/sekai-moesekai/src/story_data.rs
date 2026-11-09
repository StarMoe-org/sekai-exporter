//! Action sets and scenario backgrounds.
//!
//! [`ActionSetForm`] produces the `action-set` form, specified in `docs/spec/action-set.md`, and
//! [`ScenarioBackgroundForm`] the `scenario-background` form, specified in
//! `docs/spec/scenario-background.md`.

use std::collections::HashSet;

use sekai_convert::{
    Asset, Content, Context, ConvertError, Converter, Member, Object, ObjectId, ObjectIndex, PPtr,
    Value,
};
use sekai_format::story::{self, ActionSet, ActionSetDetail, ScenarioBackground};

use crate::roots;

const GAME_OBJECT: i32 = 1;
const TRANSFORM: i32 = 4;
const RECT_TRANSFORM: i32 = 224;
const MONO_BEHAVIOUR: i32 = 114;

/// The script of a movie of a scenario background.
const BACKGROUND_MOVIE: &str = "Assembly-CSharp/Sekai.ScenarioBackgroundMovie";

/// The area actors of an action set and the costumes they wear, as a JSON document.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActionSetForm;

impl Converter for ActionSetForm {
    fn name(&self) -> &'static str {
        "action-set"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        roots::action_set(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let action_set_id = integer(&object, "ActionSetId")?;
        let area_id = integer(&object, "AreaId")?;
        let Value::Array(items) = present(&object, "Details")? else {
            return Err(malformed("Details"));
        };
        let mut details = Vec::with_capacity(items.len());
        for (i, item) in items.iter().enumerate() {
            let Value::Struct(_) = item else {
                return Err(malformed(&format!("Details[{i}]")));
            };
            details.push(ActionSetDetail {
                area_character_id: detail_integer(item, i, "AreaCharacterId")?,
                costume2d_group_id: detail_integer(item, i, "Costume2dGroupId")?,
            });
        }
        let document = ActionSet {
            format: story::ACTION_SET_FORMAT.into(),
            version: story::VERSION,
            action_set_id,
            area_id,
            details,
        };
        let bytes = document.to_bytes().map_err(json_error)?;
        Ok(Some(Asset {
            members: vec![member("action-set", bytes)],
        }))
    }
}

/// The movies a scenario background plays, as a JSON document.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScenarioBackgroundForm;

impl Converter for ScenarioBackgroundForm {
    fn name(&self) -> &'static str {
        "scenario-background"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        roots::scenario_background(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let Some(reference) = optional_reference(&object.fields, "BackgroundPrefab")? else {
            return Ok(None);
        };
        let Some(prefab) = cx.resolve(root, reference)? else {
            return Ok(None);
        };
        let movies = movies(prefab, cx)?;
        if movies.is_empty() {
            return Ok(None);
        }
        let document = ScenarioBackground {
            format: story::SCENARIO_BACKGROUND_FORMAT.into(),
            version: story::VERSION,
            movies,
        };
        let bytes = document.to_bytes().map_err(json_error)?;
        Ok(Some(Asset {
            members: vec![member("scenario-background", bytes)],
        }))
    }
}

/// The movie names the prefab `prefab` reaches, depth first in the order of the children of every
/// object: the nonempty names of the movie components of the active objects, each once, at its
/// first place.
fn movies(prefab: ObjectId, cx: &dyn Context) -> Result<Vec<String>, ConvertError> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let mut visited = HashSet::new();
    let mut stack = vec![prefab];
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        let object = cx.object(id)?;
        if object.class_id != GAME_OBJECT || !active(&object.fields) {
            continue;
        }
        let mut children = Vec::new();
        let components = list(&object.fields, "m_Component")?;
        for (i, entry) in components.iter().enumerate() {
            let Some(reference) = entry_reference(entry, &format!("m_Component[{i}]"))? else {
                continue;
            };
            let Some(target) = cx.resolve(id, reference)? else {
                continue;
            };
            let component = cx.object(target)?;
            match component.class_id {
                TRANSFORM | RECT_TRANSFORM => transform_children(&component, cx, &mut children)?,
                MONO_BEHAVIOUR if component.script.as_deref() == Some(BACKGROUND_MOVIE) => {
                    let name = string(&component.fields, "movieAssetBundleName")?;
                    if !name.is_empty() && seen.insert(name.to_owned()) {
                        names.push(name.to_owned());
                    }
                }
                _ => {}
            }
        }
        // Pushed last to first, so that the first child is the first one popped.
        stack.extend(children.into_iter().rev());
    }
    Ok(names)
}

/// Adds the game objects of the children of `transform`, in sibling order, to `children`.
fn transform_children(
    transform: &Object,
    cx: &dyn Context,
    children: &mut Vec<ObjectId>,
) -> Result<(), ConvertError> {
    for (i, item) in list(&transform.fields, "m_Children")?.iter().enumerate() {
        let Some(reference) = entry_reference(item, &format!("m_Children[{i}]"))? else {
            continue;
        };
        let Some(target) = cx.resolve(transform.id, reference)? else {
            continue;
        };
        let child = cx.object(target)?;
        let Some(game_object) = optional_reference(&child.fields, "m_GameObject")? else {
            continue;
        };
        let Some(game_object) = cx.resolve(child.id, game_object)? else {
            continue;
        };
        children.push(game_object);
    }
    Ok(())
}

/// Whether a game object is active: `m_IsActive` that is `true` or a nonzero integer, or no
/// `m_IsActive` at all.
fn active(fields: &Value) -> bool {
    match fields.get("m_IsActive") {
        None | Some(Value::Bool(true)) => true,
        Some(Value::Bool(false)) => false,
        Some(Value::UInt(v)) => *v != 0,
        Some(Value::Int(v)) => *v != 0,
        Some(_) => false,
    }
}

/// The reference an entry of `m_Component` or of `m_Children` holds: the entry itself, or the
/// `component` of the entry.
fn entry_reference(value: &Value, path: &str) -> Result<Option<PPtr>, ConvertError> {
    match value {
        Value::PPtr(reference) => Ok(Some(*reference)),
        Value::Struct(_) => optional_reference(value, "component"),
        _ => Err(malformed(path)),
    }
}

/// The reference stored in `fields` as the field `name`, or `None` when it is absent or null.
fn optional_reference(fields: &Value, name: &str) -> Result<Option<PPtr>, ConvertError> {
    match fields.get(name) {
        None => Ok(None),
        Some(Value::PPtr(reference)) if reference.path_id == 0 => Ok(None),
        Some(Value::PPtr(reference)) => Ok(Some(*reference)),
        Some(_) => Err(malformed(name)),
    }
}

/// The array stored in `fields` as the field `name`, or no entries when the field is absent.
fn list<'v>(fields: &'v Value, name: &str) -> Result<&'v [Value], ConvertError> {
    match fields.get(name) {
        None => Ok(&[]),
        Some(Value::Array(items)) => Ok(items),
        Some(_) => Err(malformed(name)),
    }
}

/// The string stored in `fields` as the field `name`, or the empty string when the field is
/// absent.
fn string<'v>(fields: &'v Value, name: &str) -> Result<&'v str, ConvertError> {
    match fields.get(name) {
        None => Ok(""),
        Some(Value::String(s)) => Ok(s),
        Some(_) => Err(malformed(name)),
    }
}

/// The field `name` of `object`.
fn present<'o>(object: &'o Object, name: &str) -> Result<&'o Value, ConvertError> {
    object
        .fields
        .get(name)
        .ok_or_else(|| ConvertError::new("malformed", format!("{name} is absent")))
}

/// The integer stored in the field `name` of `object`.
fn integer(object: &Object, name: &str) -> Result<i64, ConvertError> {
    present(object, name)?
        .as_i64()
        .ok_or_else(|| malformed(name))
}

/// The integer stored in the field `name` of entry `index` of `Details`.
fn detail_integer(value: &Value, index: usize, name: &str) -> Result<i64, ConvertError> {
    value
        .get(name)
        .and_then(Value::as_i64)
        .ok_or_else(|| malformed(&format!("Details[{index}].{name}")))
}

/// The single member of an asset: a JSON document with the role `role`.
fn member(role: &str, bytes: Vec<u8>) -> Member {
    Member {
        role: role.into(),
        content: Content::File {
            extension: "json".into(),
            bytes,
        },
    }
}

/// A `malformed` error about the field `name`.
fn malformed(name: &str) -> ConvertError {
    ConvertError::new("malformed", format!("{name} is not of the expected type"))
}

/// The error of a document that cannot be written.
fn json_error(e: serde_json::Error) -> ConvertError {
    ConvertError::new("malformed", format!("the document cannot be written: {e}"))
}
