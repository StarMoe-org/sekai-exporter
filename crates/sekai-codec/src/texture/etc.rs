//! ETC1, ETC2 and EAC decoding.
//!
//! Written from the Khronos Data Format Specification. Blocks are big-endian. Pixels are numbered
//! down each column, so pixel `x * 4 + y` of a block holds the two index bits at that position
//! of the two index halves.

use super::decode_blocks;
use crate::DecodeError;

const TABLES: [[i32; 2]; 8] = [
    [2, 8],
    [5, 17],
    [9, 29],
    [13, 42],
    [18, 60],
    [24, 80],
    [33, 106],
    [47, 183],
];

const DISTANCES: [i32; 8] = [3, 6, 11, 16, 23, 32, 41, 64];

const ALPHA_TABLES: [[i32; 8]; 16] = [
    [-3, -6, -9, -15, 2, 5, 8, 14],
    [-3, -7, -10, -13, 2, 6, 9, 12],
    [-2, -5, -8, -13, 1, 4, 7, 12],
    [-2, -4, -6, -13, 1, 3, 5, 12],
    [-3, -6, -8, -12, 2, 5, 7, 11],
    [-3, -7, -9, -11, 2, 6, 8, 10],
    [-4, -7, -8, -11, 3, 6, 7, 10],
    [-3, -5, -8, -11, 2, 4, 7, 10],
    [-2, -6, -8, -10, 1, 5, 7, 9],
    [-2, -5, -8, -10, 1, 4, 7, 9],
    [-2, -4, -8, -10, 1, 3, 7, 9],
    [-2, -5, -7, -10, 1, 4, 6, 9],
    [-3, -4, -7, -10, 2, 3, 6, 9],
    [-1, -2, -3, -10, 0, 1, 2, 9],
    [-4, -6, -8, -9, 3, 5, 7, 8],
    [-3, -5, -7, -9, 2, 4, 6, 8],
];

/// Which modes the differential blocks of a format can select.
#[derive(Clone, Copy, PartialEq)]
enum Profile {
    Etc1,
    Etc2,
}

/// Decodes ETC1 into RGBA in stored row order.
pub(super) fn decode_etc1(
    width: usize,
    height: usize,
    data: &[u8],
) -> Result<Vec<u8>, DecodeError> {
    let mut texels = [[0; 4]; 16];
    decode_blocks(
        (4, 4, 8),
        width,
        height,
        data,
        "ETC1 image",
        |block, out| {
            color_block(word(block), Profile::Etc1, &mut texels)?;
            out(&texels);
            Ok(())
        },
    )
}

/// Decodes ETC2 RGBA8, an EAC alpha block before each colour block, into RGBA in stored row
/// order.
pub(super) fn decode_etc2_rgba8(
    width: usize,
    height: usize,
    data: &[u8],
) -> Result<Vec<u8>, DecodeError> {
    let mut texels = [[0; 4]; 16];
    decode_blocks(
        (4, 4, 16),
        width,
        height,
        data,
        "ETC2 image",
        |block, out| {
            color_block(word(&block[8..]), Profile::Etc2, &mut texels)?;
            alpha_block(word(&block[..8]), &mut texels);
            out(&texels);
            Ok(())
        },
    )
}

fn word(block: &[u8]) -> u64 {
    u64::from_be_bytes(block.try_into().expect("a whole block"))
}

fn expand4(value: i32) -> i32 {
    value * 17
}

fn expand5(value: i32) -> i32 {
    value << 3 | value >> 2
}

fn expand6(value: i32) -> i32 {
    value << 2 | value >> 4
}

fn expand7(value: i32) -> i32 {
    value << 1 | value >> 6
}

fn sign3(value: i32) -> i32 {
    if value >= 4 { value - 8 } else { value }
}

fn shifted([r, g, b]: [i32; 3], delta: i32) -> [i32; 3] {
    [r + delta, g + delta, b + delta]
}

fn store(texel: &mut [u8; 4], [r, g, b]: [i32; 3]) {
    *texel = [
        r.clamp(0, 255) as u8,
        g.clamp(0, 255) as u8,
        b.clamp(0, 255) as u8,
        255,
    ];
}

/// Fills `texels` from the 32 index bits, with `color` giving a texel's colour from its position
/// and two-bit index.
fn paint(
    indices: u32,
    texels: &mut [[u8; 4]; 16],
    color: impl Fn(usize, usize, usize) -> [i32; 3],
) {
    for y in 0..4 {
        for x in 0..4 {
            let bit = x * 4 + y;
            let index = ((indices >> (bit + 16)) & 1) << 1 | ((indices >> bit) & 1);
            store(&mut texels[y * 4 + x], color(x, y, index as usize));
        }
    }
}

