//! The evaluators of the derivation graph.
//!
//! The eager evaluator runs the pipeline; the lazy evaluator computes derivations on request. Both share one scheduler, which manages CPU slots, a memory budget and the concurrency of each remote endpoint.

#[cfg(feature = "client")]
pub mod client;
pub mod eval;
pub mod metrics;
pub mod naming;
pub mod version;

pub use eval::{Evaluation, Output, evaluate, parallel};
pub use sekai_catalog::{Derivations, Derived, MemoryDerivations};
pub use version::{BundleError, BundleInput, Version};
