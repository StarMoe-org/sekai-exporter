//! PVRTC 1 decoding.
//!
//! Ported from the reference decoder of Imagination Technologies, the owner of the format
//! (`PVRTDecompress.cpp` in the PowerVR SDK). An image is a grid of 64-bit words in Morton order.
//! Each word holds two colours, A and B, and the modulation of a block of 8 x 4 pixels at two
//! bits per pixel or 4 x 4 pixels at four. Both colours of a pixel are bilinear between the four
//! words whose block centres surround it, wrapping at the edges of the image, and its modulation
//! mixes B into A in eighths.
//!
//! The RGB formats have the base format RGB in `IMG_texture_compression_pvrtc`, so their alpha
//! samples as one whatever the words store.
//!
//! The decoding is translated to Rust from `PVRTDecompress.cpp`, altered to compute each pixel
//! from the words around it:
//!
//! Copyright (c) Imagination Technologies Ltd.
//!
//! Permission is hereby granted, free of charge, to any person obtaining a copy of this software
//! and associated documentation files (the "Software"), to deal in the Software without
//! restriction, including without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
//! Software is furnished to do so, subject to the following conditions:
//!
//! The above copyright notice and this permission notice shall be included in all copies or
//! substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
//! BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
//! NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
//! OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

use crate::DecodeError;

/// The two bit rates of PVRTC 1.
#[derive(Clone, Copy)]
pub(super) enum Rate {
    Two,
    Four,
}

impl Rate {
    /// The width of the block of a word, in pixels. Every block is four pixels high.
    fn block_width(self) -> usize {
        match self {
            Rate::Two => 8,
            Rate::Four => 4,
        }
    }
}

/// Whether the alpha the words store is sampled.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Channels {
    Rgb,
    Rgba,
}

/// The modulation values of the codes of two bits, in eighths of colour B.
const EIGHTHS: [u32; 4] = [0, 3, 5, 8];

/// Decodes PVRTC 1 into RGBA in stored row order.
pub(super) fn decode(
    rate: Rate,
    channels: Channels,
    width: usize,
    height: usize,
    data: &[u8],
) -> Result<Vec<u8>, DecodeError> {
    let image = Image::read(rate, width, height, data)?;
    let mut pixels = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            let mut texel = image.texel(x, y);
            if channels == Channels::Rgb {
                texel[3] = 255;
            }
            pixels.extend(texel);
        }
    }
    Ok(pixels)
}

/// How much of colour B a pixel takes.
#[derive(Clone, Copy, Default)]
struct Modulation {
    eighths: u32,
    /// The pixel is transparent, whatever the alpha of its colours.
    punch_through: bool,
}

impl Modulation {
    fn of(eighths: u32) -> Modulation {
        Modulation {
            eighths,
            punch_through: false,
        }
    }

    /// The modulation of a code of a four-bit word, in the mode its colour half selects.
    fn four_bit(code: u32, punch_through_mode: bool) -> Modulation {
        match (punch_through_mode, code) {
            (true, 2) => Modulation {
                eighths: 4,
                punch_through: true,
            },
            (true, code) => Modulation::of([0, 4, 4, 8][code as usize]),
            (false, code) => Modulation::of(EIGHTHS[code as usize]),
        }
    }
}

/// The neighbours an interpolated pixel of a two-bit word averages.
#[derive(Clone, Copy)]
enum Axes {
    Both,
    Horizontal,
    Vertical,
}

/// A word, unpacked.
struct Word {
    /// Colours A and B, with five bits per colour channel and four of alpha.
    colors: [[u32; 4]; 2],
    /// For a two-bit word that stores every other pixel, how the others are interpolated.
    interpolation: Option<Axes>,
}

/// The modulation half of a word, read from its lowest bits up.
struct Codes(u32);

impl Codes {
    fn take(&mut self, bits: u32) -> u32 {
        let code = self.0 & ((1 << bits) - 1);
        self.0 >>= bits;
        code
    }
}

