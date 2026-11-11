//! Live2D Cubism models and their motions.
//!
//! [`Model`] produces the `live2d-model` form and [`Motions`] the `live2d-motions` form, both
//! specified in `docs/spec/live2d.md`.

use std::collections::BTreeSet;

use sekai_format::live2d::{self, MotionDocument};
use sekai_unity::Value;

use crate::animation::clip_document;
use crate::image::{Texture, texture_memory};
use crate::roots;
use crate::{
    Asset, Content, Context, ConvertError, Converter, Cost, CostClass, Member, ObjectId,
    ObjectIndex,
};

const TEXT_ASSET: i32 = 49;
const TEXTURE_2D: i32 = 28;
/// The script of the metadata that gives a motion its fade times.
const MOTION_META: &str = "Assembly-CSharp/Sekai.Live2D.Live2DBuildMotionMetaData";
/// How every `.moc3` file begins.
const MOC_MAGIC: &[u8] = b"MOC3";

/// A model as the game loads it, from its build data.
#[derive(Debug, Clone, Copy, Default)]
pub struct Model;

impl Converter for Model {
    fn name(&self) -> &'static str {
        "live2d-model"
    }

    fn version(&self) -> u32 {
        2
    }

    fn encoders(&self) -> crate::Encoders {
        crate::Encoders::IMAGE
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        roots::live2d_model(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let fields = &object.fields;
        if !text(fields, "UserDataFileName")?.is_empty() {
            return Err(unsupported("UserDataFileName is not empty"));
        }
        if !array(fields, "CategoryRules")?.is_empty() {
            return Err(unsupported("CategoryRules is not empty"));
        }
        let files = ModelFiles::find(root, fields, cx)?;
        let mut members = Vec::new();

        let moc = stored_bytes(files.moc, cx)?;
        if !moc.starts_with(MOC_MAGIC) {
            return Err(malformed("the .moc3 file does not begin with MOC3"));
        }
        members.push(file("moc", "moc3", moc));
        if let Some(physics) = files.physics {
            members.push(file("physics", "json", stored_bytes(physics, cx)?));
        }
        for (i, &texture) in files.textures.iter().enumerate() {
            let content = Texture
                .convert(texture, cx)?
                .and_then(|asset| asset.members.into_iter().next())
                .ok_or_else(|| malformed("a texture of the model holds no image"))?
                .content;
            members.push(Member {
                role: format!("texture/{i}"),
                content,
            });
        }
        members.extend(motions(
            root,
            array(fields, "AdditionalMotionData")?,
            "motion",
            cx,
        )?);
        Ok(Some(Asset { members }))
    }

    /// The files the build data names: they share its directory and are not referenced by it.
    fn inputs(&self, root: ObjectId, cx: &dyn Context) -> Vec<ObjectId> {
        cx.object(root)
            .and_then(|object| ModelFiles::find(root, &object.fields, cx))
            .map(|files| files.all())
            .unwrap_or_default()
    }

    fn cost(&self, root: ObjectId, object: &ObjectIndex, cx: &dyn Context) -> Cost {
        let textures = cx
            .object(root)
            .and_then(|object| ModelFiles::find(root, &object.fields, cx))
            .map(|files| {
                files
                    .textures
                    .iter()
                    .map(|&t| texture_memory(t, cx).unwrap_or(0))
                    .sum::<u64>()
            })
            .unwrap_or(0);
        Cost {
            class: CostClass::Heavy,
            memory: textures.max(Cost::light(u64::from(object.size)).memory),
        }
    }
}

/// A set of motions as the game loads it, from its build data.
#[derive(Debug, Clone, Copy, Default)]
pub struct Motions;

impl Converter for Motions {
    fn name(&self) -> &'static str {
        "live2d-motions"
    }

    fn version(&self) -> u32 {
        2
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        roots::live2d_motions(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let fields = &object.fields;
        for name in ["Arms", "Poses"] {
            if !array(fields, name)?.is_empty() {
                return Err(unsupported(format!("{name} is not empty")));
            }
        }
        let mut members = motions(root, array(fields, "Motions")?, "motion", cx)?;
        members.extend(motions(root, array(fields, "Facials")?, "facial", cx)?);
        Ok(Some(Asset { members }))
    }
}

/// The objects the build data of a model names by file name.
struct ModelFiles {
    moc: ObjectId,
    textures: Vec<ObjectId>,
    physics: Option<ObjectId>,
}

