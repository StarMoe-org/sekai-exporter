//! Texture decoding.
//!
//! [`decode`] is the entry point of the formats with eight bits per channel and [`decode_hdr`]
//! of the HDR formats; [`is_hdr`] tells them apart. Each format family is a submodule dispatched
//! to by the Unity format number; a family decodes to RGBA in stored row order.

mod astc;
mod crunch;
mod etc;

use crate::{DecodeError, HdrSurface, Surface};

/// Decodes the first image of `data` in the Unity texture format `format` (`m_TextureFormat`).
///
/// `data` starts with mip level 0 of the first slice; bytes after that image (further mip
/// levels and slices) are ignored. Unity stores rows from bottom to top, and the surface has
/// them from top to bottom. A format no decoder covers is reported as
/// [`DecodeError::Unsupported`].
pub fn decode(format: i32, width: u32, height: u32, data: &[u8]) -> Result<Surface, DecodeError> {
    let (w, h) = dimensions(width, height)?;
    let mut pixels = match format {
        // The colour is zero, as in the 32-bit pixels Unity reads from this format.
        1 => texels(1, w, h, data, "Alpha8 image", |t| [0, 0, 0, t[0]]),
        3 => texels(3, w, h, data, "RGB24 image", |t| [t[0], t[1], t[2], 255]),
        4 => texels(4, w, h, data, "RGBA32 image", |t| [t[0], t[1], t[2], t[3]]),
        7 => texels(2, w, h, data, "RGB565 image", |t| {
            let v = u16::from_le_bytes([t[0], t[1]]);
            let (r, g, b) = ((v >> 11) as u8, (v >> 5 & 63) as u8, (v & 31) as u8);
            [r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2, 255]
        }),
        29 => crunch::decode(w, h, data),
        34 => etc::decode_etc1(w, h, data),
        47 => etc::decode_etc2_rgba8(w, h, data),
        48 => astc::decode(4, 4, w, h, data),
        49 => astc::decode(5, 5, w, h, data),
        50 => astc::decode(6, 6, w, h, data),
        51 => astc::decode(8, 8, w, h, data),
        52 => astc::decode(10, 10, w, h, data),
        53 => astc::decode(12, 12, w, h, data),
        _ => Err(DecodeError::Unsupported {
            what: "texture format",
            value: format.into(),
        }),
    }?;
    flip_rows(&mut pixels, w);
    Surface::new(width, height, pixels)
}

/// Whether the Unity texture format `format` decodes with [`decode_hdr`].
pub fn is_hdr(format: i32) -> bool {
    matches!(format, 66..=71)
}

/// Decodes the first image of `data` in the HDR Unity texture format `format`.
///
/// `data` and the row order are as in [`decode`]. A format that is not HDR is reported as
/// [`DecodeError::Unsupported`].
pub fn decode_hdr(
    format: i32,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<HdrSurface, DecodeError> {
    let (w, h) = dimensions(width, height)?;
    let footprint = match format {
        66 => 4,
        67 => 5,
        68 => 6,
        69 => 8,
        70 => 10,
        71 => 12,
        _ => {
            return Err(DecodeError::Unsupported {
                what: "HDR texture format",
                value: format.into(),
            });
        }
    };
    let mut pixels = astc::decode_hdr(footprint, footprint, w, h, data)?;
    flip_rows(&mut pixels, w);
    HdrSurface::new(width, height, pixels)
}

fn dimensions(width: u32, height: u32) -> Result<(usize, usize), DecodeError> {
    if width == 0 || height == 0 {
        return Err(DecodeError::Malformed {
            what: "texture",
            message: format!("empty image of {width} x {height}"),
        });
    }
    Ok((width as usize, height as usize))
}

/// Reverses the rows of an RGBA image.
fn flip_rows<T>(pixels: &mut [T], width: usize) {
    let stride = width * 4;
    let rows = pixels.len() / stride;
    for y in 0..rows / 2 {
        let (top, bottom) = pixels.split_at_mut((rows - 1 - y) * stride);
        top[y * stride..(y + 1) * stride].swap_with_slice(&mut bottom[..stride]);
    }
}

/// Expands an uncompressed image of `size` bytes per texel into RGBA in stored row order.
fn texels(
    size: usize,
    width: usize,
    height: usize,
    data: &[u8],
    what: &'static str,
    expand: impl Fn(&[u8]) -> [u8; 4],
) -> Result<Vec<u8>, DecodeError> {
    decode_blocks((1, 1, size), width, height, data, what, |texel, out| {
        out(&[expand(texel)]);
        Ok(())
    })
}

/// Decodes a block-compressed image into RGBA values of type `T` in stored row order.
///
/// `shape` is the block width, height and size in bytes. For each block in stored order,
/// `decode_block` gets its bytes and a function to call with its texels; texels beyond the edge
/// of the image are dropped.
fn decode_blocks<T, F>(
    shape: (usize, usize, usize),
    width: usize,
    height: usize,
    data: &[u8],
    what: &'static str,
    mut decode_block: F,
) -> Result<Vec<T>, DecodeError>
where
    T: Copy + Default,
    F: FnMut(&[u8], &mut dyn FnMut(&[[T; 4]])) -> Result<(), DecodeError>,
{
    let (block_width, block_height, block_bytes) = shape;
    let columns = width.div_ceil(block_width);
    let rows = height.div_ceil(block_height);
    let too_large = || DecodeError::Malformed {
        what,
        message: format!("{width} x {height} is too large"),
    };
    let needed = columns
        .checked_mul(rows)
        .and_then(|blocks| blocks.checked_mul(block_bytes))
        .ok_or_else(too_large)?;
    if data.len() < needed {
        return Err(DecodeError::Truncated {
            what,
            needed,
            available: data.len(),
        });
    }
    let size = width
        .checked_mul(height)
        .and_then(|texels| texels.checked_mul(4))
        .ok_or_else(too_large)?;
    let mut pixels = vec![T::default(); size];
    let stride = width * 4;
    for (index, block) in data[..needed].chunks_exact(block_bytes).enumerate() {
        let x0 = index % columns * block_width;
        let y0 = index / columns * block_height;
        let visible_width = block_width.min(width - x0);
        let visible_rows = block_height.min(height - y0);
        decode_block(block, &mut |texels| {
            for y in 0..visible_rows {
                let source = &texels[y * block_width..][..visible_width];
                let start = (y0 + y) * stride + x0 * 4;
                pixels[start..][..visible_width * 4].copy_from_slice(source.as_flattened());
            }
        })?;
    }
    Ok(pixels)
}
