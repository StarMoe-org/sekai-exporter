//! The content-addressed store and the catalog.
//!
//! The store keeps bytes under their SHA-256. The catalog indexes inputs, derivations, bindings,
//! publications and what each destination holds, and its derivations can be rebuilt from the
//! store.

#[cfg(feature = "catalog-db")]
pub mod db;
pub mod derivation;
pub mod holdings;
pub mod store;

pub use derivation::{
    BundleIndexes, Derivations, Derived, MemoryDerivations, MemoryUnitRecords, NoBundleIndexes,
    UnitRecords,
};
pub use holdings::{Holdings, MemoryHoldings, NoHoldings, NoRemote, Objects, Remote};
pub use store::{Key, LocalStore, MemoryStore, Store};
