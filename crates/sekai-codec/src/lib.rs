//! Decoding of platform payloads into neutral forms: decoded image surfaces and PCM samples.
//!
//! Every decoder is written in pure Rust and compiles to `wasm32`.

#[cfg(feature = "cri")]
pub mod cri;
#[cfg(feature = "texture")]
pub mod texture;
