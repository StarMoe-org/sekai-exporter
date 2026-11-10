//! Prefabs as model files and component documents.
//!
//! [`Prefab::read`] reads the game objects of a prefab. [`model_file`] writes them as a model
//! file, specified in `docs/spec/model.md`, and [`component_document`] lists their components
//! as a component document, specified in `docs/spec/component-documents.md`. Asset forms that
//! carry 3D models use both for their members and choose which prefabs are roots and which
//! components are listed.
//!
//! [`ParsedShader::read`] reads the shader a material names, and [`mesh_triangles`] the triangles
//! of a mesh that no renderer need draw, such as the mesh of a collider.

mod components;
mod config;
mod file;
mod glb;
mod material;
mod mesh;
mod prefab;
mod read;
mod shader;

pub use components::component_document;
pub use config::{MaterialKeys, ModelConfig, NodeKeys};
pub use file::{ModelFile, model_file};
pub use mesh::{MeshTriangles, mesh_triangles};
pub use prefab::{Node, Pose, Prefab};
pub use shader::{Blend, ParsedShader, Pass, ShaderProperty, StateValue, SubShader};
