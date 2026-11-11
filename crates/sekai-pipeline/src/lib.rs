//! The evaluators of the derivation graph.
//!
//! The streaming evaluator runs the pipeline bundle by bundle from a manifest, each bundle with the bundles it depends on; the lazy evaluator computes the derivations of one bundle on request, from the same units. They share one scheduler, which manages CPU slots and a memory budget.

mod canonical;
pub mod eval;
mod index_record;
pub mod isolate;
pub mod lazy;
pub mod ledger;
pub mod metrics;
pub mod naming;
pub mod objects;
mod reads;
pub mod sched;
pub mod stream;
pub mod tables;
pub mod version;

pub use eval::{Evaluation, Fusion, Options, Output, parallel};
pub use lazy::Lazy;
pub use sekai_catalog::{BundleIndexes, Derivations, Derived, MemoryDerivations, NoBundleIndexes};
pub use version::{BundleError, BundleInput, Version};
