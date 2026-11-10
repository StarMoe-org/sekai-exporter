//! The choices a form makes when it writes a model file.

use std::collections::BTreeMap;

use serde_json::Value as Json;

use crate::{Context, ConvertError, ObjectId};

/// The choices a form makes in [`model_file`](super::model_file).
#[derive(Clone, Copy)]
pub struct ModelConfig<'a> {
    /// Whether texture coordinates are written with `v` flipped, for images stored from their
    /// first row.
    pub flip_uv: bool,
    /// Whether [`EXT_TEXTURE_WEBP`](sekai_format::gltf::EXT_TEXTURE_WEBP) is declared as an
    /// extension the file uses and requires. When it is not, each texture names its image
    /// directly.
    pub declare_webp: bool,
    /// The keys the form adds to the extras of each node, when it adds any.
    pub node_keys: Option<&'a dyn NodeKeys>,
    /// The keys the form adds to the extras of each material, when it adds any.
    pub material_keys: Option<&'a dyn MaterialKeys>,
}

/// The keys a form adds to the extras of each node, such as the identity of its source object.
pub trait NodeKeys: Sync {
    /// The keys of the node that writes the game object `object`.
    fn keys(
        &self,
        object: ObjectId,
        cx: &dyn Context,
    ) -> Result<BTreeMap<String, Json>, ConvertError>;
}

/// The keys a form adds to the extras of each material, beside the keys that say what the
/// material stores.
pub trait MaterialKeys: Sync {
    /// The keys of the material `material`.
    fn keys(
        &self,
        material: ObjectId,
        cx: &dyn Context,
    ) -> Result<BTreeMap<String, Json>, ConvertError>;
}