impl Word {
    /// Unpacks the word of block `column`, `row` from its modulation and colour halves, and
    /// stores the modulation of its pixels. The low bit of the colour half selects the mode of
    /// the modulation.
    fn unpack(
        rate: Rate,
        (modulation, color): (u32, u32),
        (column, row): (usize, usize),
        stored: &mut Stored,
    ) -> Word {
        let mode = color & 1 == 1;
        let (interpolation, codes) = match rate {
            Rate::Two if mode => {
                let (axes, codes) = interpolated_codes(modulation);
                (Some(axes), codes)
            }
            _ => (None, modulation),
        };
        let mut codes = Codes(codes);
        let block_width = rate.block_width();
        for y in 0..4 {
            for x in 0..block_width {
                let modulation = match (rate, interpolation) {
                    (Rate::Two, None) => Modulation::of(codes.take(1) * 8),
                    (Rate::Two, Some(_)) if (x ^ y) & 1 == 1 => continue,
                    (Rate::Two, Some(_)) => Modulation::of(EIGHTHS[codes.take(2) as usize]),
                    (Rate::Four, _) => Modulation::four_bit(codes.take(2), mode),
                };
                stored.set(column * block_width + x, row * 4 + y, modulation);
            }
        }
        Word {
            colors: [color_a(color), color_b(color)],
            interpolation,
        }
    }
}

/// The modulation each pixel's word stores for it, over the padded image. A pixel its word
/// interpolates has none stored and holds the default.
struct Stored {
    width: usize,
    height: usize,
    /// By row.
    pixels: Vec<Modulation>,
}

impl Stored {
    fn at(&self, x: usize, y: usize) -> Modulation {
        self.pixels[y * self.width + x]
    }

    fn set(&mut self, x: usize, y: usize, modulation: Modulation) {
        self.pixels[y * self.width + x] = modulation;
    }

    /// The modulation of an interpolated pixel, from its neighbours along `axes`, wrapping at
    /// the edges. The neighbours are stored ones, in its own word or the next.
    fn interpolated(&self, x: usize, y: usize, axes: Axes) -> Modulation {
        let eighths = |x: usize, y: usize| self.at(x, y).eighths;
        let (west, east) = ((x + self.width - 1) % self.width, (x + 1) % self.width);
        let (north, south) = ((y + self.height - 1) % self.height, (y + 1) % self.height);
        let horizontal = eighths(west, y) + eighths(east, y);
        let vertical = eighths(x, north) + eighths(x, south);
        Modulation::of(match axes {
            Axes::Both => (horizontal + vertical + 2) / 4,
            Axes::Horizontal => horizontal.div_ceil(2),
            Axes::Vertical => vertical.div_ceil(2),
        })
    }
}

/// An image padded to the smallest size the format stores, with its words unpacked.
struct Image {
    rate: Rate,
    columns: usize,
    rows: usize,
    /// The words, by row.
    words: Vec<Word>,
    stored: Stored,
}

impl Image {
    fn read(rate: Rate, width: usize, height: usize, data: &[u8]) -> Result<Image, DecodeError> {
        let block_width = rate.block_width();
        let (width, height) = (width.max(2 * block_width), height.max(8));
        if !width.is_power_of_two() || !height.is_power_of_two() {
            return Err(DecodeError::Malformed {
                what: "PVRTC image",
                message: format!("{width} x {height} is not a power of two in each dimension"),
            });
        }
        let (columns, rows) = (width / block_width, height / 4);
        let needed = columns * rows * 8;
        if data.len() < needed {
            return Err(DecodeError::Truncated {
                what: "PVRTC image",
                needed,
                available: data.len(),
            });
        }
        let mut stored = Stored {
            width,
            height,
            pixels: vec![Modulation::default(); width * height],
        };
        let words = (0..rows)
            .flat_map(|row| (0..columns).map(move |column| (column, row)))
            .map(|(column, row)| {
                let at = morton(columns, rows, column, row) * 8;
                let half = |offset: usize| {
                    u32::from_le_bytes(std::array::from_fn(|k| data[at + offset + k]))
                };
                Word::unpack(rate, (half(0), half(4)), (column, row), &mut stored)
            })
            .collect();
        Ok(Image {
            rate,
            columns,
            rows,
            words,
            stored,
        })
    }

    /// The word at `column`, `row`, wrapping at the edges.
    fn word(&self, column: usize, row: usize) -> &Word {
        &self.words[(row % self.rows) * self.columns + column % self.columns]
    }

    /// The pixel at `x`, `y`, with eight bits per channel.
    fn texel(&self, x: usize, y: usize) -> [u8; 4] {
        let Modulation {
            eighths,
            punch_through,
        } = self.modulation(x, y);
        let [a, b] = self.colors(x, y);
        let mut texel =
            std::array::from_fn(|c| ((a[c] * (8 - eighths) + b[c] * eighths) / 8) as u8);
        if punch_through {
            texel[3] = 0;
        }
        texel
    }

