//! ASTC decoding: LDR and HDR profiles, two-dimensional blocks.
//!
//! Written from the ASTC chapter of the Khronos Data Format Specification. Where the
//! specification decodes an illegal block to an error colour, this decoder reports the block as
//! malformed; HDR content in an image of an LDR format is reported as unsupported.

use super::decode_blocks;
use crate::DecodeError;

const BLOCK_BYTES: usize = 16;
const MAX_TEXELS: usize = 12 * 12;

/// Room for the largest weight grid and the neighbours its last row and column read as zero.
const PLANE_LEN: usize = 64 + 12 + 1;

#[derive(Clone, Copy)]
enum Kind {
    Bits,
    Trit,
    Quint,
}

/// The integer sequence ranges in the order of the specification: how each value is packed and
/// its number of low bits.
const RANGES: [(Kind, u32); 21] = [
    (Kind::Bits, 1),
    (Kind::Trit, 0),
    (Kind::Bits, 2),
    (Kind::Quint, 0),
    (Kind::Trit, 1),
    (Kind::Bits, 3),
    (Kind::Quint, 1),
    (Kind::Trit, 2),
    (Kind::Bits, 4),
    (Kind::Quint, 2),
    (Kind::Trit, 3),
    (Kind::Bits, 5),
    (Kind::Quint, 3),
    (Kind::Trit, 4),
    (Kind::Bits, 6),
    (Kind::Quint, 4),
    (Kind::Trit, 5),
    (Kind::Bits, 7),
    (Kind::Quint, 5),
    (Kind::Trit, 6),
    (Kind::Bits, 8),
];

/// The first range with enough levels for colour endpoints: 0 to 5.
const MIN_COLOR_RANGE: usize = 4;

/// Weight values in sequence order, for the weight ranges (the first twelve of [`RANGES`]).
#[rustfmt::skip]
const WEIGHTS: [&[u8]; 12] = [
    &[0, 64],
    &[0, 32, 64],
    &[0, 21, 43, 64],
    &[0, 16, 32, 48, 64],
    &[0, 64, 12, 52, 25, 39],
    &[0, 9, 18, 27, 37, 46, 55, 64],
    &[0, 64, 7, 57, 14, 50, 21, 43, 28, 36],
    &[0, 64, 17, 47, 5, 59, 23, 41, 11, 53, 28, 36],
    &[0, 4, 8, 12, 17, 21, 25, 29, 35, 39, 43, 47, 52, 56, 60, 64],
    &[0, 64, 16, 48, 3, 61, 19, 45, 6, 58, 23, 41, 9, 55, 26, 38, 13, 51, 29, 35],
    &[0, 64, 8, 56, 16, 48, 24, 40, 2, 62, 11, 53, 19, 45, 27, 37, 5, 59, 13, 51, 22, 42, 30, 34],
    &[0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30,
      34, 36, 38, 40, 42, 44, 46, 48, 50, 52, 54, 56, 58, 60, 62, 64],
];

/// Decodes an LDR image of `block_width` by `block_height` blocks into RGBA in stored row order.
pub(super) fn decode(
    block_width: usize,
    block_height: usize,
    width: usize,
    height: usize,
    data: &[u8],
) -> Result<Vec<u8>, DecodeError> {
    let mut texels = [[0; 4]; MAX_TEXELS];
    let mut bytes = [[0; 4]; MAX_TEXELS];
    decode_blocks(
        (block_width, block_height, BLOCK_BYTES),
        width,
        height,
        data,
        "ASTC image",
        |block, out| {
            let block = u128::from_le_bytes(block.try_into().expect("a whole block"));
            decode_block(block, block_width, block_height, false, &mut texels)?;
            for (byte, texel) in bytes.iter_mut().zip(&texels) {
                *byte = texel.map(|value| (value >> 8) as u8);
            }
            out(&bytes);
            Ok(())
        },
    )
}

