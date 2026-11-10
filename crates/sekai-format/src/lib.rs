//! Types of the published documents: object documents, asset documents, index shards, listings
//! and the documents asset forms carry as members.
//!
//! Consumers depend on this crate to read the published output.

pub mod asset;
pub mod audio;
pub mod clip;
pub mod components;
pub mod costume;
pub mod episode;
pub mod gltf;
pub mod identity;
pub mod live2d;
pub mod number;
pub mod publication;
pub mod story;

pub use asset::{AssetDocument, AssetError, MemberFile, SourceObject, SourceResource};
