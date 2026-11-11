//! The parts, motions, roles and costumes of the 3D costume screen.
//!
//! [`CostumeModel`] produces the `costume-model` form and [`CostumeMotion`] the `costume-motion`
//! form; [`CostumeRoles`] produces the `costume-role` form and [`Costumes`] the `costume` form.
//! All four are specified in `docs/spec/costume.md`.

mod combination;
mod sources;
mod tables;

use sekai_convert::animation::clip_document;
use sekai_convert::model::{ModelConfig, Prefab, component_document, model_file};
use sekai_convert::{
    Asset, Content, Context, ConvertError, Converter, Cost, CostClass, Member, Object, ObjectId,
    ObjectIndex,
};

use crate::roots;

pub use combination::{CostumeRoles, Costumes};

/// The scripts of the springs and colliders the costume viewer simulates.
const SPRING_SCRIPTS: &[&str] = &[
    "ThirdParty.SpringBone/UTJ.SpringManager",
    "Assembly-CSharp/Sekai.SekaiSpringBone",
    "ThirdParty.SpringBone/UTJ.SpringSphereCollider",
    "ThirdParty.SpringBone/UTJ.SpringCapsuleCollider",
];
/// The constraints the costume viewer applies.
const CONSTRAINTS: &[&str] = &[
    "AimConstraint",
    "ParentConstraint",
    "PositionConstraint",
    "RotationConstraint",
];
/// The memory of converting a part and encoding its textures, which is known only once its
/// objects are read: a few textures of up to 2048 × 2048 with their working copies.
const PART_MEMORY: u64 = 256 << 20;

/// A part of a costume: a body, a face with its hair, or a head accessory.
#[derive(Debug, Clone, Copy, Default)]
pub struct CostumeModel;

impl Converter for CostumeModel {
    fn name(&self) -> &'static str {
        "costume-model"
    }

    fn version(&self) -> u32 {
        2
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
        roots::costume_model(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let prefab = Prefab::read(root, cx)?;
        let model = model_file(&prefab, &PART, cx)?;
        let components = component_document(&prefab, is_listed, cx)?
            .map(|document| {
                document.to_bytes().map_err(|e| {
                    ConvertError::new(
                        "malformed",
                        format!("the components cannot be written: {e}"),
                    )
                })
            })
            .transpose()?;
        let mut members = vec![file("model", "glb", model.glb)];
        members.extend(components.map(|bytes| file("components", "json", bytes)));
        members.extend(model.images);
        Ok(Some(Asset { members }))
    }

    fn cost(&self, _root: ObjectId, _object: &ObjectIndex, _cx: &dyn Context) -> Cost {
        Cost {
            class: CostClass::Heavy,
            memory: PART_MEMORY,
        }
    }
}

/// How a part's model file is written.
const PART: ModelConfig<'static> = ModelConfig {
    flip_uv: true,
    node_keys: None,
    material_keys: None,
};

/// Whether the component document of a part lists `component`.
fn is_listed(component: &Object) -> bool {
    match &component.script {
        Some(script) => SPRING_SCRIPTS.contains(&script.as_str()),
        None => CONSTRAINTS.contains(&component.class.as_str()),
    }
}

/// One motion of the costume screen.
#[derive(Debug, Clone, Copy, Default)]
pub struct CostumeMotion;

impl Converter for CostumeMotion {
    fn name(&self) -> &'static str {
        "costume-motion"
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
        roots::costume_motion(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let bytes = clip_document(root, cx)?.to_bytes().map_err(|e| {
            ConvertError::new("malformed", format!("the clip cannot be written: {e}"))
        })?;
        Ok(Some(Asset {
            members: vec![file("motion", "json", bytes)],
        }))
    }
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