/// Decodes an HDR image into binary16 RGBA in stored row order.
pub(super) fn decode_hdr(
    block_width: usize,
    block_height: usize,
    width: usize,
    height: usize,
    data: &[u8],
) -> Result<Vec<u16>, DecodeError> {
    let mut texels = [[0; 4]; MAX_TEXELS];
    decode_blocks(
        (block_width, block_height, BLOCK_BYTES),
        width,
        height,
        data,
        "ASTC image",
        |block, out| {
            let block = u128::from_le_bytes(block.try_into().expect("a whole block"));
            decode_block(block, block_width, block_height, true, &mut texels)?;
            out(&texels);
            Ok(())
        },
    )
}

fn illegal(message: &str) -> DecodeError {
    DecodeError::Malformed {
        what: "ASTC block",
        message: message.to_owned(),
    }
}

fn bits(block: u128, start: u32, len: u32) -> u32 {
    (block >> start) as u32 & ((1 << len) - 1)
}

/// The number of bits `count` values of `range` occupy.
fn sequence_bits(range: usize, count: u32) -> u32 {
    let (kind, low) = RANGES[range];
    count * low
        + match kind {
            Kind::Bits => 0,
            Kind::Trit => (8 * count).div_ceil(5),
            Kind::Quint => (7 * count).div_ceil(3),
        }
}

fn trits(t: u32) -> [u32; 5] {
    let field = |start: u32, len: u32| -> u32 { (t >> start) & ((1 << len) - 1) };
    let (c, t4, t3) = if field(2, 3) == 7 {
        (field(5, 3) << 2 | field(0, 2), 2, 2)
    } else if field(5, 2) == 3 {
        (field(0, 5), 2, field(7, 1))
    } else {
        (field(0, 5), field(7, 1), field(5, 2))
    };
    let c_field = |start: u32, len: u32| -> u32 { (c >> start) & ((1 << len) - 1) };
    let carry = |high: u32, low: u32| high << 1 | (low & !high & 1);
    let (t2, t1, t0) = if c_field(0, 2) == 3 {
        (2, c_field(4, 1), carry(c_field(3, 1), c_field(2, 1)))
    } else if c_field(2, 2) == 3 {
        (2, 2, c_field(0, 2))
    } else {
        (
            c_field(4, 1),
            c_field(2, 2),
            carry(c_field(1, 1), c_field(0, 1)),
        )
    };
    [t0, t1, t2, t3, t4]
}

fn quints(q: u32) -> [u32; 5] {
    let field = |start: u32, len: u32| -> u32 { (q >> start) & ((1 << len) - 1) };
    let (q2, q1, q0);
    if field(1, 2) == 3 && field(5, 2) == 0 {
        let high = field(0, 1);
        q2 = high << 2 | (field(4, 1) & !high & 1) << 1 | (field(3, 1) & !high & 1);
        q1 = 4;
        q0 = 4;
    } else {
        let c;
        if field(1, 2) == 3 {
            q2 = 4;
            c = field(3, 2) << 3 | (!field(5, 2) & 3) << 1 | field(0, 1);
        } else {
            q2 = field(5, 2);
            c = field(0, 5);
        }
        if c & 7 == 5 {
            q1 = 4;
            q0 = c >> 3;
        } else {
            q1 = c >> 3;
            q0 = c & 7;
        }
    }
    [q0, q1, q2, 0, 0]
}

/// Reads an integer sequence of `out.len()` values of `range` from bit `start`.
///
/// Trits and quints are packed in groups, their bits interleaved with the low bits. A group cut
/// short by the end of the sequence has its missing high bits read as zero.
fn read_sequence(block: u128, start: u32, range: usize, out: &mut [u8]) {
    let (kind, low) = RANGES[range];
    let packing: &[(u32, u32)] = match kind {
        Kind::Bits => &[(0, 0)],
        Kind::Trit => &[(0, 2), (2, 2), (4, 1), (5, 2), (7, 1)],
        Kind::Quint => &[(0, 3), (3, 2), (5, 2)],
    };
    let mut pos = start;
    let mut take = |len| {
        let value = bits(block, pos, len);
        pos += len;
        value
    };
    for group in out.chunks_mut(packing.len()) {
        let mut low_bits = [0; 5];
        let mut high = 0;
        for (value, &(offset, len)) in low_bits.iter_mut().zip(&packing[..group.len()]) {
            *value = take(low);
            high |= take(len) << offset;
        }
        let digits = match kind {
            Kind::Bits => [0; 5],
            Kind::Trit => trits(high),
            Kind::Quint => quints(high),
        };
        for ((value, digit), low_value) in group.iter_mut().zip(digits).zip(low_bits) {
            *value = (digit << low | low_value) as u8;
        }
    }
}

