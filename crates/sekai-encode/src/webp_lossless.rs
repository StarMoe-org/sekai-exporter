//! Lossless WebP, the canonical image form. Written in pure Rust, so native and WebAssembly
//! builds give the same bytes.

use image_webp::{ColorType, WebPEncoder};
use sekai_codec::Surface;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "webp-lossless";

/// The encoder version.
pub const VERSION: u32 = 2;

/// Writes `surface` as a lossless WebP.
///
/// WebP images are at most 16384 pixels wide and high; larger surfaces fail.
pub fn encode(surface: &Surface) -> Result<Vec<u8>, EncodeError> {
    let mut out = Vec::new();
    WebPEncoder::new(&mut out)
        .encode(
            surface.pixels(),
            surface.width(),
            surface.height(),
            ColorType::Rgba8,
        )
        .map_err(|e| EncodeError::Failed {
            encoder: NAME,
            message: e.to_string(),
        })?;
    Ok(out)
}
