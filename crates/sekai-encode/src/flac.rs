//! FLAC, the canonical audio form. Written in pure Rust, so native and WebAssembly builds give the same bytes.

use sekai_codec::Pcm;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "flac";

/// The encoder version.
pub const VERSION: u32 = 1;

/// Writes `pcm` as a 16-bit FLAC stream.
pub fn encode(pcm: &Pcm) -> Result<Vec<u8>, EncodeError> {
    let _ = (pcm,);
    Err(EncodeError::not_written(NAME))
}