/// Scales an encoded colour value of `range` to eight bits.
fn unquantize_color(range: usize, value: u32) -> i32 {
    let (kind, low) = RANGES[range];
    let (is_trit, c) = match kind {
        Kind::Bits => {
            let mut out = value << (8 - low);
            let mut filled = low;
            while filled < 8 {
                out |= out >> filled;
                filled *= 2;
            }
            return (out & 0xFF) as i32;
        }
        Kind::Trit => (true, [204, 93, 44, 22, 11, 5][low as usize - 1]),
        Kind::Quint => (false, [113, 54, 26, 13, 6][low as usize - 1]),
    };
    let bit = |i: u32| (value >> i) & 1;
    let (b, x, d, e, f) = (bit(1), bit(2), bit(3), bit(4), bit(5));
    let pattern = match (is_trit, low) {
        (_, 1) => 0,
        (true, 2) => b * 0b1_0001_0110,
        (false, 2) => b * 0b1_0000_1100,
        (true, 3) => x << 8 | b << 7 | x << 3 | b << 2 | x << 1 | b,
        (false, 3) => x << 8 | b << 7 | x << 2 | b << 1 | x,
        (true, 4) => d << 8 | x << 7 | b << 6 | d << 2 | x << 1 | b,
        (false, 4) => d << 8 | x << 7 | b << 6 | d << 1 | x,
        (true, 5) => e << 8 | d << 7 | x << 6 | b << 5 | e << 1 | d,
        (false, 5) => e << 8 | d << 7 | x << 6 | b << 5 | e,
        _ => f << 8 | e << 7 | d << 6 | x << 5 | b << 4 | f,
    };
    let a = if bit(0) == 1 { 0x1FF } else { 0 };
    let t = ((value >> low) * c + pattern) ^ a;
    ((a & 0x80) | (t >> 2)) as i32
}

struct Mode {
    grid_width: usize,
    grid_height: usize,
    dual: bool,
    weight_range: usize,
    weight_bits: u32,
}

fn block_mode(mode: u32, block_width: usize, block_height: usize) -> Result<Mode, DecodeError> {
    let field = |start: u32, len: u32| -> u32 { (mode >> start) & ((1 << len) - 1) };
    let a = field(5, 2);
    let mut high = field(9, 1) == 1;
    let mut dual = field(10, 1) == 1;
    let mut range = field(4, 1);
    let (grid_width, grid_height);
    if field(0, 2) != 0 {
        range |= field(0, 2) << 1;
        let b = field(7, 2);
        (grid_width, grid_height) = match field(2, 2) {
            0 => (b + 4, a + 2),
            1 => (b + 8, a + 2),
            2 => (a + 2, b + 8),
            _ if field(8, 1) == 1 => ((b & 1) + 2, a + 2),
            _ => (a + 2, (b & 1) + 6),
        };
    } else {
        if field(2, 2) == 0 {
            return Err(illegal("reserved block mode"));
        }
        range |= field(2, 2) << 1;
        (grid_width, grid_height) = match field(7, 2) {
            0 => (12, a + 2),
            1 => (a + 2, 12),
            2 => {
                high = false;
                dual = false;
                (a + 6, field(9, 2) + 6)
            }
            _ => match a {
                0 => (6, 10),
                1 => (10, 6),
                _ => return Err(illegal("reserved block mode")),
            },
        };
    }
    let count = grid_width * grid_height * (1 + dual as u32);
    let weight_range = (range - 2 + 6 * high as u32) as usize;
    if count > 64 {
        return Err(illegal("too many weights"));
    }
    let weight_bits = sequence_bits(weight_range, count);
    if !(24..=96).contains(&weight_bits) {
        return Err(illegal("weight bits out of range"));
    }
    if grid_width as usize > block_width || grid_height as usize > block_height {
        return Err(illegal("weight grid is larger than the block"));
    }
    Ok(Mode {
        grid_width: grid_width as usize,
        grid_height: grid_height as usize,
        dual,
        weight_range,
        weight_bits,
    })
}