    fn modulation(&self, x: usize, y: usize) -> Modulation {
        let block_width = self.rate.block_width();
        match self.word(x / block_width, y / 4).interpolation {
            Some(axes) if (x ^ y) & 1 == 1 => self.stored.interpolated(x, y, axes),
            _ => self.stored.at(x, y),
        }
    }

    /// Colours A and B at `x`, `y`, with eight bits per channel: bilinear between the four
    /// words whose block centres surround the pixel.
    fn colors(&self, x: usize, y: usize) -> [[u32; 4]; 2] {
        let block_width = self.rate.block_width();
        let from_west = x + self.stored.width - block_width / 2;
        let from_north = y + self.stored.height - 2;
        let (column, row) = (from_west / block_width, from_north / 4);
        let (i, j) = ((from_west % block_width) as u32, (from_north % 4) as u32);
        // The weights sum to four times the block width, which the scale brings to 32.
        let width = block_width as u32;
        let scale = 8 / width;
        let corners = [
            (0, 0, (width - i) * (4 - j)),
            (1, 0, i * (4 - j)),
            (0, 1, (width - i) * j),
            (1, 1, i * j),
        ];
        std::array::from_fn(|which| {
            let mut sum = [0; 4];
            for (east, south, weight) in corners {
                let word = self.word(column + east, row + south);
                for (total, value) in sum.iter_mut().zip(word.colors[which]) {
                    *total += value * weight * scale;
                }
            }
            let [r, g, b, a] = sum;
            let widen = |s: u32| (s >> 7) + (s >> 2);
            [widen(r), widen(g), widen(b), (a >> 5) + (a >> 1)]
        })
    }
}

/// The modulation codes of a two-bit word that stores every other pixel, with the axes its
/// other pixels are interpolated along.
///
/// The low bit of the first stored pixel selects the axes and then takes the value of its high
/// bit. When it selects one axis, the low bit of the centre pixel (the eleventh stored) tells
/// which, and then takes the value of its high bit.
fn interpolated_codes(mut codes: u32) -> (Axes, u32) {
    let axes = if codes & 1 == 0 {
        Axes::Both
    } else {
        let axes = if codes >> 20 & 1 == 1 {
            Axes::Vertical
        } else {
            Axes::Horizontal
        };
        codes = copy_bit(codes, 21, 20);
        axes
    };
    (axes, copy_bit(codes, 1, 0))
}

/// `value` with bit `to` set to bit `from`.
fn copy_bit(value: u32, from: u32, to: u32) -> u32 {
    value & !(1 << to) | (value >> from & 1) << to
}

/// Colour A of a word's colour half: opaque RGB 5:5:4, or ARGB 3:4:4:3, widened to five bits per
/// colour channel and four of alpha.
fn color_a(color: u32) -> [u32; 4] {
    if color & 0x8000 != 0 {
        [
            color >> 10 & 31,
            color >> 5 & 31,
            color & 0x1e | (color & 0x1e) >> 4,
            15,
        ]
    } else {
        [
            (color & 0xf00) >> 7 | (color & 0xf00) >> 11,
            (color & 0xf0) >> 3 | (color & 0xf0) >> 7,
            (color & 0xe) << 1 | (color & 0xe) >> 2,
            (color & 0x7000) >> 11,
        ]
    }
}

/// Colour B of a word's colour half: opaque RGB 5:5:5, or ARGB 3:4:4:4, widened as colour A.
fn color_b(color: u32) -> [u32; 4] {
    if color & 0x8000_0000 != 0 {
        [color >> 26 & 31, color >> 21 & 31, color >> 16 & 31, 15]
    } else {
        [
            (color & 0xf00_0000) >> 23 | (color & 0xf00_0000) >> 27,
            (color & 0xf0_0000) >> 19 | (color & 0xf0_0000) >> 23,
            (color & 0xf_0000) >> 15 | (color & 0xf_0000) >> 19,
            (color & 0x7000_0000) >> 27,
        ]
    }
}

/// The position of word `column`, `row` in the Morton order of a grid of `columns` x `rows`
/// words, both powers of two. The bits of the row and the column interleave, the row's lowest,
/// up to the smaller dimension, and the remaining bits of the longer one follow.
fn morton(columns: usize, rows: usize, column: usize, row: usize) -> usize {
    let (shorter, rest) = if rows < columns {
        (rows, column)
    } else {
        (columns, row)
    };
    let bits = shorter.trailing_zeros();
    let interleaved: usize = (0..bits)
        .map(|bit| (row >> bit & 1) << (2 * bit) | (column >> bit & 1) << (2 * bit + 1))
        .sum();
    interleaved | (rest >> bits) << (2 * bits)
}
