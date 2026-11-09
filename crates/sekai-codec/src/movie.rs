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
