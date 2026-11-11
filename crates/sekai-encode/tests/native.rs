//! The encoders that need a C library. Tests that need a library or ffmpeg pass without checking
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

mod movie {
    use super::{Command, EncodeError, Stdio, tone};
    use sekai_codec::Movie;
    use sekai_encode::mp4;

    /// An MPEG-2 video stream of `frames` test frames, or `None` without ffmpeg.
    fn stream(frames: u32) -> Option<Vec<u8>> {
        let output = Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc=size=64x48:rate=30",
            ])
            .args(["-frames:v", &frames.to_string(), "-f", "mpeg2video", "-"])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        assert!(output.status.success(), "ffmpeg failed");
        Some(output.stdout)
    }

    #[test]
    fn a_movie_is_the_same_on_every_run() {
        let Some(stream) = stream(12) else {
            return;
        };
        let movie = Movie {
            stream,
            width: 64,
            height: 48,
            framerate_n: 30,
            framerate_d: 1,
            audio: Some(tone(2, 44100 / 2)),
        };
        let first = match mp4::encode(&movie) {
            Err(EncodeError::Unavailable { .. }) => return,
            other => other.expect("encoded"),
        };
        assert_eq!(&first[4..8], b"ftyp");
        assert_eq!(first, mp4::encode(&movie).expect("encoded"));
    }
}

