//! CRI middleware formats: HCA audio, the ACB cue sheet with its AWB wave bank, and USM video.
//!
//! [`decode_hca`] turns one HCA stream into PCM. Reading an ACB yields its cues and the HCA
//! streams they play, taken from the ACB itself or from its AWB; reading a USM yields its video
//! and audio streams as stored. Those readers are submodules of this one.

mod acb;
mod adx;
mod awb;
mod hca;
mod hca_tables;
mod usm;
mod utf;

pub use acb::{Acb, Cue, Waveform, read_acb};
pub use awb::{Awb, AwbStream, read_awb};
pub use usm::{AudioStream, Usm, VideoFrame, VideoStream, read_usm};

use crate::{DecodeError, Pcm};

/// The key of an encrypted HCA stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HcaKey {
    /// The game's key.
    pub key: u64,
    /// The subkey of the AWB the stream comes from, or 0 for a stream outside an AWB.
    pub subkey: u16,
}

/// What the header of an HCA stream says about its audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HcaInfo {
    /// The channel count.
    pub channels: u16,
    /// The sample rate in hertz.
    pub sample_rate: u32,
    /// The samples per channel that the frames hold, including the encoder delay and padding.
    pub frames: u64,
}

/// Reads the header of one HCA stream without decoding its frames.
pub fn hca_info(data: &[u8]) -> Result<HcaInfo, DecodeError> {
    hca::info(data)
}

/// Decodes one ADX stream to PCM.
///
/// Streams that are not version 4, encoding type 3 with four-bit samples, including encrypted
/// ones, are reported as [`DecodeError::Unsupported`].
pub fn decode_adx(data: &[u8]) -> Result<Pcm, DecodeError> {
    adx::decode(data, false)
}

/// Decodes the whole frames of one ADX stream. A stream carried in a USM file can end before the
/// sample count of its header.
pub fn decode_adx_available(data: &[u8]) -> Result<Pcm, DecodeError> {
    adx::decode(data, true)
}

/// Decodes one HCA stream to PCM, without its encoder delay and padding.
///
/// A stream of cipher type 56 needs `key`; decoding one without it fails with
/// [`DecodeError::Unsupported`]. The checksum of a frame covers its stored bytes, so a wrong key
/// is not detected.
pub fn decode_hca(data: &[u8], key: Option<HcaKey>) -> Result<Pcm, DecodeError> {
    hca::decode(data, key)
}
