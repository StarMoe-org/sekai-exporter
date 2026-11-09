//! Types of the published documents: object documents, asset documents, index shards, listings
//! and the documents asset forms carry as members.
//!
//! Consumers depend on this crate to read the published output.

pub mod asset;
pub mod clip;
pub mod identity;
pub mod live2d;
pub mod publication;

pub use asset::{AssetDocument, AssetError, MemberFile, SourceObject, SourceResource};
