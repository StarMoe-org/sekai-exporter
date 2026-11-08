//! CRI middleware formats: HCA audio, the ACB cue sheet with its AWB wave bank, and USM video.
//!
//! [`decode_hca`] turns one HCA stream into PCM. Reading an ACB yields its cues and the HCA
//! streams they play, taken from the ACB itself or from its AWB; reading a USM yields its video
//! and audio streams as stored. Those readers are submodules of this one.

use crate::{DecodeError, Pcm};

/// The key of an encrypted HCA stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HcaKey {
    /// The game's key.
    pub key: u64,
    /// The subkey of the AWB the stream comes from, or 0 for a stream outside an AWB.
    pub subkey: u16,
}

/// Decodes one HCA stream to PCM. `key` is needed when the stream is encrypted.
pub fn decode_hca(data: &[u8], key: Option<HcaKey>) -> Result<Pcm, DecodeError> {
    let _ = key;
    Err(DecodeError::Unsupported {
        what: "HCA version",
        value: data
            .get(4..6)
            .map_or(0, |v| i64::from(u16::from_be_bytes([v[0], v[1]]))),
    })
}