impl ModelFiles {
    fn find(root: ObjectId, fields: &Value, cx: &dyn Context) -> Result<Self, ConvertError> {
        let container = cx.container(root)?;
        let directory = container
            .iter()
            .find_map(|(path, id)| (*id == root).then_some(path.as_str()))
            .and_then(|path| path.rsplit_once('/'))
            .map(|(directory, _)| directory)
            .ok_or_else(|| malformed("the build data has no container path"))?;
        let named = |name: &str, class_id: i32| -> Result<ObjectId, ConvertError> {
            let target = format!("{directory}/{name}").to_ascii_lowercase();
            let id = container
                .iter()
                .find_map(|(path, id)| (path.to_ascii_lowercase() == target).then_some(*id))
                .ok_or_else(|| {
                    ConvertError::new("missing_object", format!("the bundle has no {name}"))
                })?;
            let object = cx.object(id)?;
            if object.class_id != class_id {
                return Err(malformed(format!("{name} is a {}", object.class)));
            }
            Ok(id)
        };
        let moc = named(text(fields, "Moc3FileName")?, TEXT_ASSET)?;
        let textures = array(fields, "TextureNames")?
            .iter()
            .map(|name| {
                let name = name
                    .as_str()
                    .ok_or_else(|| malformed("TextureNames holds a value that is not a string"))?;
                named(name, TEXTURE_2D)
            })
            .collect::<Result<_, _>>()?;
        let physics = match text(fields, "PhysicsFileName")? {
            "" => None,
            name => Some(named(name, TEXT_ASSET)?),
        };
        Ok(Self {
            moc,
            textures,
            physics,
        })
    }

    fn all(&self) -> Vec<ObjectId> {
        let mut ids = vec![self.moc];
        ids.extend(self.physics);
        ids.extend(&self.textures);
        ids
    }
}

/// The members `<kind>/<name>` of the motion entries `entries`, in their order.
fn motions(
    root: ObjectId,
    entries: &[Value],
    kind: &str,
    cx: &dyn Context,
) -> Result<Vec<Member>, ConvertError> {
    let mut names = BTreeSet::new();
    let mut members = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = text(entry, "ClipAssetName")?;
        if !names.insert(name) {
            return Err(malformed(format!("two {kind} entries are named {name}")));
        }
        let referenced = |field_name: &str| -> Result<Option<ObjectId>, ConvertError> {
            let reference = field(entry, field_name)?
                .as_pptr()
                .ok_or_else(|| malformed(format!("{field_name} is not a reference")))?;
            cx.resolve(root, reference)
        };
        let Some(clip) = referenced("Clip")? else {
            continue;
        };
        let (fade_in, fade_out) = match referenced("Meta")? {
            None => (None, None),
            Some(meta) => {
                let meta = cx.object(meta)?;
                if meta.script.as_deref() != Some(MOTION_META) {
                    return Err(malformed(format!(
                        "the metadata of the {kind} {name} is not motion metadata"
                    )));
                }
                (
                    Some(seconds(&meta.fields, "FadeInTime")?),
                    Some(seconds(&meta.fields, "FadeOutTime")?),
                )
            }
        };
        let document = MotionDocument {
            format: live2d::FORMAT.into(),
            version: live2d::VERSION,
            fade_in,
            fade_out,
            clip: clip_document(clip, cx)?,
        };
        let bytes = document
            .to_bytes()
            .map_err(|e| malformed(format!("the {kind} {name}: {e}")))?;
        members.push(file(&format!("{kind}/{name}"), "json", bytes));
    }
    Ok(members)
}

/// The bytes a `TextAsset` stores.
fn stored_bytes(id: ObjectId, cx: &dyn Context) -> Result<Vec<u8>, ConvertError> {
    Ok(field(&cx.object(id)?.fields, "m_Script")?
        .as_bytes()
        .ok_or_else(|| malformed("m_Script is not a string"))?
        .to_vec())
}

fn file(role: &str, extension: &str, bytes: Vec<u8>) -> Member {
    Member {
        role: role.into(),
        content: Content::File {
            extension: extension.into(),
            bytes,
        },
    }
}

fn seconds(fields: &Value, name: &str) -> Result<f32, ConvertError> {
    let value = field(fields, name)?
        .as_f64()
        .map(|v| v as f32)
        .ok_or_else(|| malformed(format!("{name} is not a number")))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(malformed(format!("{name} is not finite")))
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ConvertError> {
    value
        .get(name)
        .ok_or_else(|| malformed(format!("the field {name} is missing")))
}

fn text<'a>(value: &'a Value, name: &str) -> Result<&'a str, ConvertError> {
    field(value, name)?
        .as_str()
        .ok_or_else(|| malformed(format!("{name} is not a string")))
}

fn array<'a>(value: &'a Value, name: &str) -> Result<&'a [Value], ConvertError> {
    field(value, name)?
        .as_array()
        .ok_or_else(|| malformed(format!("{name} is not an array")))
}

fn malformed(message: impl Into<String>) -> ConvertError {
    ConvertError::new("malformed", message)
}

fn unsupported(message: impl Into<String>) -> ConvertError {
    ConvertError::new("unsupported", message)
}
