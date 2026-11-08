//! FLAC, the canonical audio form. Written in pure Rust, so native and WebAssembly builds give
//! the same bytes.

use flacenc::component::BitRepr;
use flacenc::error::Verify;
use flacenc::source::MemSource;
use sekai_codec::Pcm;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "flac";

/// The encoder version.
pub const VERSION: u32 = 1;

/// The samples per channel in one FLAC frame.
const BLOCK_SIZE: usize = 4096;

/// Writes `pcm` as a 16-bit FLAC stream.
///
/// FLAC carries at most eight channels; other channel counts fail.
pub fn encode(pcm: &Pcm) -> Result<Vec<u8>, EncodeError> {
    let failed = |message: String| EncodeError::Failed {
        encoder: NAME,
        message,
    };
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| failed(e.to_string()))?;
    let samples: Vec<i32> = pcm.samples().iter().map(|&s| s.into()).collect();
    let source = MemSource::from_samples(
        &samples,
        pcm.channels().into(),
        16,
        pcm.sample_rate() as usize,
    );
    let mut stream = flacenc::encode_with_fixed_block_size(&config, source, BLOCK_SIZE)
        .map_err(|e| failed(e.to_string()))?;
    // The stream header's minimum block size excludes the last block, which may be shorter.
    // Every other block has the fixed size.
    stream
        .stream_info_mut()
        .set_block_sizes(BLOCK_SIZE, BLOCK_SIZE)
        .map_err(|e| failed(e.to_string()))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream.write(&mut sink).map_err(|e| failed(e.to_string()))?;
    Ok(sink.as_slice().to_vec())
}
