//! Effect and background prefabs of stories.
//!
//! [`ScenarioPrefab`] produces the `scenario-prefab` form, specified in
//! `docs/spec/scenario-prefab.md`.

use sekai_convert::animation::clip_document;
use sekai_convert::image::{Texture, texture_memory};
use sekai_convert::{
    Asset, Context, ConvertError, Converter, Cost, CostClass, Encoders, Member, Object, ObjectId,
    ObjectIndex,
};

use crate::graph::{GraphDocument, Medium, fields_only, file_member, reachable};
use crate::roots;

const TEXTURE_2D: i32 = 28;
const ANIMATION_CLIP: i32 = 74;

/// The fields of a texture whose image is its member.
const TEXTURE_IMAGE_FIELDS: &[&str] = &["image data", "m_StreamData"];

/// The document of the form, with the images of its textures and the clip documents of its clips.
const PREFAB: GraphDocument = GraphDocument {
    format: "scenario-prefab",
    version: 1,
    role: "prefab",
    media,
};

/// An effect or a background prefab of a story with every object it refers to.
#[derive(Debug, Clone, Copy, Default)]
pub struct ScenarioPrefab;

impl Converter for ScenarioPrefab {
    fn name(&self) -> &'static str {
        "scenario-prefab"
    }

    fn version(&self) -> u32 {
        1
    }

    fn encoders(&self) -> Encoders {
        Encoders::TEXTURE
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        roots::scenario_prefab(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        PREFAB.asset(root, cx).map(Some)
    }

    fn cost(&self, root: ObjectId, object: &ObjectIndex, cx: &dyn Context) -> Cost {
        let textures: u64 = reachable(root, cx)
            .unwrap_or_default()
            .iter()
            .filter(|object| object.class_id == TEXTURE_2D)
            .map(|texture| texture_memory(texture.id, cx).unwrap_or(0))
            .sum();
        Cost {
            class: CostClass::Heavy,
            memory: textures.max(Cost::light(u64::from(object.size)).memory),
        }
    }
}

/// The image of a texture and the clip document of a clip, each a member beside its object.
fn media(n: usize, object: &Object, cx: &dyn Context) -> Result<Medium, ConvertError> {
    let id = object.id;
    match object.class_id {
        TEXTURE_2D => {
            let image = Texture.convert(id, cx)?;
            let image = image.and_then(|asset| asset.members.into_iter().next());
            Ok(Medium {
                member: image.map(|image| Member {
                    role: format!("texture/{n}"),
                    content: image.content,
                }),
                held: TEXTURE_IMAGE_FIELDS,
            })
        }
        ANIMATION_CLIP => {
            let clip = representable(clip_document(id, cx))?
                .map(|clip| clip.to_bytes().map_err(json_error))
                .transpose()?;
            Ok(Medium {
                member: clip.map(|clip| file_member(format!("clip/{n}"), "json", clip)),
                held: &[],
            })
        }
        _ => fields_only(n, object, cx),
    }
}

/// A decoded form of an object, or `None` for an object the form does not represent.
fn representable<T>(decoded: Result<T, ConvertError>) -> Result<Option<T>, ConvertError> {
    match decoded {
        Ok(decoded) => Ok(Some(decoded)),
        Err(e) if e.code == "unsupported" => Ok(None),
        Err(e) => Err(e),
    }
}

fn json_error(e: serde_json::Error) -> ConvertError {
    ConvertError::new("malformed", e.to_string())
}
