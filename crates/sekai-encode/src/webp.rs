//! Lossy WebP through libwebp, linked statically in native builds.

use sekai_codec::Surface;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "webp";

/// The encoder version.
pub const VERSION: u32 = 1;

/// Writes `surface` as a lossy WebP. `quality` is libwebp's quality factor, from 0 to 100.
pub fn encode(surface: &Surface, quality: f32) -> Result<Vec<u8>, EncodeError> {
    let _ = (surface, quality);
    Err(EncodeError::not_written(NAME))
}
