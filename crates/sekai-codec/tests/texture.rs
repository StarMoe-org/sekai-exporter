//! Texture decoding gives the pixels of the reference decoders and rejects bad input by name.
#![cfg(feature = "texture")]

use std::fs;
use std::path::Path;

use sekai_codec::texture::{decode, decode_hdr, is_hdr};
use sekai_codec::{DecodeError, HdrSurface};
use sha2::{Digest, Sha256};

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The fixtures: format, width, height, file name and the SHA-256 of the reference pixels.
fn fixtures() -> Vec<(i32, u32, u32, Vec<u8>, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/textures");
    fs::read_to_string(root.join("expected.txt"))
        .unwrap()
        .lines()
        .map(|line| {
            let fields: Vec<_> = line.split(' ').collect();
            (
                fields[0].parse().unwrap(),
                fields[1].parse().unwrap(),
                fields[2].parse().unwrap(),
                fs::read(root.join(fields[3])).unwrap(),
                fields[4].to_owned(),
            )
        })
        .collect()
}

#[test]
fn fixtures_decode_to_the_reference_pixels() {
    let fixtures = fixtures();
    assert!(!fixtures.is_empty());
    for (format, width, height, data, expected) in fixtures {
        let surface = decode(format, width, height, &data).unwrap();
        assert_eq!(sha256(surface.pixels()), expected, "format {format}");
    }
}

/// The crunch and ASTC HDR fixtures, in the form of [`fixtures`].
fn compressed_fixtures() -> Vec<(i32, u32, u32, Vec<u8>, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/textures");
    fs::read_to_string(root.join("compressed.txt"))
        .unwrap()
        .lines()
        .map(|line| {
            let fields: Vec<_> = line.split(' ').collect();
            (
                fields[0].parse().unwrap(),
                fields[1].parse().unwrap(),
                fields[2].parse().unwrap(),
                fs::read(root.join(fields[3])).unwrap(),
                fields[4].to_owned(),
            )
        })
        .collect()
}

#[test]
fn crunch_and_hdr_fixtures_decode_to_the_reference_pixels() {
    let fixtures = compressed_fixtures();
    assert_eq!(fixtures.len(), 4);
    for (format, width, height, data, expected) in fixtures {
        let found = if is_hdr(format) {
            let surface = decode_hdr(format, width, height, &data).unwrap();
            let bytes: Vec<u8> = surface
                .pixels()
                .iter()
                .flat_map(|value| value.to_le_bytes())
                .collect();
            sha256(&bytes)
        } else {
            sha256(decode(format, width, height, &data).unwrap().pixels())
        };
        assert_eq!(found, expected, "format {format}");
    }
}

#[test]
fn a_crunch_file_is_checked() {
    let (format, width, height, data, _) = compressed_fixtures().swap_remove(0);
    assert_eq!(format, 29);
    assert!(matches!(
        decode(format, width, height, &data[..32]),
        Err(DecodeError::Truncated { .. })
    ));
    assert!(matches!(
        decode(format, width + 1, height, &data),
        Err(DecodeError::Malformed { .. })
    ));
    let mut bad = data;
    bad[0] ^= 1;
    assert!(matches!(
        decode(format, width, height, &bad),
        Err(DecodeError::Malformed { .. })
    ));
}

#[test]
fn the_hdr_view_clamps_and_rounds() {
    let surface = HdrSurface::new(1, 1, vec![0x3C00, 0x4000, 0x3800, 0xBC00]).unwrap();
    assert_eq!(surface.to_surface().pixels(), [255, 255, 128, 0]);
}

#[test]
fn only_the_astc_hdr_formats_are_hdr() {
    assert!((66..=71).all(is_hdr));
    assert!(!is_hdr(65) && !is_hdr(72) && !is_hdr(50));
    assert!(matches!(
        decode_hdr(50, 4, 4, &[0; 16]),
        Err(DecodeError::Unsupported { value: 50, .. })
    ));
}

#[test]
fn a_short_payload_is_truncated() {
    for (format, width, height, data, _) in fixtures() {
        let error = decode(format, width, height, &data[..data.len() - 1]).unwrap_err();
        assert!(
            matches!(error, DecodeError::Truncated { .. }),
            "format {format}: {error}"
        );
    }
}

