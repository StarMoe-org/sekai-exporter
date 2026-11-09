//! Inputs specific to this game.
//!
//! This crate supplies versions, manifests, bundles and game data tables to the evaluator.

#[cfg(feature = "cdn")]
pub mod cdn;
#[cfg(feature = "client")]
pub mod client;
pub mod obfuscation;
pub mod sink;