fn void_extent(block: u128, hdr: bool, texels: &mut [[u16; 4]]) -> Result<(), DecodeError> {
    let float = bits(block, 9, 1) == 1;
    if float && !hdr {
        return Err(DecodeError::Unsupported {
            what: "ASTC HDR constant block",
            value: 1,
        });
    }
    if bits(block, 10, 2) != 3 {
        return Err(illegal("reserved bits of a constant block are not set"));
    }
    let [s0, s1, t0, t1] = [12, 25, 38, 51].map(|start| bits(block, start, 13));
    if (s0, s1, t0, t1) != (0x1FFF, 0x1FFF, 0x1FFF, 0x1FFF) && (s0 >= s1 || t0 >= t1) {
        return Err(illegal("constant block with an empty extent"));
    }
    let color = [64, 80, 96, 112].map(|start| bits(block, start, 16));
    texels.fill(color.map(|value| {
        if hdr && !float {
            unorm_to_half(value)
        } else {
            value as u16
        }
    }));
    Ok(())
}

/// The binary16 form of a 16-bit unsigned normalized value.
fn unorm_to_half(value: u32) -> u16 {
    if value == 0xFFFF {
        return 0x3C00;
    }
    if value < 4 {
        return (value << 8) as u16;
    }
    let zeros = value.leading_zeros() - 16;
    ((((value << (zeros + 1)) & 0xFFFF) >> 6) | (14 - zeros) << 10) as u16
}

/// The binary16 form of a 16-bit value of the logarithmic number system of HDR endpoints.
fn lns_to_half(value: u32) -> u16 {
    let mantissa = value & 0x7FF;
    let curved = if mantissa < 512 {
        mantissa * 3
    } else if mantissa < 1536 {
        mantissa * 4 - 512
    } else {
        mantissa * 5 - 2048
    };
    (((value >> 11) << 10 | curved >> 3).min(0x7BFF)) as u16
}

/// Splits the pair `a`, `b`: moves the top bit of `a` into `b` and returns `a` as a signed
/// 6-bit offset, then `b`.
fn bit_transfer(a: i32, b: i32) -> (i32, i32) {
    let base = (b >> 1) | (a & 0x80);
    let mut offset = (a >> 1) & 0x3F;
    if offset & 0x20 != 0 {
        offset -= 0x40;
    }
    (offset, base)
}

fn blue_contract([r, g, b, a]: [i32; 4]) -> [i32; 4] {
    [(r + b) >> 1, (g + b) >> 1, b, a]
}

/// The endpoints of a partition: their 16-bit values, and which channels are logarithmic.
#[derive(Clone, Copy, Default)]
struct Endpoints {
    pair: [[u32; 4]; 2],
    lns: [bool; 4],
}

