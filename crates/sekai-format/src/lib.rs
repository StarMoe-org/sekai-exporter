//! Types of the published documents: object documents, asset documents, index shards and listings.
//!
//! Consumers depend on this crate to read the published output.

pub mod asset;
pub mod identity;
pub mod publication;

pub use asset::{AssetDocument, AssetError, MemberFile, SourceObject, SourceResource};
