//! Reads Unity files into an exact object graph.
//!
//! This crate reads UnityFS containers and serialized files, compiles type trees into reading programs, and reads objects through them. It has no C dependencies and no procedural macros, and it compiles to `wasm32`.
