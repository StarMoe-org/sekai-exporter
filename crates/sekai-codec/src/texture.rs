//! Texture decoding.
//!
//! [`decode`] is the one entry point. Each format family (uncompressed, ETC and EAC, ASTC, BC)
//! is a submodule that it dispatches to by the Unity format number.

use crate::{DecodeError, Surface};

/// Decodes the first image of `data` in the Unity texture format `format` (`m_TextureFormat`).
///
/// `data` starts with mip level 0 of the first slice; bytes after that image (further mip
/// levels and slices) are ignored. Unity stores rows from bottom to top, and the surface has
/// them from top to bottom. A format no decoder covers is reported as
/// [`DecodeError::Unsupported`].
pub fn decode(format: i32, width: u32, height: u32, data: &[u8]) -> Result<Surface, DecodeError> {
    let _ = (width, height, data);
    Err(DecodeError::Unsupported {
        what: "texture format",
        value: format.into(),
    })
}
