use std::fmt;

use crate::DecodeError;

/// A decoded image: 8-bit RGBA with straight alpha, rows from top to bottom.
///
/// Every texture format decodes to this one form, so encoders take a single input type.
/// Formats with more than eight bits per channel are not represented and are reported as
/// unsupported by their decoder.
#[derive(Clone, PartialEq, Eq)]
pub struct Surface {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

impl Surface {
    /// A surface from its pixels, four bytes per pixel in row order.
    ///
    /// Fails when `pixels` is not `width * height * 4` bytes long.
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Result<Self, DecodeError> {
        let needed = width as usize * height as usize * 4;
        if pixels.len() != needed {
            return Err(DecodeError::Malformed {
                what: "surface",
                message: format!(
                    "{width} x {height} needs {needed} bytes, {} given",
                    pixels.len()
                ),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// The width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixels: RGBA, rows from top to bottom.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    /// The pixels, taken out of the surface.
    pub fn into_pixels(self) -> Vec<u8> {
        self.pixels
    }
}

impl fmt::Debug for Surface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Surface({} x {})", self.width, self.height)
    }
}