#[test]
fn bytes_after_the_first_image_are_ignored() {
    for (format, width, height, mut data, expected) in fixtures() {
        data.extend_from_slice(&[0xA5; 64]);
        let surface = decode(format, width, height, &data).unwrap();
        assert_eq!(sha256(surface.pixels()), expected, "format {format}");
    }
}

#[test]
fn rows_run_from_top_to_bottom() {
    let stored = [1, 2, 3, 4, 5, 6, 7, 8];
    let surface = decode(4, 1, 2, &stored).unwrap();
    assert_eq!(surface.pixels(), [5, 6, 7, 8, 1, 2, 3, 4]);
}

#[test]
fn uncompressed_formats_expand_to_rgba() {
    assert_eq!(decode(1, 1, 1, &[9]).unwrap().pixels(), [0, 0, 0, 9]);
    assert_eq!(
        decode(3, 1, 1, &[1, 2, 3]).unwrap().pixels(),
        [1, 2, 3, 255]
    );
}

#[test]
fn rgb565_replicates_the_top_bits() {
    assert_eq!(
        decode(7, 2, 1, &[0x00, 0xF8, 0x41, 0x08]).unwrap().pixels(),
        [255, 0, 0, 255, 8, 8, 8, 255]
    );
}

#[test]
fn an_empty_image_is_malformed() {
    assert!(matches!(
        decode(4, 0, 4, &[]),
        Err(DecodeError::Malformed { .. })
    ));
}

#[test]
fn an_unknown_format_is_unsupported() {
    assert!(matches!(
        decode(-1, 4, 4, &[0; 64]),
        Err(DecodeError::Unsupported { value: -1, .. })
    ));
}

/// A constant ASTC block of the given colour and block mode bits.
fn constant_block(mode: u16, color: [u16; 4]) -> Vec<u8> {
    let mut block = vec![0xFF; 8];
    block[..2].copy_from_slice(&(mode | 0xF000).to_le_bytes());
    for value in color {
        block.extend_from_slice(&value.to_le_bytes());
    }
    block
}

#[test]
fn an_astc_constant_block_takes_the_top_byte() {
    let block = constant_block(0x0DFC, [0x12FF, 0x3400, 0xFFFF, 0x0080]);
    let surface = decode(48, 4, 4, &block).unwrap();
    assert_eq!(surface.pixels()[..4], [0x12, 0x34, 0xFF, 0x00]);
}

#[test]
fn an_astc_hdr_constant_block_is_unsupported() {
    let block = constant_block(0x0FFC, [0; 4]);
    assert!(matches!(
        decode(48, 4, 4, &block),
        Err(DecodeError::Unsupported { .. })
    ));
}

#[test]
fn a_reserved_astc_block_mode_is_malformed() {
    assert!(matches!(
        decode(48, 4, 4, &[0; 16]),
        Err(DecodeError::Malformed { .. })
    ));
}

#[test]
fn an_etc1_block_with_a_colour_out_of_range_is_malformed() {
    // Red is 31 with an offset of +1, and the differential bit is set.
    let block = [0xF9, 0, 0, 0x02, 0, 0, 0, 0];
    assert!(matches!(
        decode(34, 4, 4, &block),
        Err(DecodeError::Malformed { .. })
    ));
}

#[test]
fn r8_keeps_its_value_in_red() {
    let surface = decode(63, 1, 1, &[200]).unwrap();
    assert_eq!(surface.pixels(), [200, 0, 0, 255]);
}

#[test]
fn eac_r_narrows_eleven_bits_to_eight() {
    // Base 100, multiplier 1, table 0, and every texel selects the table value -3.
    let block = [100, 0x10, 0, 0, 0, 0, 0, 0];
    let surface = decode(41, 4, 4, &block).unwrap();
    assert_eq!(surface.pixels()[..4], [97, 0, 0, 255]);
}

#[test]
fn etc2_rgb_reads_an_individual_block_as_etc1_does() {
    let block = [0x80, 0x80, 0x80, 0, 0, 0, 0, 0];
    let etc2 = decode(45, 4, 4, &block).unwrap();
    let etc1 = decode(34, 4, 4, &block).unwrap();
    assert_eq!(etc2.pixels(), etc1.pixels());
    assert_eq!(etc2.pixels()[3], 255);
}