/// The endpoints of colour endpoint format `format`, from its encoded values.
fn endpoints(format: u32, v: &[i32], hdr: bool) -> Result<Endpoints, DecodeError> {
    let pair = match format {
        0 => [[v[0], v[0], v[0], 255], [v[1], v[1], v[1], 255]],
        1 => {
            let low = (v[0] >> 2) | (v[1] & 0xC0);
            let high = (low + (v[1] & 0x3F)).min(255);
            [[low, low, low, 255], [high, high, high, 255]]
        }
        4 => [[v[0], v[0], v[0], v[2]], [v[1], v[1], v[1], v[3]]],
        5 => {
            let (dl, l) = bit_transfer(v[1], v[0]);
            let (da, a) = bit_transfer(v[3], v[2]);
            [[l, l, l, a], [l + dl, l + dl, l + dl, a + da]]
        }
        6 | 10 => {
            let scaled = |c| (c * v[3]) >> 8;
            let (a0, a1) = if format == 10 {
                (v[4], v[5])
            } else {
                (255, 255)
            };
            [
                [scaled(v[0]), scaled(v[1]), scaled(v[2]), a0],
                [v[0], v[1], v[2], a1],
            ]
        }
        8 | 12 => {
            let (a0, a1) = if format == 12 {
                (v[6], v[7])
            } else {
                (255, 255)
            };
            let first = [v[0], v[2], v[4], a0];
            let second = [v[1], v[3], v[5], a1];
            if v[1] + v[3] + v[5] >= v[0] + v[2] + v[4] {
                [first, second]
            } else {
                [blue_contract(second), blue_contract(first)]
            }
        }
        9 | 13 => {
            let (dr, r) = bit_transfer(v[1], v[0]);
            let (dg, g) = bit_transfer(v[3], v[2]);
            let (db, b) = bit_transfer(v[5], v[4]);
            let (da, a) = if format == 13 {
                bit_transfer(v[7], v[6])
            } else {
                (0, 255)
            };
            let base = [r, g, b, a];
            let moved = [r + dr, g + dg, b + db, a + da];
            if dr + dg + db >= 0 {
                [base, moved]
            } else {
                [blue_contract(moved), blue_contract(base)]
            }
        }
        _ if !hdr => {
            return Err(DecodeError::Unsupported {
                what: "ASTC HDR endpoint format",
                value: format.into(),
            });
        }
        _ => return Ok(hdr_endpoints(format, v)),
    };
    Ok(Endpoints {
        pair: pair.map(|point| point.map(|c| c.clamp(0, 255) as u32 * 257)),
        lns: [false; 4],
    })
}

fn bit(value: i32, index: u32) -> i32 {
    (value >> index) & 1
}

/// The endpoints of an HDR endpoint format: 2, 3, 7, 11, 14 or 15.
fn hdr_endpoints(format: u32, v: &[i32]) -> Endpoints {
    let [first, second] = match format {
        2 => hdr_luminance_large(v),
        3 => hdr_luminance_small(v),
        7 => hdr_rgb_scale(v),
        _ => hdr_rgb(v),
    };
    let ([a0, a1], alpha_lns) = match format {
        14 => ([v[6] as u32 * 257, v[7] as u32 * 257], false),
        15 => (hdr_alpha(v[6], v[7]), true),
        _ => ([0x7800, 0x7800], true),
    };
    let point = |[r, g, b]: [i32; 3], a| [(r as u32) << 4, (g as u32) << 4, (b as u32) << 4, a];
    Endpoints {
        pair: [point(first, a0), point(second, a1)],
        lns: [true, true, true, alpha_lns],
    }
}

fn hdr_luminance_large(v: &[i32]) -> [[i32; 3]; 2] {
    let (y0, y1) = if v[1] >= v[0] {
        (v[0] << 4, v[1] << 4)
    } else {
        ((v[1] << 4) + 8, (v[0] << 4) - 8)
    };
    [[y0; 3], [y1; 3]]
}

fn hdr_luminance_small(v: &[i32]) -> [[i32; 3]; 2] {
    let (y0, delta) = if v[0] & 0x80 != 0 {
        ((v[1] & 0xE0) << 4 | (v[0] & 0x7F) << 2, (v[1] & 0x1F) << 2)
    } else {
        ((v[1] & 0xF0) << 4 | (v[0] & 0x7F) << 1, (v[1] & 0xF) << 1)
    };
    [[y0; 3], [(y0 + delta).min(0xFFF); 3]]
}

