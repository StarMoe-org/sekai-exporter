//! The canonical encoders write lossless output, the same bytes on every run, and fixed bytes
//! for fixed input.

use std::io::Cursor;

use sekai_codec::{Pcm, Surface};
use sekai_encode::{EncodeError, flac, png, webp_lossless};
use sha2::{Digest, Sha256};

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A small pseudo-random generator, so the inputs are the same everywhere.
struct Lcg(u32);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.0 >> 8
    }
}

/// Gradients in red and green, noise in blue and alpha.
fn image(width: u32, height: u32) -> Surface {
    let mut rng = Lcg(width * 31 + height);
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for y in 0..height {
        for x in 0..width {
            pixels.push((x * 255 / width.max(2).saturating_sub(1)) as u8);
            pixels.push((y * 255 / height.max(2).saturating_sub(1)) as u8);
            pixels.push(rng.next() as u8);
            pixels.push(if x % 7 == 0 {
                0
            } else {
                128 + (rng.next() % 128) as u8
            });
        }
    }
    Surface::new(width, height, pixels).unwrap()
}

/// A triangle wave per channel with a little noise, using the full 16-bit range at the peaks.
fn audio(channels: u16, frames: usize) -> Pcm {
    let mut rng = Lcg(u32::from(channels) * 1000 + frames as u32);
    let mut samples = Vec::with_capacity(frames * usize::from(channels));
    for i in 0..frames {
        for c in 0..usize::from(channels) {
            let period = 200 + 37 * c;
            let phase = (i % period) as i32 * 4 * 32767 / period as i32;
            let wave = if phase < 2 * 32767 {
                phase - 32767
            } else {
                3 * 32767 - phase
            };
            let noise = (rng.next() % 64) as i32 - 32;
            samples.push((wave + noise).clamp(-32768, 32767) as i16);
        }
    }
    Pcm::new(44_100, channels, samples).unwrap()
}

fn decode_png(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut reader = ::png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!(info.color_type, ::png::ColorType::Rgba);
    assert_eq!(info.bit_depth, ::png::BitDepth::Eight);
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}

fn decode_webp(bytes: &[u8]) -> (u32, u32, Vec<u8>) {
    let mut decoder = image_webp::WebPDecoder::new(Cursor::new(bytes)).unwrap();
    assert!(!decoder.is_lossy());
    let (width, height) = decoder.dimensions();
    let mut pixels = vec![0; decoder.output_buffer_size().unwrap()];
    decoder.read_image(&mut pixels).unwrap();
    if !decoder.has_alpha() {
        pixels = pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect();
    }
    (width, height, pixels)
}

fn decode_flac(bytes: &[u8]) -> (u32, u16, Vec<i16>) {
    let mut reader = claxon::FlacReader::new(Cursor::new(bytes)).unwrap();
    let info = reader.streaminfo();
    assert_eq!(info.bits_per_sample, 16);
    let samples = reader
        .samples()
        .map(|s| i16::try_from(s.unwrap()).unwrap())
        .collect();
    (info.sample_rate, info.channels as u16, samples)
}

const SIZES: [(u32, u32); 4] = [(1, 1), (37, 23), (256, 3), (3, 300)];

#[test]
fn png_round_trips() {
    for (w, h) in SIZES {
        let surface = image(w, h);
        let (width, height, pixels) = decode_png(&png::encode(&surface).unwrap());
        assert_eq!((width, height), (w, h));
        assert_eq!(pixels, surface.pixels(), "{w} x {h}");
    }
}

#[test]
fn webp_lossless_round_trips() {
    for (w, h) in SIZES {
        let surface = image(w, h);
        let (width, height, pixels) = decode_webp(&webp_lossless::encode(&surface).unwrap());
        assert_eq!((width, height), (w, h));
        assert_eq!(pixels, surface.pixels(), "{w} x {h}");
    }
    let opaque = Surface::new(2, 2, [9, 8, 7, 255].repeat(4)).unwrap();
    let (_, _, pixels) = decode_webp(&webp_lossless::encode(&opaque).unwrap());
    assert_eq!(pixels, opaque.pixels());
}

#[test]
fn flac_round_trips() {
    for (channels, frames) in [(1, 1), (1, 10_000), (2, 4096), (2, 9_001), (8, 5_000)] {
        let pcm = audio(channels, frames);
        let (rate, ch, samples) = decode_flac(&flac::encode(&pcm).unwrap());
        assert_eq!((rate, ch), (44_100, channels));
        assert_eq!(
            samples,
            pcm.samples(),
            "{channels} channels, {frames} frames"
        );
    }
}

#[test]
fn output_is_fixed() {
    let surface = image(37, 23);
    let pcm = audio(2, 9_001);
    let outputs = [
        ("png", png::encode(&surface).unwrap()),
        ("webp-lossless", webp_lossless::encode(&surface).unwrap()),
        ("flac", flac::encode(&pcm).unwrap()),
    ];
    let again = [
        png::encode(&surface).unwrap(),
        webp_lossless::encode(&surface).unwrap(),
        flac::encode(&pcm).unwrap(),
    ];
    let expected = [
        (
            "png",
            "bb8c87e8aded900be8f8fc332540d1ac9c9ae35fd45346153bc27f684e152c30",
        ),
        (
            "webp-lossless",
            "06d7c9d2f34ec372924c7a36ebd2624ceb000621df11354a9408ee324afd4cbf",
        ),
        (
            "flac",
            "8eff9a89756bfce12f9729cc1327a23fd78acc80a2b00fba7d835be4569f398c",
        ),
    ];
    for (((name, bytes), repeat), (_, hash)) in outputs.iter().zip(&again).zip(expected) {
        assert_eq!(bytes, repeat, "{name} changed between runs");
        assert_eq!(sha256(bytes), hash, "{name}");
    }
}

fn is_failed(result: Result<Vec<u8>, EncodeError>) -> bool {
    matches!(result, Err(EncodeError::Failed { .. }))
}

#[test]
fn out_of_range_input_fails() {
    let empty = Surface::new(0, 0, Vec::new()).unwrap();
    assert!(is_failed(png::encode(&empty)));
    assert!(is_failed(webp_lossless::encode(&empty)));
    let wide = image(16_385, 1);
    assert!(is_failed(webp_lossless::encode(&wide)));
    assert!(is_failed(flac::encode(&audio(9, 100))));
}

#[test]
fn empty_audio() {
    let silent = Pcm::new(44_100, 2, Vec::new()).unwrap();
    let (rate, channels, samples) = decode_flac(&flac::encode(&silent).unwrap());
    assert_eq!((rate, channels), (44_100, 2));
    assert!(samples.is_empty());
}
