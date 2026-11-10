//! What the MySekai model forms add to model files: the keys of nodes and materials, and the
//! collision of game objects.

mod collision;
mod materials;
mod nodes;
mod scripts;

pub(crate) use collision::{ColliderMeshes, Collision};
pub(crate) use materials::ShaderKeys;
pub(crate) use nodes::{source_keys, source_object};
pub(crate) use scripts::is_script;
