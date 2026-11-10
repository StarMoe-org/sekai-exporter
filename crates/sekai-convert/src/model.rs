//! Prefabs as model files and component documents.
//!
//! [`Prefab::read`] reads the game objects of a prefab. [`model_file`] writes them as a model
//! file, specified in `docs/spec/model.md`, and [`component_document`] lists their components
//! as a component document, specified in `docs/spec/component-documents.md`. Asset forms that
//! carry 3D models use both for their members and choose which prefabs are roots and which
//! components are listed.

mod components;
mod config;
mod file;
mod glb;
mod material;
mod mesh;
mod prefab;
mod read;

pub use components::component_document;
pub use config::{ModelConfig, NodeKeys};
pub use file::{ModelFile, model_file};
pub use prefab::Prefab;
