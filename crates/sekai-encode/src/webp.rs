//! Lossy WebP through libwebp, linked statically in native builds.

use sekai_codec::Surface;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "webp";

/// The encoder version.
pub const VERSION: u32 = 1;

/// Writes `surface` as a lossy WebP. `quality` is libwebp's quality factor, from 0 to 100.
///
/// WebP images are at most 16383 pixels wide and high; larger surfaces fail.
pub fn encode(surface: &Surface, quality: f32) -> Result<Vec<u8>, EncodeError> {
    let encoder = webp::Encoder::new(
        surface.pixels(),
        webp::PixelLayout::Rgba,
        surface.width(),
        surface.height(),
    );
    let encoded = encoder
        .encode_simple(false, quality)
        .map_err(|error| EncodeError::Failed {
            encoder: NAME,
            message: format!("{error:?}"),
        })?;
    Ok(encoded.to_vec())
}
