//! AFS2 wave banks (AWB).

use crate::DecodeError;

const HEADER_SIZE: usize = 0x10;

fn malformed(message: &str) -> DecodeError {
    DecodeError::Malformed {
        what: "AWB",
        message: message.into(),
    }
}

fn le_uint(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rev()
        .fold(0, |value, &b| value << 8 | usize::from(b))
}

/// One stream of a wave bank.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AwbStream<'a> {
    /// The ID that the `MemoryAwbId` or `StreamAwbId` of a waveform refers to.
    pub wave_id: u32,
    /// The stream as stored.
    pub data: &'a [u8],
}

/// A wave bank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Awb<'a> {
    /// Combined with the game's key to decrypt the streams of this bank.
    pub subkey: u16,
    /// The streams in the order of the bank.
    pub streams: Vec<AwbStream<'a>>,
}

impl<'a> Awb<'a> {
    /// The stream with a wave ID.
    pub fn stream(&self, wave_id: u32) -> Option<&AwbStream<'a>> {
        self.streams.iter().find(|s| s.wave_id == wave_id)
    }
}

/// Reads a wave bank.
pub fn read_awb(data: &[u8]) -> Result<Awb<'_>, DecodeError> {
    if data.len() < HEADER_SIZE {
        return Err(DecodeError::Truncated {
            what: "AWB header",
            needed: HEADER_SIZE,
            available: data.len(),
        });
    }
    if &data[..4] != b"AFS2" {
        return Err(malformed("no AFS2 signature"));
    }
    let offset_size = usize::from(data[5]);
    let id_size = le_uint(&data[6..8]);
    let count = le_uint(&data[8..12]);
    let alignment = le_uint(&data[12..14]);
    let subkey = le_uint(&data[14..16]) as u16;
    if !matches!(offset_size, 2 | 4) {
        return Err(DecodeError::Unsupported {
            what: "AWB offset size",
            value: offset_size as i64,
        });
    }
    if !matches!(id_size, 2 | 4) {
        return Err(DecodeError::Unsupported {
            what: "AWB wave ID size",
            value: id_size as i64,
        });
    }
    if alignment == 0 {
        return Err(malformed("alignment is zero"));
    }
    let needed = count
        .checked_mul(id_size)
        .and_then(|ids| ids.checked_add(count.checked_add(1)?.checked_mul(offset_size)?))
        .and_then(|tables| tables.checked_add(HEADER_SIZE))
        .ok_or_else(|| malformed("stream count is too large"))?;
    if data.len() < needed {
        return Err(DecodeError::Truncated {
            what: "AWB tables",
            needed,
            available: data.len(),
        });
    }
    let offsets = HEADER_SIZE + count * id_size;
    let offset_at = |index: usize| le_uint(&data[offsets + index * offset_size..][..offset_size]);
    // Offsets are stored unaligned, except that the last one is the end of the file.
    let aligned = |offset: usize| offset.next_multiple_of(alignment);

    let mut streams = Vec::with_capacity(count);
    for index in 0..count {
        let wave_id = le_uint(&data[HEADER_SIZE + index * id_size..][..id_size]) as u32;
        let start = aligned(offset_at(index));
        let next = offset_at(index + 1);
        let end = if next < data.len() {
            aligned(next)
        } else {
            next
        };
        let stream = data
            .get(start..end)
            .ok_or_else(|| malformed("a stream lies outside the file"))?;
        streams.push(AwbStream {
            wave_id,
            data: stream,
        });
    }
    Ok(Awb { subkey, streams })
}
