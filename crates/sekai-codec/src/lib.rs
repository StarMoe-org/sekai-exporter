//! Decoding of platform payloads into neutral forms: decoded image surfaces and PCM samples.
//!
//! Every decoder is written in pure Rust and compiles to `wasm32`. Decoders read bytes and
//! return a [`Surface`] or [`Pcm`]; they know nothing about Unity objects, files or encoders.

mod error;
mod pcm;
mod surface;

#[cfg(feature = "cri")]
pub mod cri;
#[cfg(feature = "texture")]
pub mod texture;

pub use error::DecodeError;
pub use pcm::Pcm;
pub use surface::Surface;