fn color_block(bits: u64, profile: Profile, texels: &mut [[u8; 4]; 16]) -> Result<(), DecodeError> {
    let field = |start: u32, len: u32| ((bits >> start) & ((1 << len) - 1)) as i32;
    let indices = bits as u32;
    let bases = if field(33, 1) == 0 {
        [
            [field(60, 4), field(52, 4), field(44, 4)].map(expand4),
            [field(56, 4), field(48, 4), field(40, 4)].map(expand4),
        ]
    } else {
        let base = [field(59, 5), field(51, 5), field(43, 5)];
        let delta = [field(56, 3), field(48, 3), field(40, 3)].map(sign3);
        let moved = [base[0] + delta[0], base[1] + delta[1], base[2] + delta[2]];
        let fits = moved.map(|value| (0..=31).contains(&value));
        if fits != [true; 3] {
            if profile == Profile::Etc1 {
                return Err(DecodeError::Malformed {
                    what: "ETC1 block",
                    message: "differential colour out of range".to_owned(),
                });
            }
            match fits {
                [false, _, _] => t_mode(bits, texels),
                [true, false, _] => h_mode(bits, texels),
                _ => planar_mode(bits, texels),
            }
            return Ok(());
        }
        [base.map(expand5), moved.map(expand5)]
    };
    let tables = [TABLES[field(37, 3) as usize], TABLES[field(34, 3) as usize]];
    let flipped = field(32, 1) == 1;
    paint(indices, texels, |x, y, index| {
        let second = if flipped { y >= 2 } else { x >= 2 } as usize;
        let [small, large] = tables[second];
        shifted(bases[second], [small, large, -small, -large][index])
    });
    Ok(())
}

fn t_mode(bits: u64, texels: &mut [[u8; 4]; 16]) {
    let field = |start: u32, len: u32| ((bits >> start) & ((1 << len) - 1)) as i32;
    let first = [field(59, 2) << 2 | field(56, 2), field(52, 4), field(48, 4)].map(expand4);
    let second = [field(44, 4), field(40, 4), field(36, 4)].map(expand4);
    let distance = DISTANCES[(field(34, 2) << 1 | field(32, 1)) as usize];
    let colors = [
        first,
        shifted(second, distance),
        second,
        shifted(second, -distance),
    ];
    paint(bits as u32, texels, |_, _, index| colors[index]);
}

fn h_mode(bits: u64, texels: &mut [[u8; 4]; 16]) {
    let field = |start: u32, len: u32| ((bits >> start) & ((1 << len) - 1)) as i32;
    let first = [
        field(59, 4),
        field(56, 3) << 1 | field(52, 1),
        field(51, 1) << 3 | field(47, 3),
    ];
    let second = [field(43, 4), field(39, 4), field(35, 4)];
    let order = (first >= second) as i32;
    let distance = DISTANCES[(field(34, 1) << 2 | field(32, 1) << 1 | order) as usize];
    let (first, second) = (first.map(expand4), second.map(expand4));
    let colors = [
        shifted(first, distance),
        shifted(first, -distance),
        shifted(second, distance),
        shifted(second, -distance),
    ];
    paint(bits as u32, texels, |_, _, index| colors[index]);
}

fn planar_mode(bits: u64, texels: &mut [[u8; 4]; 16]) {
    let field = |start: u32, len: u32| ((bits >> start) & ((1 << len) - 1)) as i32;
    let expand = |[r, g, b]: [i32; 3]| [expand6(r), expand7(g), expand6(b)];
    let origin = expand([
        field(57, 6),
        field(56, 1) << 6 | field(49, 6),
        field(48, 1) << 5 | field(43, 2) << 3 | field(39, 3),
    ]);
    let horizontal = expand([field(34, 5) << 1 | field(32, 1), field(25, 7), field(19, 6)]);
    let vertical = expand([field(13, 6), field(6, 7), field(0, 6)]);
    for y in 0..4 {
        for x in 0..4 {
            let channel = |c: usize| {
                let (dx, dy) = (horizontal[c] - origin[c], vertical[c] - origin[c]);
                (x as i32 * dx + y as i32 * dy + 4 * origin[c] + 2) >> 2
            };
            store(&mut texels[y * 4 + x], [channel(0), channel(1), channel(2)]);
        }
    }
}

/// Replaces the alpha of `texels` with the values of an EAC alpha block.
fn alpha_block(bits: u64, texels: &mut [[u8; 4]; 16]) {
    let base = (bits >> 56) as i32;
    let multiplier = (bits >> 52 & 0xF) as i32;
    let table = ALPHA_TABLES[(bits >> 48 & 0xF) as usize];
    for y in 0..4 {
        for x in 0..4 {
            let shift = 45 - 3 * (x * 4 + y);
            let index = (bits >> shift & 7) as usize;
            texels[y * 4 + x][3] = (base + table[index] * multiplier).clamp(0, 255) as u8;
        }
    }
}
