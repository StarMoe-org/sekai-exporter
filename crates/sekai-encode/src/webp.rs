//! Lossy WebP through libwebp, linked statically in native builds.

use sekai_codec::Surface;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "webp";

/// The encoder version.
pub const VERSION: u32 = 2;

/// The libwebp method, which trades speed for size from 0, the fastest, to 6.
const METHOD: i32 = 5;

fn failed(message: String) -> EncodeError {
    EncodeError::Failed {
        encoder: NAME,
        message,
    }
}

/// Writes `surface` as a lossy WebP with libwebp method 5. `quality` is libwebp's quality
/// factor, from 0 to 100.
///
/// WebP images are at most 16383 pixels wide and high; larger surfaces fail.
pub fn encode(surface: &Surface, quality: f32) -> Result<Vec<u8>, EncodeError> {
    let encoder = webp::Encoder::new(
        surface.pixels(),
        webp::PixelLayout::Rgba,
        surface.width(),
        surface.height(),
    );
    let mut config = webp::WebPConfig::new()
        .map_err(|()| failed("libwebp gives no default configuration".to_owned()))?;
    config.quality = quality;
    config.method = METHOD;
    config.alpha_compression = 1;
    let encoded = encoder
        .encode_advanced(&config)
        .map_err(|error| failed(format!("{error:?}")))?;
    Ok(encoded.to_vec())
}
