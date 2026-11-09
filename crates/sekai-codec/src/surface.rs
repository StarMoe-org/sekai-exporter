use std::fmt;

use crate::DecodeError;

/// A decoded image: 8-bit RGBA with straight alpha, rows from top to bottom.
///
/// Every texture format with eight bits per channel decodes to this one form, so encoders take
/// a single input type. HDR formats decode to [`HdrSurface`].
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

/// A decoded HDR image: linear RGBA as binary16 values, rows from top to bottom.
#[derive(Clone, PartialEq, Eq)]
pub struct HdrSurface {
    width: u32,
    height: u32,
    pixels: Vec<u16>,
}

impl HdrSurface {
    /// A surface from the bit patterns of its values, four per pixel in row order.
    ///
    /// Fails when `pixels` is not `width * height * 4` values long.
    pub fn new(width: u32, height: u32, pixels: Vec<u16>) -> Result<Self, DecodeError> {
        let needed = width as usize * height as usize * 4;
        if pixels.len() != needed {
            return Err(DecodeError::Malformed {
                what: "HDR surface",
                message: format!(
                    "{width} x {height} needs {needed} values, {} given",
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

    /// The bit patterns of the binary16 values: RGBA, rows from top to bottom.
    pub fn pixels(&self) -> &[u16] {
        &self.pixels
    }

    /// The 8-bit view of the image: every value clamped to 0..=1 and scaled to 255, rounded.
    pub fn to_surface(&self) -> Surface {
        let bytes = self
            .pixels
            .iter()
            .map(|&value| (half_to_f32(value).clamp(0.0, 1.0) * 255.0 + 0.5) as u8)
            .collect();
        Surface::new(self.width, self.height, bytes).expect("four values per pixel")
    }
}

impl fmt::Debug for HdrSurface {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "HdrSurface({} x {})", self.width, self.height)
    }
}

/// The value of a binary16 bit pattern.
fn half_to_f32(half: u16) -> f32 {
    let bits = u32::from(half);
    let (sign, exponent, mantissa) = (bits >> 15, bits >> 10 & 0x1F, bits & 0x3FF);
    let magnitude = match exponent {
        0 => mantissa as f32 / 16_777_216.0,
        0x1F => f32::from_bits(0x7F80_0000 | mantissa << 13),
        _ => f32::from_bits((exponent + 112) << 23 | mantissa << 13),
    };
    if sign == 1 { -magnitude } else { magnitude }
}
