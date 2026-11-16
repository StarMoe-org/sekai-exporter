//! The pieces of MySekai furniture and the attachments of custom furniture as model files.
//!
//! [`FixtureModel`] produces the `fixture-model` form: one piece of furniture, or one attachment of
//! a custom piece, as the model file of its prefab, with the source identity, renderer state and
//! collision of every node and, on the root, the collision document of the prefab, and with the
//! attach points of its `FixtureView` as a document beside it. The form is specified in
//! `docs/spec/fixture-model.md`.

mod attach;
mod nodes;

use crate::roots::{FixturePrefab, fixture_model, fixture_prefab};
use sekai_convert::Converter;
use sekai_convert::model::{ModelConfig, Prefab, model_file};
use sekai_convert::{
    Asset, Content, Context, ConvertError, Cost, CostClass, Member, ObjectId, ObjectIndex,
};

use crate::model_keys::{Collision, ShaderKeys};
use crate::scripts::malformed;
use nodes::PieceNodes;

/// The converting of a piece, estimated from its textures: a few images of up to 2048 × 2048 with
/// their working copies.
const PIECE_MEMORY: u64 = 64 << 20;

/// One piece of furniture or one attachment: its prefab as a model file, with its collision.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixtureModel;

impl Converter for FixtureModel {
    fn name(&self) -> &'static str {
        "fixture-model"
    }

    fn version(&self) -> u32 {
        4
    }

    fn encoders(&self) -> sekai_convert::Encoders {
        sekai_convert::Encoders::IMAGE
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        fixture_model(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let prefab = Prefab::read(root, cx)?;
        let view = nodes::placement(&prefab, kind(root, cx)?)?;
        let collision = Collision::read(&prefab, cx)?;
        let node_keys = PieceNodes::read(&prefab, &collision, view.is_some(), cx)?;
        let config = ModelConfig {
            node_keys: Some(&node_keys),
            material_keys: Some(&ShaderKeys),
        };
        let model = model_file(&prefab, &config, cx)?;
        let mut members = vec![file("model", "glb", model.glb)];
        members.extend(model.images);
        if let Some(view) = view {
            let attach_points = serde_json::to_vec(&attach::document(&prefab, view, cx)?)
                .map_err(|e| ConvertError::new("malformed", e.to_string()))?;
            members.push(file("attach-points", "json", attach_points));
        }
        Ok(Some(Asset { members }))
    }

    fn cost(&self, _root: ObjectId, _object: &ObjectIndex, _cx: &dyn Context) -> Cost {
        Cost {
            class: CostClass::Heavy,
            memory: PIECE_MEMORY,
        }
    }
}

/// What the prefab whose root game object is `root` is, by the container path that lists it.
fn kind(root: ObjectId, cx: &dyn Context) -> Result<FixturePrefab, ConvertError> {
    cx.container(root)?
        .into_iter()
        .filter(|(_, object)| *object == root)
        .find_map(|(path, _)| fixture_prefab(&path))
        .ok_or_else(|| malformed("the root is not a prefab of a piece or an attachment"))
}

/// A member of the asset that holds finished bytes.
fn file(role: &str, extension: &str, bytes: Vec<u8>) -> Member {
    Member {
        role: role.into(),
        content: Content::File {
            extension: extension.into(),
            bytes,
        },
    }
}
