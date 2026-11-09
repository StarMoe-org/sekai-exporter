//! Crunch-compressed DXT5, in the variant of the format that Unity writes.
//!
//! The stream layout follows `crn_decomp.h` of Unity's crunch library. A file holds shared
//! palettes of endpoints and selectors and, per mip level, Huffman-coded indices into them. The
//! indices are expanded to BC3 blocks, which are decoded with the formulas of the S3TC chapter
//! of the Khronos Data Format Specification.
//!
//! The unpacking is translated to Rust from `crn_decomp.h`, altered to decode only this format:
//!
//! Copyright (c) 2010-2016 Richard Geldreich, Jr. and Binomial LLC
//!
//! This software is provided 'as-is', without any express or implied warranty. In no event will
//! the authors be held liable for any damages arising from the use of this software.
//!
//! Permission is granted to anyone to use this software for any purpose, including commercial
//! applications, and to alter it and redistribute it freely, subject to the following
//! restrictions:
//!
//! 1. The origin of this software must not be misrepresented; you must not claim that you wrote
//!    the original software. If you use this software in a product, an acknowledgment in the
//!    product documentation would be appreciated but is not required.
//! 2. Altered source versions must be plainly marked as such, and must not be misrepresented as
//!    being the original software.
//! 3. This notice may not be removed or altered from any source distribution.

use super::decode_blocks;
use crate::DecodeError;

const SIGNATURE: u32 = 0x4878;
const FORMAT_DXT5: u32 = 2;
/// The offset of the level offsets, and the size of the header without them.
const LEVEL_OFFSETS: usize = 70;
const MAX_CODE_SIZE: usize = 16;
const MAX_SYMBOLS: u32 = 8192;
const MAX_CODE_LENGTH_CODES: u32 = 21;
const SMALL_ZERO_RUN: u32 = 17;
const LARGE_ZERO_RUN: u32 = 18;
const SMALL_REPEAT: u32 = 19;
const LARGE_REPEAT: u32 = 20;
/// The order in which the code lengths of the code length codes are stored.
const CODE_LENGTH_ORDER: [u32; 21] = [
    17, 18, 19, 20, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15, 16,
];
const DXT5_FROM_LINEAR: [u32; 8] = [0, 2, 3, 4, 5, 6, 7, 1];

fn malformed(message: &str) -> DecodeError {
    DecodeError::Malformed {
        what: "crunch file",
        message: message.to_owned(),
    }
}

fn truncated(needed: usize, available: usize) -> DecodeError {
    DecodeError::Truncated {
        what: "crunch file",
        needed,
        available,
    }
}

/// Big-endian numbers of the header.
fn number(data: &[u8], offset: usize, size: usize) -> u32 {
    data[offset..offset + size]
        .iter()
        .fold(0, |value, &byte| value << 8 | u32::from(byte))
}

/// A palette section of the header.
#[derive(Clone, Copy)]
struct Section {
    offset: usize,
    size: usize,
    count: usize,
}

fn section(header: &[u8], offset: usize) -> Section {
    Section {
        offset: number(header, offset, 3) as usize,
        size: number(header, offset + 3, 3) as usize,
        count: number(header, offset + 6, 2) as usize,
    }
}

/// Reads bits from the most significant end of each byte. Codes may look past the end of the
/// data, but no code may use bits past it.
struct Bits<'a> {
    data: &'a [u8],
    position: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8], offset: usize, size: usize) -> Result<Self, DecodeError> {
        let end = offset.saturating_add(size);
        let data = data
            .get(offset..end)
            .ok_or_else(|| truncated(end, data.len()))?;
        Ok(Self { data, position: 0 })
    }

    /// The next sixteen bits, without using them.
    fn peek(&self) -> u32 {
        let byte = self.position / 8;
        let at = |index: usize| u32::from(self.data.get(byte + index).copied().unwrap_or(0));
        let window = at(0) << 16 | at(1) << 8 | at(2);
        (window >> (8 - self.position % 8)) & 0xFFFF
    }

    fn skip(&mut self, count: u32) -> Result<(), DecodeError> {
        self.position += count as usize;
        if self.position > self.data.len() * 8 {
            return Err(truncated(self.position.div_ceil(8), self.data.len()));
        }
        Ok(())
    }

    /// Reads up to sixteen bits.
    fn read(&mut self, count: u32) -> Result<u32, DecodeError> {
        let value = self.peek() >> (16 - count);
        self.skip(count)?;
        Ok(value)
    }
}

/// A canonical Huffman code.
#[derive(Default)]
struct Huffman {
    /// By code size: the first code, the number of codes and the index of the first symbol.
    first: [u32; MAX_CODE_SIZE + 1],
    count: [u32; MAX_CODE_SIZE + 1],
    start: [u32; MAX_CODE_SIZE + 1],
    symbols: Vec<u16>,
}

