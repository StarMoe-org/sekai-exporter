use std::fmt;

use crate::DecodeError;

/// Decoded audio: interleaved signed 16-bit samples.
///
/// Sixteen bits is the resolution reference decoders produce for the game's codecs, so decoded
/// audio can be compared with them sample by sample.
#[derive(Clone, PartialEq, Eq)]
pub struct Pcm {
    sample_rate: u32,
    channels: u16,
    samples: Vec<i16>,
}

impl Pcm {
    /// Audio from its samples, interleaved by channel.
    ///
    /// Fails when the sample rate or the channel count is zero, or when the samples do not fill
    /// whole frames.
    pub fn new(sample_rate: u32, channels: u16, samples: Vec<i16>) -> Result<Self, DecodeError> {
        if sample_rate == 0 || channels == 0 || !samples.len().is_multiple_of(channels as usize) {
            return Err(DecodeError::Malformed {
                what: "pcm",
                message: format!(
                    "{} samples, {channels} channels at {sample_rate} Hz",
                    samples.len()
                ),
            });
        }
        Ok(Self {
            sample_rate,
            channels,
            samples,
        })
    }

    /// The sample rate in hertz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// The number of channels.
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// The number of frames: samples per channel.
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels as usize
    }

    /// The samples, interleaved by channel.
    pub fn samples(&self) -> &[i16] {
        &self.samples
    }
}

impl fmt::Debug for Pcm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Pcm({} frames, {} channels, {} Hz)",
            self.frames(),
            self.channels,
            self.sample_rate
        )
    }
}
