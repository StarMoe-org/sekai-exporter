//! Types of the published documents: object documents, asset documents, index roots with their
//! table chunks and shards, path indexes, directory trees, version histories, the chunking they
//! share, and the documents asset forms carry as members.
//!
//! Consumers depend on this crate to read the published output.

pub mod asset;
pub mod audio;
pub mod chunks;
pub mod clip;
pub mod components;
pub mod costume;
pub mod directory_tree;
pub mod episode;
pub mod gltf;
pub mod identity;
pub mod live2d;
pub mod number;
pub mod paths;
pub mod publication;
pub mod story;
pub mod version_history;

pub use asset::{AssetDocument, AssetError, MemberFile, SourceObject, SourceResource};
