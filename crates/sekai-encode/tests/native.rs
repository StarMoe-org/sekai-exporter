//! The encoders that need a C library. Tests that need LAME or ffmpeg pass without checking
//! anything on a machine that does not have them.

use std::process::{Command, Stdio};

use sekai_codec::{Pcm, Surface};
use sekai_encode::EncodeError;

fn tone(channels: u16, length: usize) -> Pcm {
    let mut samples = Vec::with_capacity(length * usize::from(channels));
    for i in 0..length {
        for channel in 0..channels {
            let phase = i as f64 * (440.0 + f64::from(channel) * 110.0) / 44100.0;
            samples.push((phase * std::f64::consts::TAU).sin().mul_add(9000.0, 0.0) as i16);
        }
    }
    Pcm::new(44100, channels, samples).expect("valid audio")
}

fn image(width: u32, height: u32) -> Surface {
    let mut pixels = Vec::new();
    for y in 0..height {
        for x in 0..width {
            pixels.extend([(x * 3) as u8, (y * 5) as u8, ((x ^ y) * 7) as u8, 255]);
        }
    }
    Surface::new(width, height, pixels).expect("valid image")
}

#[cfg(feature = "webp")]
mod lossy_webp {
    use super::image;
    use sekai_encode::webp;

    #[test]
    fn output_is_the_same_on_every_run_and_decodes_to_the_size() {
        let surface = image(64, 40);
        let first = webp::encode(&surface, 80.0).expect("encoded");
        assert_eq!(first, webp::encode(&surface, 80.0).expect("encoded"));
        assert_eq!(&first[..4], b"RIFF");
        assert_eq!(&first[8..12], b"WEBP");
        let decoded = ::webp::Decoder::new(&first).decode().expect("decoded");
        assert_eq!((decoded.width(), decoded.height()), (64, 40));
    }

    #[test]
    fn lower_quality_gives_fewer_bytes() {
        let surface = image(128, 128);
        let high = webp::encode(&surface, 95.0).expect("encoded");
        let low = webp::encode(&surface, 10.0).expect("encoded");
        assert!(low.len() < high.len());
    }
}

#[cfg(feature = "mp3")]
mod mp3 {
    use super::{Command, EncodeError, Pcm, Stdio, tone};
    use sekai_encode::mp3;

    #[test]
    fn a_missing_library_is_unavailable() {
        let result = mp3::encode_with(&["no-such-library-for-this-test"], &tone(1, 1152), 128);
        let Err(EncodeError::Unavailable { message, .. }) = result else {
            panic!("expected unavailable, got {result:?}");
        };
        assert!(message.contains("no-such-library-for-this-test"));
    }

    /// Decodes with ffmpeg to the sample count and channel count, if ffmpeg exists.
    fn decode_with_ffmpeg(bytes: &[u8], channels: u16) -> Option<usize> {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "sekai-encode-test-{}-{channels}.mp3",
            std::process::id()
        ));
        std::fs::write(&path, bytes).expect("written");
        let output = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&path)
            .args(["-f", "s16le", "-ac"])
            .arg(channels.to_string())
            .arg("-")
            .stdin(Stdio::null())
            .output();
        std::fs::remove_file(&path).expect("removed");
        let output = output.ok()?;
        assert!(output.status.success(), "ffmpeg failed");
        Some(output.stdout.len() / 2 / usize::from(channels))
    }

    fn check(pcm: &Pcm) {
        let first = match mp3::encode(pcm, 128) {
            Err(EncodeError::Unavailable { .. }) => return,
            other => other.expect("encoded"),
        };
        assert_eq!(first, mp3::encode(pcm, 128).expect("encoded"));
        let channels = pcm.channels();
        let expected = pcm.samples().len() / usize::from(channels);
        if let Some(decoded) = decode_with_ffmpeg(&first, channels) {
            let frames = (expected + 2 * 1152).div_ceil(1152) * 1152;
            assert!(
                (expected..=frames + 1152).contains(&decoded),
                "{expected} samples became {decoded}"
            );
        }
    }

    #[test]
    fn mono_and_stereo_are_the_same_on_every_run_and_decode_to_the_length() {
        check(&tone(1, 44100));
        check(&tone(2, 30000));
    }

    #[test]
    fn three_channels_fail() {
        let result = mp3::encode(&tone(3, 1152), 128);
        assert!(matches!(
            result,
            Err(EncodeError::Failed { .. } | EncodeError::Unavailable { .. })
        ));
    }
}
