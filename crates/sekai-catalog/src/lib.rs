//! The content-addressed store and the catalog.
//!
//! The store keeps bytes under their SHA-256. The catalog indexes inputs, derivations, bindings and publications, and can be rebuilt from the store.

#[cfg(feature = "catalog-db")]
pub mod db;
pub mod derivation;
pub mod store;

pub use derivation::{BundleIndexes, Derivations, Derived, MemoryDerivations, NoBundleIndexes};
pub use store::{Key, LocalStore, MemoryStore, Store};