fn hdr_rgb_scale(v: &[i32]) -> [[i32; 3]; 2] {
    let modeval = (v[0] & 0xC0) >> 6 | bit(v[1], 7) << 2 | bit(v[2], 7) << 3;
    let (major, mode) = if modeval & 0xC != 0xC {
        (modeval >> 2, modeval & 3)
    } else if modeval != 0xF {
        (modeval & 3, 4)
    } else {
        (0, 5)
    };
    let moved = [
        bit(v[1], 6),
        bit(v[1], 5),
        bit(v[2], 6),
        bit(v[2], 5),
        bit(v[3], 7),
        bit(v[3], 6),
        bit(v[3], 5),
    ];
    let mut values = [v[0] & 0x3F, v[1] & 0x1F, v[2] & 0x1F, v[3] & 0x1F];
    // Which value takes which moved bit at which position, in the modes of the mask.
    const PLACEMENT: [(usize, i32, usize, u32); 17] = [
        (1, 0x30, 0, 6),
        (1, 0x3A, 1, 5),
        (2, 0x30, 2, 6),
        (2, 0x3A, 3, 5),
        (3, 0x3D, 6, 5),
        (3, 0x2D, 5, 6),
        (3, 0x04, 4, 7),
        (0, 0x3B, 4, 6),
        (0, 0x04, 3, 6),
        (0, 0x10, 5, 7),
        (0, 0x0F, 2, 7),
        (0, 0x05, 1, 8),
        (0, 0x0A, 0, 8),
        (0, 0x05, 0, 9),
        (0, 0x02, 6, 9),
        (0, 0x01, 3, 10),
        (0, 0x02, 5, 10),
    ];
    for (target, modes, source, position) in PLACEMENT {
        if modes >> mode & 1 != 0 {
            values[target] |= moved[source] << position;
        }
    }
    let shift = [1, 1, 2, 3, 4, 5][mode as usize];
    let [mut red, mut green, mut blue, scale] = values.map(|value| value << shift);
    if mode != 5 {
        green = red - green;
        blue = red - blue;
    }
    match major {
        1 => std::mem::swap(&mut red, &mut green),
        2 => std::mem::swap(&mut red, &mut blue),
        _ => {}
    }
    [
        [red - scale, green - scale, blue - scale].map(|c| c.max(0)),
        [red, green, blue].map(|c| c.max(0)),
    ]
}

fn hdr_rgb(v: &[i32]) -> [[i32; 3]; 2] {
    let modeval = bit(v[1], 7) | bit(v[2], 7) << 1 | bit(v[3], 7) << 2;
    let major = bit(v[4], 7) | bit(v[5], 7) << 1;
    if major == 3 {
        return [
            [v[0] << 4, v[2] << 4, (v[4] & 0x7F) << 5],
            [v[1] << 4, v[3] << 4, (v[5] & 0x7F) << 5],
        ];
    }
    let mut a = v[0] | (v[1] & 0x40) << 2;
    let mut b0 = v[2] & 0x3F;
    let mut b1 = v[3] & 0x3F;
    let mut c = v[1] & 0x3F;
    let mut d0 = v[4] & 0x7F;
    let mut d1 = v[5] & 0x7F;
    let moved = [
        bit(v[2], 6),
        bit(v[3], 6),
        bit(v[4], 6),
        bit(v[5], 6),
        bit(v[4], 5),
        bit(v[5], 5),
    ];
    let in_mode = |modes: i32| modes >> modeval & 1 != 0;
    if in_mode(0xA4) {
        a |= moved[0] << 9;
    }
    if in_mode(0x8) {
        a |= moved[2] << 9;
    }
    if in_mode(0x50) {
        a |= moved[4] << 9 | moved[5] << 10;
    }
    if in_mode(0xA0) {
        a |= moved[1] << 10;
    }
    if in_mode(0xC0) {
        a |= moved[2] << 11;
    }
    if in_mode(0x4) {
        c |= moved[1] << 6;
    }
    if in_mode(0xE8) {
        c |= moved[3] << 6;
    }
    if in_mode(0x20) {
        c |= moved[2] << 7;
    }
    if in_mode(0x5B) {
        b0 |= moved[0] << 6;
        b1 |= moved[1] << 6;
    }
    if in_mode(0x12) {
        b0 |= moved[2] << 7;
        b1 |= moved[3] << 7;
    }
    if in_mode(0xAF) {
        d0 |= moved[4] << 5;
        d1 |= moved[5] << 5;
    }
    if in_mode(0x5) {
        d0 |= moved[2] << 6;
        d1 |= moved[3] << 6;
    }
    let unused = 32 - [7, 6, 7, 6, 5, 6, 5, 6][modeval as usize];
    let (d0, d1) = ((d0 << unused) >> unused, (d1 << unused) >> unused);
    let shift = (modeval >> 1) ^ 3;
    let [a, b0, b1, c, d0, d1] = [a, b0, b1, c, d0, d1].map(|value| value << shift);
    let mut second = [a, a - b0, a - b1].map(|channel| channel.clamp(0, 4095));
    let mut first = [a - c, a - b0 - c - d0, a - b1 - c - d1].map(|channel| channel.clamp(0, 4095));
    if major != 0 {
        let other = major as usize;
        first.swap(0, other);
        second.swap(0, other);
    }
    [first, second]
}

