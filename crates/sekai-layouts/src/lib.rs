//! Field layout tables for objects stored without type trees, and their loader.
//!
//! The tables are compiled into the same reading programs as type trees. A [`VersionTable`] reads
//! the type tree information of a TPK file: the release layout of every engine class at every
//! Unity version the file covers, and a [`ScriptTable`] the fields of game scripts. A
//! [`PlayerLayouts`] puts the two together for the player data of a client package.

mod buffer;
mod error;
mod script;
mod table;
mod tpk;

pub use error::TableError;
pub use script::{PlayerLayouts, ScriptTable};
pub use table::VersionTable;
