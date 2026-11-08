//! The evaluators of the derivation graph.
//!
//! The eager evaluator runs the pipeline; the lazy evaluator computes derivations on request. Both share one scheduler, which manages CPU slots, a memory budget and the concurrency of each remote endpoint.

#[cfg(feature = "client")]
pub mod client;
