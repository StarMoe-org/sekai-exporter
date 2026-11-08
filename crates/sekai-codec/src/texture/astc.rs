//! ASTC decoding: LDR profile, two-dimensional blocks.
//!
//! Written from the ASTC chapter of the Khronos Data Format Specification. Where the
//! specification decodes an illegal block to an error colour, this decoder reports the block as
//! malformed; HDR content is reported as unsupported.

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

/// Decodes an image of `block_width` by `block_height` blocks into RGBA in stored row order.
pub(super) fn decode(
    block_width: usize,
    block_height: usize,
    width: usize,
    height: usize,
    data: &[u8],
) -> Result<Vec<u8>, DecodeError> {
    let mut texels = [[0; 4]; MAX_TEXELS];
    decode_blocks(
        (block_width, block_height, BLOCK_BYTES),
        width,
        height,
        data,
        "ASTC image",
        |block, out| {
            let block = u128::from_le_bytes(block.try_into().expect("a whole block"));
            decode_block(block, block_width, block_height, &mut texels)?;
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

fn void_extent(block: u128, texels: &mut [[u8; 4]]) -> Result<(), DecodeError> {
    if bits(block, 9, 1) == 1 {
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
    texels.fill(color.map(to_byte));
    Ok(())
}

/// The eight-bit form of a 16-bit colour value.
fn to_byte(value: u32) -> u8 {
    (value >> 8) as u8
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

/// The pair of endpoints of colour endpoint format `format`, from its encoded values.
fn endpoints(format: u32, v: &[i32]) -> Result<[[i32; 4]; 2], DecodeError> {
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
        _ => {
            return Err(DecodeError::Unsupported {
                what: "ASTC HDR endpoint format",
                value: format.into(),
            });
        }
    };
    Ok(pair.map(|point| point.map(|c| c.clamp(0, 255))))
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
    texels: &mut [[u8; 4]],
) -> Result<(), DecodeError> {
    if bits(block, 0, 9) == 0x1FC {
        return void_extent(block, texels);
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
    let mut colors = [[[0; 4]; 2]; 4];
    let mut next = 0;
    for (color, &format) in colors.iter_mut().zip(formats) {
        let count = ((format >> 2) as usize + 1) * 2;
        *color = endpoints(format, &values[next..next + count])?;
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
            let [low, high] = colors[partition];
            for (c, byte) in texel.iter_mut().enumerate() {
                let w = if second_plane == Some(c) {
                    second
                } else {
                    first
                };
                let (c0, c1) = (low[c] as u32 * 257, high[c] as u32 * 257);
                *byte = to_byte((c0 * (64 - w) + c1 * w + 32) >> 6);
            }
        }
    }
    Ok(())
}
