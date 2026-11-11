//! The canonical encoders write output that decodes to the input, losslessly for images and close
//! to it for audio, the same bytes on every run, and fixed bytes for fixed input.

use std::io::Cursor;

use sekai_codec::{Pcm, Sound, Surface};
use sekai_encode::{EncodeError, mp3, png};
use sha2::{Digest, Sha256};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

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

/// The sample rate, the channel count and the interleaved samples an MP3 decodes to, trimmed by
/// the encoder delay and the padding its Info frame gives.
fn decode_mp3(bytes: &[u8]) -> (u32, u16, Vec<f32>) {
    let source = MediaSourceStream::new(Box::new(Cursor::new(bytes.to_vec())), Default::default());
    let gapless = FormatOptions {
        enable_gapless: true,
        ..Default::default()
    };
    let mut format = symphonia::default::get_probe()
        .format(&Hint::new(), source, &gapless, &MetadataOptions::default())
        .unwrap()
        .format;
    let parameters = format.default_track().unwrap().codec_params.clone();
    let mut decoder = symphonia::default::get_codecs()
        .make(&parameters, &DecoderOptions::default())
        .unwrap();
    let mut samples = Vec::new();
    while let Ok(packet) = format.next_packet() {
        let decoded = decoder.decode(&packet).unwrap();
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        buffer.copy_interleaved_ref(decoded);
        samples.extend_from_slice(buffer.samples());
    }
    let channels = parameters.channels.unwrap().count() as u16;
    (parameters.sample_rate.unwrap(), channels, samples)
}

/// The ratio in decibels of the signal of `pcm` to the error of `decoded`.
fn signal_to_noise(pcm: &Pcm, decoded: &[f32]) -> f64 {
    let (mut signal, mut noise) = (0.0, 0.0);
    for (&s, &d) in pcm.samples().iter().zip(decoded) {
        let s = f64::from(s) / 32768.0;
        signal += s * s;
        noise += (s - f64::from(d)) * (s - f64::from(d));
    }
    10.0 * (signal / noise).log10()
}

/// A sine per channel, a different pitch on each.
fn tone(channels: u16, frames: usize, sample_rate: u32) -> Pcm {
    let mut samples = Vec::with_capacity(frames * usize::from(channels));
    for i in 0..frames {
        for c in 0..channels {
            let hertz = 440.0 + 110.0 * f64::from(c);
            let phase = std::f64::consts::TAU * hertz * i as f64 / f64::from(sample_rate);
            samples.push((12_000.0 * phase.sin()) as i16);
        }
    }
    Pcm::new(sample_rate, channels, samples).unwrap()
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
fn mp3_decodes_to_the_samples_closely_and_to_their_length_exactly() {
    // Lengths of a whole number of frames, and of a few samples over one.
    for (channels, frames, rate) in [
        (1, 44_100, 44_100),
        (2, 1152 * 30, 48_000),
        (2, 20_005, 24_000),
        (1, 576 * 40, 22_050),
    ] {
        let pcm = tone(channels, frames, rate);
        let (decoded_rate, decoded_channels, decoded) =
            decode_mp3(&mp3::encode(&Sound::Pcm(pcm.clone())).unwrap());
        assert_eq!((decoded_rate, decoded_channels), (rate, channels));
        assert_eq!(decoded.len(), pcm.samples().len(), "{channels} x {frames}");
        let snr = signal_to_noise(&pcm, &decoded);
        assert!(snr > 40.0, "{channels} channels at {rate} Hz: {snr:.1} dB");
    }
}

#[test]
fn the_bitrate_follows_the_source() {
    // One and a half times the source, rounded up to a bitrate of the MPEG version.
    assert_eq!(mp3::bitrate(44_100, Some(113_000)), Ok(192));
    assert_eq!(mp3::bitrate(44_100, Some(64_000)), Ok(96));
    assert_eq!(mp3::bitrate(48_000, Some(256_000)), Ok(320));
    assert_eq!(mp3::bitrate(24_000, Some(48_000)), Ok(80));
    assert_eq!(mp3::bitrate(22_050, Some(160_000)), Ok(160));
    // Decoded samples take the highest bitrate.
    assert_eq!(mp3::bitrate(44_100, None), Ok(320));
    assert_eq!(mp3::bitrate(16_000, None), Ok(160));
    assert!(mp3::bitrate(96_000, None).is_err());
}

#[test]
fn output_is_fixed() {
    let surface = image(37, 23);
    let sound = Sound::Pcm(audio(2, 9_001));
    let outputs = [
        ("png", png::encode(&surface).unwrap()),
        ("mp3", mp3::encode(&sound).unwrap()),
    ];
    let again = [png::encode(&surface).unwrap(), mp3::encode(&sound).unwrap()];
    let expected = [
        (
            "png",
            "ff56074ae834f2842303badc18c25840ef21d4ee92e08b86235bb6dbe8ae6b4c",
        ),
        (
            "mp3",
            "4fe480a1c7153b5980c362b2aebc0349194060d60df5abb609dee92bda421bc2",
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
    let sound = |pcm| mp3::encode(&Sound::Pcm(pcm));
    assert!(is_failed(sound(audio(3, 100))));
    assert!(is_failed(sound(Pcm::new(44_100, 2, Vec::new()).unwrap())));
    assert!(is_failed(sound(
        Pcm::new(96_000, 1, vec![0; 1152]).unwrap()
    )));
}
