//! CRI middleware formats: HCA audio, the ACB cue sheet with its AWB wave bank, and USM video.
//!
//! [`decode_hca`] turns one HCA stream into PCM. Reading an ACB yields its cues and the HCA
//! streams they play, taken from the ACB itself or from its AWB; reading a USM yields its video
//! and audio streams as stored. Those readers are submodules of this one.

mod acb;
mod awb;
mod hca;
mod hca_tables;
mod utf;

pub use acb::{Acb, Cue, Waveform, read_acb};
pub use awb::{Awb, AwbStream, read_awb};

use crate::{DecodeError, Pcm};

/// The key of an encrypted HCA stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HcaKey {
    /// The game's key.
    pub key: u64,
    /// The subkey of the AWB the stream comes from, or 0 for a stream outside an AWB.
    pub subkey: u16,
}

/// Decodes one HCA stream to PCM, without its encoder delay and padding.
///
/// A stream of cipher type 56 needs `key`; decoding one without it fails with
/// [`DecodeError::Unsupported`]. The checksum of a frame covers its stored bytes, so a wrong key
/// is not detected.
pub fn decode_hca(data: &[u8], key: Option<HcaKey>) -> Result<Pcm, DecodeError> {
    hca::decode(data, key)
}
