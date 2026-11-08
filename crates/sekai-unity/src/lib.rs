//! Reads Unity files into an exact object graph.
//!
//! This crate reads UnityFS containers and serialized files, compiles type trees into reading programs, and reads objects through them. It has no C dependencies and no procedural macros, and it compiles to `wasm32`.

pub mod bundle;
pub mod cache;
pub mod class_id;
mod common_strings;
pub mod document;
mod endian;
pub mod error;
pub mod index;
pub mod layout;
pub mod read;
pub mod serialized;
pub mod source;
pub mod typetree;
pub mod value;

pub use bundle::{Bundle, Entry};
pub use cache::{BlockCache, BoundedBlockCache, NoBlockCache};
pub use endian::Endian;
pub use error::{Error, Result};
pub use layout::Layout;
pub use read::{Field, Scalar, Visitor, read_object};
pub use serialized::SerializedFile;
pub use source::{ByteSource, Chunk};
pub use typetree::{TypeTree, TypeTreeNode};
pub use value::Value;
