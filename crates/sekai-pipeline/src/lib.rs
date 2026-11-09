//! The evaluators of the derivation graph.
//!
//! The eager evaluator runs the pipeline; the lazy evaluator computes derivations on request. Both share one scheduler, which manages CPU slots, a memory budget and the concurrency of each remote endpoint.

#[cfg(feature = "client")]
pub mod client;
pub mod eval;
mod index_record;
pub mod lazy;
pub mod ledger;
pub mod metrics;
pub mod naming;
mod reads;
pub mod sched;
pub mod version;

pub use eval::{
    Evaluation, Fusion, Options, Output, evaluate, evaluate_fused, evaluate_with, parallel,
};
pub use lazy::Lazy;
pub use sekai_catalog::{BundleIndexes, Derivations, Derived, MemoryDerivations, NoBundleIndexes};
pub use version::{BundleError, BundleInput, Version};
