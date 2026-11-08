//! AAC audio tracks through libavcodec, loaded at run time in native builds.
//!
//! The output is the raw frames and decoder configuration that an MP4 audio track stores, so
//! the MP4 writer can place them without parsing a container.

use sekai_codec::Pcm;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "aac";

/// The encoder version.
pub const VERSION: u32 = 1;

/// An encoded AAC-LC track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AacTrack {
    /// The `AudioSpecificConfig` of the track.
    pub config: Vec<u8>,
    /// The raw frames, in order. Every frame holds `samples_per_frame` samples per channel.
    pub frames: Vec<Vec<u8>>,
    /// The samples per channel in one frame.
    pub samples_per_frame: u32,
    /// The samples per channel at the start of the first frame that are encoder delay.
    pub delay: u32,
}

/// Encodes `pcm` as a constant-bitrate AAC-LC track.
pub fn encode(pcm: &Pcm, bitrate_kbps: u32) -> Result<AacTrack, EncodeError> {
    let _ = (pcm, bitrate_kbps);
    Err(EncodeError::not_written(NAME))
}
