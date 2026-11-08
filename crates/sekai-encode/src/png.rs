//! PNG.

use sekai_codec::Surface;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "png";

/// The encoder version.
pub const VERSION: u32 = 1;

/// Writes `surface` as an 8-bit RGBA PNG.
pub fn encode(surface: &Surface) -> Result<Vec<u8>, EncodeError> {
    let _ = (surface,);
    Err(EncodeError::not_written(NAME))
}