fn hdr_alpha(v6: i32, v7: i32) -> [u32; 2] {
    let modeval = bit(v6, 7) | (v7 >> 6 & 2);
    let (mut base, mut delta) = (v6 & 0x7F, v7 & 0x7F);
    let (first, second) = if modeval == 3 {
        (base << 5, delta << 5)
    } else {
        base |= (delta << (modeval + 1)) & 0x780;
        delta &= 0x3F >> modeval;
        delta ^= 32 >> modeval;
        delta -= 32 >> modeval;
        let base = base << (4 - modeval);
        (base, (base + (delta << (4 - modeval))).clamp(0, 0xFFF))
    };
    [(first as u32) << 4, (second as u32) << 4]
}

/// The partition of a texel, for a seed that already includes the partition count.
fn select_partition(seed: u32, count: u32, x: u32, y: u32, small: bool) -> usize {
    let (x, y) = if small { (x << 1, y << 1) } else { (x, y) };
    let mut r = seed;
    r ^= r >> 15;
    r = r.wrapping_sub(r << 17);
    r = r.wrapping_add(r << 7);
    r = r.wrapping_add(r << 4);
    r ^= r >> 5;
    r = r.wrapping_add(r << 16);
    r ^= r >> 7;
    r ^= r >> 3;
    r ^= r << 6;
    r ^= r >> 17;

    let by_count = if count == 3 { 6 } else { 5 };
    let by_seed = if seed & 2 != 0 { 4 } else { 5 };
    let (shift1, shift2) = if seed & 1 != 0 {
        (by_seed, by_count)
    } else {
        (by_count, by_seed)
    };
    let scale = |i: u32, shift: u32| {
        let n = (r >> (4 * i)) & 0xF;
        (n * n) >> shift
    };
    let mix = |i: u32, offset: u32| {
        (scale(i, shift1) * x + scale(i + 1, shift2) * y + (r >> offset)) & 0x3F
    };
    let a = mix(0, 14);
    let b = mix(2, 10);
    let c = if count >= 3 { mix(4, 6) } else { 0 };
    let d = if count == 4 { mix(6, 2) } else { 0 };
    if a >= b && a >= c && a >= d {
        0
    } else if b >= c && b >= d {
        1
    } else if c >= d {
        2
    } else {
        3
    }
}

