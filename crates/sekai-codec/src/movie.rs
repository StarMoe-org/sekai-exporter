//! A video stream with its picture format and audio.

use crate::Pcm;

/// An MPEG video elementary stream, its picture format and its decoded audio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Movie {
    /// The elementary stream.
    pub stream: Vec<u8>,
    /// The picture width in pixels.
    pub width: u32,
    /// The picture height in pixels.
    pub height: u32,
    /// The frame rate as a fraction.
    pub framerate_n: u32,
    /// The frame rate as a fraction.
    pub framerate_d: u32,
    /// The decoded audio track.
    pub audio: Option<Pcm>,
}

/// The start code of a picture in an MPEG video stream. Start codes do not occur anywhere else
/// in the stream.
const PICTURE_START: [u8; 4] = [0, 0, 1, 0];

impl Movie {
    /// The bitrate of the stream in bit/s: its size over its duration, its pictures shown at the
    /// frame rate. `None` when the stream has no picture or the frame rate has a zero
    /// denominator.
    #[must_use]
    pub fn bitrate(&self) -> Option<u64> {
        let pictures = self
            .stream
            .windows(4)
            .filter(|w| *w == PICTURE_START)
            .count();
        let bits = u64::try_from(self.stream.len()).ok()?.checked_mul(8)?;
        let per_second = bits.checked_mul(u64::from(self.framerate_n))?;
        let duration = u64::try_from(pictures)
            .ok()?
            .checked_mul(u64::from(self.framerate_d))?;
        per_second.checked_div(duration)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie(stream: Vec<u8>, framerate_n: u32, framerate_d: u32) -> Movie {
        Movie {
            stream,
            width: 64,
            height: 48,
            framerate_n,
            framerate_d,
            audio: None,
        }
    }

    /// `pictures` pictures of 1000 bytes each, after a sequence header.
    fn stream(pictures: usize) -> Vec<u8> {
        let mut stream = vec![0, 0, 1, 0xB3, 7, 7, 7, 7];
        for _ in 0..pictures {
            stream.extend(PICTURE_START);
            stream.extend([0x55; 996]);
        }
        stream
    }

    #[test]
    fn the_bitrate_is_the_size_over_the_pictures_at_the_frame_rate() {
        // 30 pictures and 8 header bytes, one second at 30 frames per second.
        assert_eq!(movie(stream(30), 30, 1).bitrate(), Some(240_064));
        // At 29.97 frames per second the same pictures last 1.001 seconds.
        assert_eq!(movie(stream(30), 30_000, 1001).bitrate(), Some(239_824));
    }

    #[test]
    fn a_stream_without_pictures_or_a_frame_rate_has_no_bitrate() {
        assert_eq!(movie(vec![0, 0, 1, 0xB3], 30, 1).bitrate(), None);
        assert_eq!(movie(stream(30), 30, 0).bitrate(), None);
    }
}
