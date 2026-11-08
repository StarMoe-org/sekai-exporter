//! MP3 through LAME, loaded at run time in native builds.

use sekai_codec::Pcm;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "mp3";

/// The encoder version.
pub const VERSION: u32 = 1;

/// Writes `pcm` as a constant-bitrate MP3 stream.
pub fn encode(pcm: &Pcm, bitrate_kbps: u32) -> Result<Vec<u8>, EncodeError> {
    let _ = (pcm, bitrate_kbps);
    Err(EncodeError::not_written(NAME))
}
