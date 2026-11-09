//! Field layout tables for objects stored without type trees, and their loader.
//!
//! The tables are compiled into the same reading programs as type trees. A [`VersionTable`] reads
//! the type tree information of a TPK file: the release layout of every engine class at every
//! Unity version the file covers.

mod buffer;
mod error;
mod table;
mod tpk;

pub use error::TpkError;
pub use table::VersionTable;