impl Huffman {
    fn new(sizes: &[u8]) -> Result<Self, DecodeError> {
        let mut code = Self::default();
        for &size in sizes {
            if usize::from(size) > MAX_CODE_SIZE {
                return Err(malformed("a code is longer than sixteen bits"));
            }
            code.count[usize::from(size)] += 1;
        }
        code.count[0] = 0;
        let (mut next, mut used) = (0u32, 0);
        for size in 1..=MAX_CODE_SIZE {
            code.first[size] = next;
            code.start[size] = used;
            next = (next + code.count[size]) << 1;
            used += code.count[size];
        }
        code.symbols = vec![0; used as usize];
        let mut position = code.start;
        for (symbol, &size) in sizes.iter().enumerate() {
            if size != 0 {
                code.symbols[position[usize::from(size)] as usize] = symbol as u16;
                position[usize::from(size)] += 1;
            }
        }
        Ok(code)
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Result<u32, DecodeError> {
        let window = bits.peek();
        for size in 1..=MAX_CODE_SIZE {
            let offset = (window >> (16 - size)).wrapping_sub(self.first[size]);
            if offset < self.count[size] {
                bits.skip(size as u32)?;
                return Ok(self.symbols[(self.start[size] + offset) as usize].into());
            }
        }
        Err(malformed("a code matches no symbol"))
    }

    /// Reads the description of a code.
    fn read(bits: &mut Bits<'_>) -> Result<Self, DecodeError> {
        let used = bits.read(32 - MAX_SYMBOLS.leading_zeros())? as usize;
        if used == 0 {
            return Ok(Self::default());
        }
        let sent = bits.read(5)?;
        if !(1..=MAX_CODE_LENGTH_CODES).contains(&sent) {
            return Err(malformed("bad number of code length codes"));
        }
        let mut length_sizes = [0; MAX_CODE_LENGTH_CODES as usize];
        for &code in &CODE_LENGTH_ORDER[..sent as usize] {
            length_sizes[code as usize] = bits.read(3)? as u8;
        }
        let lengths = Self::new(&length_sizes)?;
        let mut sizes = vec![0; used];
        let mut filled = 0;
        while filled < used {
            let code = lengths.decode(bits)?;
            if code <= 16 {
                sizes[filled] = code as u8;
                filled += 1;
                continue;
            }
            let run = match code {
                SMALL_ZERO_RUN => bits.read(3)? + 3,
                LARGE_ZERO_RUN => bits.read(7)? + 11,
                SMALL_REPEAT => bits.read(2)? + 3,
                LARGE_REPEAT => bits.read(6)? + 7,
                _ => return Err(malformed("bad code length code")),
            } as usize;
            if run > used - filled {
                return Err(malformed("a run passes the end of the code lengths"));
            }
            if code == SMALL_REPEAT || code == LARGE_REPEAT {
                let previous = match filled {
                    0 => 0,
                    _ => sizes[filled - 1],
                };
                if previous == 0 {
                    return Err(malformed("a repeat of no code length"));
                }
                sizes[filled..filled + run].fill(previous);
            }
            filled += run;
        }
        Self::new(&sizes)
    }
}

fn color_endpoints(file: &[u8], section: Section) -> Result<Vec<u32>, DecodeError> {
    let mut bits = Bits::new(file, section.offset, section.size)?;
    let channel5 = Huffman::read(&mut bits)?;
    let channel6 = Huffman::read(&mut bits)?;
    let mut value = [0u32; 6];
    (0..section.count)
        .map(|_| {
            for (index, current) in value.iter_mut().enumerate() {
                let (code, mask) = if index % 3 == 1 {
                    (&channel6, 63)
                } else {
                    (&channel5, 31)
                };
                *current = (*current + code.decode(&mut bits)?) & mask;
            }
            let [a, b, c, d, e, f] = value;
            Ok(c | b << 5 | a << 11 | f << 16 | e << 21 | d << 27)
        })
        .collect()
}

fn color_selectors(file: &[u8], section: Section) -> Result<Vec<u32>, DecodeError> {
    let mut bits = Bits::new(file, section.offset, section.size)?;
    let code = Huffman::read(&mut bits)?;
    let mut s = 0u32;
    (0..section.count)
        .map(|_| {
            for shift in (0..32).step_by(4) {
                s ^= code.decode(&mut bits)? << shift;
            }
            Ok(((s ^ s << 1) & 0xAAAA_AAAA) | (s >> 1 & 0x5555_5555))
        })
        .collect()
}

fn alpha_endpoints(file: &[u8], section: Section) -> Result<Vec<u32>, DecodeError> {
    let mut bits = Bits::new(file, section.offset, section.size)?;
    let code = Huffman::read(&mut bits)?;
    let (mut a, mut b) = (0, 0);
    (0..section.count)
        .map(|_| {
            a = (a + code.decode(&mut bits)?) & 255;
            b = (b + code.decode(&mut bits)?) & 255;
            Ok(a | b << 8)
        })
        .collect()
}

fn alpha_selectors(file: &[u8], section: Section) -> Result<Vec<[u32; 3]>, DecodeError> {
    let mut bits = Bits::new(file, section.offset, section.size)?;
    let code = Huffman::read(&mut bits)?;
    let from_linear = |linear: u32| {
        DXT5_FROM_LINEAR[(linear & 7) as usize] | DXT5_FROM_LINEAR[(linear >> 3 & 7) as usize] << 3
    };
    let mut linear = [0u32; 2];
    (0..section.count)
        .map(|_| {
            let mut words = [0u32; 2];
            for (word, state) in words.iter_mut().zip(&mut linear) {
                for shift in (0..24).step_by(6) {
                    *state ^= code.decode(&mut bits)? << shift;
                    *word |= from_linear(*state >> shift & 0x3F) << shift;
                }
            }
            let [s0, s1] = words;
            Ok([s0 & 0xFFFF, (s0 >> 16 | s1 << 8) & 0xFFFF, s1 >> 8 & 0xFFFF])
        })
        .collect()
}

/// The endpoint indices of a block, and how the next row's block refers to them.
#[derive(Clone, Copy, Default)]
struct Column {
    reference: u32,
    color: usize,
    alpha: usize,
}

/// Adds a palette delta to `index`, wrapping once at the end of the palette.
fn advance(index: &mut usize, delta: u32, palette: usize) {
    *index += delta as usize;
    if *index >= palette {
        *index -= palette;
    }
}

/// Unpacks level 0 of the first face of a crunch file into BC3 blocks.
fn unpack(file: &[u8], width: u32, height: u32) -> Result<Vec<u8>, DecodeError> {
    if file.len() < LEVEL_OFFSETS + 4 {
        return Err(truncated(LEVEL_OFFSETS + 4, file.len()));
    }
    if number(file, 0, 2) != SIGNATURE {
        return Err(malformed("bad signature"));
    }
    let header_size = number(file, 2, 2) as usize;
    let levels = usize::from(file[16]);
    if levels == 0 || header_size < LEVEL_OFFSETS + levels * 4 {
        return Err(malformed("bad header size"));
    }
    if file.len() < header_size {
        return Err(truncated(header_size, file.len()));
    }
    if (number(file, 12, 2), number(file, 14, 2)) != (width, height) {
        return Err(malformed("the file has other dimensions than the texture"));
    }
    let format = u32::from(file[18]);
    if format != FORMAT_DXT5 {
        return Err(DecodeError::Unsupported {
            what: "crunch format",
            value: format.into(),
        });
    }
    let [
        endpoints_color,
        selectors_color,
        endpoints_alpha,
        selectors_alpha,
    ] = [33, 41, 49, 57].map(|offset| section(file, offset));
    if [
        endpoints_color,
        selectors_color,
        endpoints_alpha,
        selectors_alpha,
    ]
    .iter()
    .any(|palette| palette.count == 0)
    {
        return Err(malformed("an empty palette"));
    }

    let mut tables = Bits::new(
        file,
        number(file, 67, 3) as usize,
        number(file, 65, 2) as usize,
    )?;
    let reference_code = Huffman::read(&mut tables)?;
    let color_endpoint_delta = Huffman::read(&mut tables)?;
    let color_selector_delta = Huffman::read(&mut tables)?;
    let alpha_endpoint_delta = Huffman::read(&mut tables)?;
    let alpha_selector_delta = Huffman::read(&mut tables)?;

    let palette_color = color_endpoints(file, endpoints_color)?;
    let selectors = color_selectors(file, selectors_color)?;
    let palette_alpha = alpha_endpoints(file, endpoints_alpha)?;
    let alpha_selector = alpha_selectors(file, selectors_alpha)?;

    let level = number(file, LEVEL_OFFSETS, 4) as usize;
    let level_end = if levels > 1 {
        number(file, LEVEL_OFFSETS + 4, 4) as usize
    } else {
        file.len()
    };
    let size = level_end
        .checked_sub(level)
        .ok_or_else(|| malformed("mip level offsets out of order"))?;
    let mut bits = Bits::new(file, level, size)?;

    let (blocks_x, blocks_y) = ((width as usize).div_ceil(4), (height as usize).div_ceil(4));
    let mut blocks = vec![0; blocks_x * blocks_y * 16];
    let mut columns = vec![Column::default(); blocks_x.next_multiple_of(2)];
    let (mut color, mut alpha, mut group) = (0, 0, 0);
    for y in 0..blocks_y.next_multiple_of(2) {
        for x in 0..columns.len() {
            if y % 2 == 0 && x % 2 == 0 {
                group = reference_code.decode(&mut bits)?;
            }
            let column = &mut columns[x];
            let reference = if y % 2 == 1 {
                column.reference
            } else {
                let current = group & 3;
                column.reference = group >> 2 & 3;
                group >>= 4;
                current
            };
            match reference {
                0 => {
                    let delta = color_endpoint_delta.decode(&mut bits)?;
                    advance(&mut color, delta, palette_color.len());
                    column.color = color;
                    let delta = alpha_endpoint_delta.decode(&mut bits)?;
                    advance(&mut alpha, delta, palette_alpha.len());
                    column.alpha = alpha;
                }
                1 => {
                    column.color = color;
                    column.alpha = alpha;
                }
                _ => {
                    color = column.color;
                    alpha = column.alpha;
                }
            }
            let color_selector = color_selector_delta.decode(&mut bits)? as usize;
            let selector = alpha_selector_delta.decode(&mut bits)? as usize;
            if x < blocks_x && y < blocks_y {
                let out_of_range = || malformed("an index outside its palette");
                let [s0, s1, s2] = *alpha_selector.get(selector).ok_or_else(out_of_range)?;
                let words = [
                    *palette_alpha.get(alpha).ok_or_else(out_of_range)? | s0 << 16,
                    s1 | s2 << 16,
                    *palette_color.get(color).ok_or_else(out_of_range)?,
                    *selectors.get(color_selector).ok_or_else(out_of_range)?,
                ];
                let block = &mut blocks[(y * blocks_x + x) * 16..][..16];
                for (bytes, word) in block.as_chunks_mut::<4>().0.iter_mut().zip(words) {
                    bytes.copy_from_slice(&word.to_le_bytes());
                }
            }
        }
    }
    Ok(blocks)
}

/// A channel of `max` levels at the fraction `numerator / (max * 3)`, as eight bits, rounded.
fn scaled(numerator: u32, max: u32) -> u8 {
    ((510 * numerator + 3 * max) / (6 * max)) as u8
}

/// Decodes a BC3 block into its sixteen texels in row order.
fn decode_bc3(block: &[u8]) -> [[u8; 4]; 16] {
    let word = |offset: usize| u32::from(u16::from_le_bytes([block[offset], block[offset + 1]]));
    let (color0, color1) = (word(8), word(10));
    let selectors = u32::from_le_bytes(block[12..16].try_into().expect("four bytes"));
    let alpha_bits = (0..6).fold(0u64, |value, i| value | u64::from(block[2 + i]) << (8 * i));
    let (a0, a1) = (u32::from(block[0]), u32::from(block[1]));
    let alpha = |code: u32| -> u8 {
        let (first, second, parts) = match (code, a0 > a1) {
            (0, _) => (1, 0, 1),
            (1, _) => (0, 1, 1),
            (6, false) => return 0,
            (7, false) => return 255,
            (_, true) => (8 - code, code - 1, 7),
            (_, false) => (6 - code, code - 1, 5),
        };
        ((2 * (first * a0 + second * a1) + parts) / (2 * parts)) as u8
    };
    let channel = |shift: u32, max: u32, code: u32| -> u8 {
        let c0 = (color0 >> shift) & max;
        let c1 = (color1 >> shift) & max;
        let (first, second) = match code {
            0 => (3, 0),
            1 => (0, 3),
            2 => (2, 1),
            _ => (1, 2),
        };
        scaled(first * c0 + second * c1, max)
    };
    std::array::from_fn(|index| {
        let index = index as u32;
        let code = selectors >> (2 * index) & 3;
        [
            channel(11, 31, code),
            channel(5, 63, code),
            channel(0, 31, code),
            alpha((alpha_bits >> (3 * index) & 7) as u32),
        ]
    })
}

/// Decodes level 0 of a crunched DXT5 image into RGBA in stored row order.
pub(super) fn decode(width: usize, height: usize, data: &[u8]) -> Result<Vec<u8>, DecodeError> {
    let (too_wide, too_high) = (width > 0xFFFF, height > 0xFFFF);
    if too_wide || too_high {
        return Err(malformed("the image is larger than the format allows"));
    }
    let blocks = unpack(data, width as u32, height as u32)?;
    decode_blocks(
        (4, 4, 16),
        width,
        height,
        &blocks,
        "BC3 image",
        |block, out| {
            out(&decode_bc3(block));
            Ok(())
        },
    )
}
