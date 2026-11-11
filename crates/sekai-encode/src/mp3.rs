//! MP3, the canonical audio form: MPEG audio layer III at a constant bitrate.
//!
//! The encoder is pure Rust and computes with functions whose results do not depend on the
//! target, so a sound encodes to the same bytes everywhere. A stream is decoded while it is
//! encoded; its samples are never all held at once.

use rusty_mp3::{Mp3Encoder, Mp3EncoderConfig};
use sekai_codec::Sound;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "mp3";

/// The encoder version.
pub const VERSION: u32 = 3;

/// The bitrates of MPEG-1, for 32, 44.1 and 48 kHz, in kbit/s.
const MPEG1: [u32; 14] = [
    32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
];
/// The bitrates of MPEG-2 and MPEG-2.5, for 8 to 24 kHz, in kbit/s.
const MPEG2: [u32; 14] = [8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];
/// Samples per channel handed to the encoder at a time.
const BLOCK: usize = 16384;

fn failed(message: String) -> EncodeError {
    EncodeError::Failed {
        encoder: NAME,
        message,
    }
}

/// The bitrates the MPEG version of `sample_rate` has, or `None` for a rate no version has.
fn bitrates(sample_rate: u32) -> Option<&'static [u32]> {
    match sample_rate {
        32_000 | 44_100 | 48_000 => Some(&MPEG1),
        8_000 | 11_025 | 12_000 | 16_000 | 22_050 | 24_000 => Some(&MPEG2),
        _ => None,
    }
}

/// The bitrate a sound of `sample_rate` is encoded at, in kbit/s: one and a half times
/// `source`, the bitrate of its lossy source in bit/s, rounded up to a bitrate of its MPEG
/// version and at most the highest one. Decoded samples, which have no lossy source, take the
/// highest.
pub fn bitrate(sample_rate: u32, source: Option<u32>) -> Result<u32, EncodeError> {
    let rates = bitrates(sample_rate)
        .ok_or_else(|| failed(format!("no MPEG version has {sample_rate} Hz")))?;
    let highest = rates[rates.len() - 1];
    Ok(match source {
        Some(source) => {
            let wanted = u64::from(source) * 3 / 2;
            let fits = rates.iter().find(|&&rate| u64::from(rate) * 1000 >= wanted);
            fits.copied().unwrap_or(highest)
        }
        None => highest,
    })
}

/// Encodes `sound` at [`bitrate`], with an Info frame that gives its length.
///
/// Fails when the sound has more than two channels, no samples or a sample rate no MPEG version
/// has, or when a stream does not decode.
pub fn encode(sound: &Sound) -> Result<Vec<u8>, EncodeError> {
    let (channels, sample_rate) = (sound.channels(), sound.sample_rate());
    if channels > 2 {
        return Err(failed(format!("{channels} channels")));
    }
    if sound.frames() == 0 {
        return Err(failed("a sound without samples has no frames".to_owned()));
    }
    let mut encoder = Mp3Encoder::new(Mp3EncoderConfig {
        bitrate_kbps: bitrate(sample_rate, sound.bitrate())?,
        vbr_quality: None,
    });
    let mut pushed = Ok(());
    sound
        .decode_each(|samples| {
            for block in samples.chunks(BLOCK * usize::from(channels)) {
                if pushed.is_ok() {
                    pushed = encoder.push_pcm_s16(block, channels, sample_rate);
                }
            }
        })
        .map_err(|e| failed(e.to_string()))?;
    pushed.map_err(|e| failed(e.to_string()))?;
    // The Info frame comes first, and the encoder writes it once it has every frame, so the
    // packets are taken only after the last one.
    encoder.finish();
    let mut bytes = Vec::new();
    while let Ok(packet) = encoder.next_packet() {
        bytes.extend(packet);
    }
    Ok(bytes)
}