fn decode_block(
    block: u128,
    block_width: usize,
    block_height: usize,
    hdr: bool,
    texels: &mut [[u16; 4]],
) -> Result<(), DecodeError> {
    if bits(block, 0, 9) == 0x1FC {
        return void_extent(block, hdr, texels);
    }
    let mode = block_mode(bits(block, 0, 11), block_width, block_height)?;
    let partitions = bits(block, 11, 2) + 1;
    if mode.dual && partitions == 4 {
        return Err(illegal("four partitions with two weight planes"));
    }

    // The fields below the weights are placed downwards from the start of the weights.
    let mut below_weights = 128 - mode.weight_bits;
    let mut formats = [0; 4];
    let mut seed = 0;
    let colors_start;
    if partitions == 1 {
        formats[0] = bits(block, 13, 4);
        colors_start = 17;
    } else {
        seed = bits(block, 13, 10) + (partitions - 1) * 1024;
        colors_start = 29;
        let low = bits(block, 23, 6);
        if low & 3 == 0 {
            formats = [low >> 2; 4];
        } else {
            let high_len = 3 * partitions - 4;
            below_weights -= high_len;
            let field = low | bits(block, below_weights, high_len) << 6;
            let class = (field & 3) - 1;
            for i in 0..partitions {
                let raised = (field >> (2 + i)) & 1;
                let low_format = (field >> (2 + partitions + 2 * i)) & 3;
                formats[i as usize] = (class + raised) << 2 | low_format;
            }
        }
    }
    let second_plane = mode.dual.then(|| {
        below_weights -= 2;
        bits(block, below_weights, 2) as usize
    });

    let formats = &formats[..partitions as usize];
    let value_count: u32 = formats.iter().map(|format| ((format >> 2) + 1) * 2).sum();
    if value_count > 18 {
        return Err(illegal("more than 18 endpoint values"));
    }
    let color_bits = below_weights
        .checked_sub(colors_start)
        .ok_or_else(|| illegal("no room for endpoint values"))?;
    let color_range = (MIN_COLOR_RANGE..RANGES.len())
        .rev()
        .find(|&range| sequence_bits(range, value_count) <= color_bits)
        .ok_or_else(|| illegal("no endpoint range fits"))?;

    let mut encoded = [0; 18];
    read_sequence(
        block,
        colors_start,
        color_range,
        &mut encoded[..value_count as usize],
    );
    let values = encoded.map(|value| unquantize_color(color_range, value.into()));
    let mut colors = [Endpoints::default(); 4];
    let mut next = 0;
    for (color, &format) in colors.iter_mut().zip(formats) {
        let count = ((format >> 2) as usize + 1) * 2;
        *color = endpoints(format, &values[next..next + count], hdr)?;
        next += count;
    }

    let mut grid = [0; 64];
    let grid_len = mode.grid_width * mode.grid_height;
    let step = 1 + mode.dual as usize;
    read_sequence(
        block.reverse_bits(),
        0,
        mode.weight_range,
        &mut grid[..grid_len * step],
    );
    let table = WEIGHTS[mode.weight_range];
    let mut planes = [[0; PLANE_LEN]; 2];
    for i in 0..grid_len {
        planes[0][i] = u32::from(table[grid[i * step] as usize]);
        if mode.dual {
            planes[1][i] = u32::from(table[grid[i * 2 + 1] as usize]);
        }
    }

    let small = block_width * block_height < 31;
    let scale_x = (1024 + block_width / 2) / (block_width - 1);
    let scale_y = (1024 + block_height / 2) / (block_height - 1);
    let rows = texels.chunks_exact_mut(block_width).take(block_height);
    for (y, row) in rows.enumerate() {
        let gy = (scale_y * y * (mode.grid_height - 1) + 32) >> 6;
        let (jy, fy) = (gy >> 4, (gy & 0xF) as u32);
        for (x, texel) in row.iter_mut().enumerate() {
            let gx = (scale_x * x * (mode.grid_width - 1) + 32) >> 6;
            let (jx, fx) = (gx >> 4, (gx & 0xF) as u32);
            let w11 = (fx * fy + 8) >> 4;
            let (w10, w01, w00) = (fy - w11, fx - w11, 16 + w11 - fx - fy);
            let v = jx + jy * mode.grid_width;
            let weight = |plane: &[u32; PLANE_LEN]| {
                (plane[v] * w00
                    + plane[v + 1] * w01
                    + plane[v + mode.grid_width] * w10
                    + plane[v + mode.grid_width + 1] * w11
                    + 8)
                    >> 4
            };
            let first = weight(&planes[0]);
            let second = weight(&planes[1]);
            let partition = if partitions == 1 {
                0
            } else {
                select_partition(seed, partitions, x as u32, y as u32, small)
            };
            let Endpoints {
                pair: [low, high],
                lns,
            } = colors[partition];
            for (c, value) in texel.iter_mut().enumerate() {
                let w = if second_plane == Some(c) {
                    second
                } else {
                    first
                };
                let mixed = (low[c] * (64 - w) + high[c] * w + 32) >> 6;
                *value = match (hdr, lns[c]) {
                    (false, _) => mixed as u16,
                    (true, true) => lns_to_half(mixed),
                    (true, false) => unorm_to_half(mixed),
                };
            }
        }
    }
    Ok(())
}
