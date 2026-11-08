//! PNG.

use sekai_codec::Surface;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "png";

/// The encoder version.
pub const VERSION: u32 = 1;

/// Writes `surface` as an 8-bit RGBA PNG.
///
/// The compression setting is fixed, so the same surface always gives the same bytes.
pub fn encode(surface: &Surface) -> Result<Vec<u8>, EncodeError> {
    let failed = |e: png::EncodingError| EncodeError::Failed {
        encoder: NAME,
        message: e.to_string(),
    };
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, surface.width(), surface.height());
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Balanced);
    let mut writer = encoder.write_header().map_err(failed)?;
    writer.write_image_data(surface.pixels()).map_err(failed)?;
    writer.finish().map_err(failed)?;
    Ok(out)
}
