//! Lossless WebP, the canonical image form. Written in pure Rust, so native and WebAssembly builds give the same bytes.

use sekai_codec::Surface;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "webp-lossless";

/// The encoder version.
pub const VERSION: u32 = 1;

/// Writes `surface` as a lossless WebP.
pub fn encode(surface: &Surface) -> Result<Vec<u8>, EncodeError> {
    let _ = (surface,);
    Err(EncodeError::not_written(NAME))
}
