//! ADX decoding.
//!
//! Follows the decoder of vgmstream (see the notice in `hca_tables.rs`). Covers version 4 files
//! of encoding type 3 with four-bit samples, the form USM files carry.

use crate::{DecodeError, Pcm};

const FRAME_SIZE: usize = 0x12;
const SAMPLES_PER_FRAME: usize = 32;
const VERSION: u16 = 0x0400;
const ENCODING_TYPE: u8 = 3;
const SAMPLE_BITS: u8 = 4;
const HISTORY_OFFSET: usize = 0x18;
const SIGNATURE: [u8; 6] = *b"(c)CRI";

fn malformed(message: &str) -> DecodeError {
    DecodeError::Malformed {
        what: "ADX stream",
        message: message.into(),
    }
}

fn unsupported(what: &'static str, value: u32) -> DecodeError {
    DecodeError::Unsupported {
        what,
        value: value.into(),
    }
}

/// The cosine, from its series, so that every target computes the same value.
fn cosine(x: f64) -> f64 {
    let square = x * x;
    let mut term = 1.0;
    let mut sum = 1.0;
    for n in 1..30 {
        term *= -square / f64::from((2 * n - 1) * (2 * n));
        sum += term;
    }
    sum
}

/// The two filter coefficients of a cutoff frequency.
fn coefficients(cutoff: u16, sample_rate: u32) -> (i32, i32) {
    let x = f32::from(cutoff);
    let y = sample_rate as f32;
    let z = cosine(2.0 * std::f64::consts::PI * f64::from(x) / f64::from(y)) as f32;
    let a = (std::f64::consts::SQRT_2 - f64::from(z)) as f32;
    let b = (std::f64::consts::SQRT_2 - 1.0) as f32;
    let c = (a - ((a + b) * (a - b)).sqrt()) / b;
    (
        i32::from((c * 8192.0) as i16),
        i32::from((c * c * -4096.0) as i16),
    )
}

/// Decodes a stream that is not encrypted to PCM.
pub(super) fn decode(data: &[u8]) -> Result<Pcm, DecodeError> {
    if data.len() < HISTORY_OFFSET {
        return Err(DecodeError::Truncated {
            what: "ADX header",
            needed: HISTORY_OFFSET,
            available: data.len(),
        });
    }
    if data[..2] != [0x80, 0x00] {
        return Err(malformed("no ADX signature"));
    }
    let start = usize::from(u16::from_be_bytes([data[2], data[3]])) + 4;
    if start < HISTORY_OFFSET + SIGNATURE.len() || data.len() < start {
        return Err(malformed("the data offset is outside the file"));
    }
    if data[start - SIGNATURE.len()..start] != SIGNATURE {
        return Err(malformed("no copyright signature before the data"));
    }
    if data[4] != ENCODING_TYPE {
        return Err(unsupported("ADX encoding type", u32::from(data[4])));
    }
    if usize::from(data[5]) != FRAME_SIZE {
        return Err(unsupported("ADX frame size", u32::from(data[5])));
    }
    if data[6] != SAMPLE_BITS {
        return Err(unsupported("ADX sample bits", u32::from(data[6])));
    }
    let version = u16::from_be_bytes([data[0x12], data[0x13]]);
    if version != VERSION {
        return Err(unsupported("ADX version", u32::from(version)));
    }
    let channels = usize::from(data[7]);
    let sample_rate = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);
    let samples = u32::from_be_bytes([data[12], data[13], data[14], data[15]]) as usize;
    if channels == 0 || sample_rate == 0 || samples == 0 {
        return Err(malformed("empty format"));
    }
    let frames = samples.div_ceil(SAMPLES_PER_FRAME);
    let needed = frames
        .checked_mul(channels)
        .and_then(|count| count.checked_mul(FRAME_SIZE))
        .and_then(|size| size.checked_add(start))
        .ok_or_else(|| malformed("sample count is too large"))?;
    if data.len() < needed {
        return Err(DecodeError::Truncated {
            what: "ADX frames",
            needed,
            available: data.len(),
        });
    }
    if start < HISTORY_OFFSET + channels * 4 {
        return Err(malformed("the header has no room for the filter history"));
    }

    let cutoff = u16::from_be_bytes([data[0x10], data[0x11]]);
    let (coefficient1, coefficient2) = coefficients(cutoff, sample_rate);
    let mut output = vec![0i16; samples * channels];
    for channel in 0..channels {
        let at = HISTORY_OFFSET + channel * 4;
        let mut history1 = i32::from(i16::from_be_bytes([data[at], data[at + 1]]));
        let mut history2 = i32::from(i16::from_be_bytes([data[at + 2], data[at + 3]]));
        for index in 0..samples {
            let frame_index = index / SAMPLES_PER_FRAME;
            let frame =
                &data[start + (frame_index * channels + channel) * FRAME_SIZE..][..FRAME_SIZE];
            let stop = frame[0] == 0x80 && frame[1] == 0x01;
            let scale = if stop {
                0
            } else {
                i32::from(i16::from_be_bytes([frame[0], frame[1]])) + 1
            };
            let position = index % SAMPLES_PER_FRAME;
            let byte = frame[2 + position / 2];
            let nibble = if position.is_multiple_of(2) {
                byte >> 4
            } else {
                byte & 15
            };
            let delta = i32::from(((nibble << 4) as i8) >> 4);
            let predicted = coefficient1
                .wrapping_mul(history1)
                .wrapping_add(coefficient2.wrapping_mul(history2))
                >> 12;
            let sample = delta.wrapping_mul(scale).wrapping_add(predicted);
            let sample = sample.clamp(-32768, 32767);
            output[index * channels + channel] = sample as i16;
            history2 = history1;
            history1 = sample;
        }
    }
    Pcm::new(sample_rate, channels as u16, output)
}
