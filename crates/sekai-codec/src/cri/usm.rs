//! Separating the streams of USM files.

use super::utf::Table;
use crate::DecodeError;

const CHUNK_HEADER: usize = 8;
const DATA_STREAM: u8 = 0;
const DATA_HEADER: u8 = 1;
const DATA_SEEK: u8 = 3;
const DATA_MARKER: u8 = 2;
const CONTENTS_END: &[u8] = b"#CONTENTS END";

fn malformed(message: impl Into<String>) -> DecodeError {
    DecodeError::Malformed {
        what: "USM",
        message: message.into(),
    }
}

/// One frame of a video stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFrame<'a> {
    /// The frame as stored.
    pub data: &'a [u8],
    /// The presentation time, in units of 1/[`VideoFrame::time_scale`] seconds.
    pub time: u32,
    /// The units of [`VideoFrame::time`] per second.
    pub time_scale: u32,
    /// Whether the file lists this frame as a seek point.
    pub seek_point: bool,
}

/// A video stream with the values of its `VIDEO_HDRINFO` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoStream<'a> {
    /// The channel number of the stream.
    pub channel: u8,
    /// The `mpeg_codec` value: 1 for MPEG-1, 5 for H.264, 9 for VP9.
    pub mpeg_codec: u32,
    /// The picture width in pixels.
    pub width: u32,
    /// The picture height in pixels.
    pub height: u32,
    /// The frame rate as a fraction.
    pub framerate_n: u32,
    /// The frame rate as a fraction.
    pub framerate_d: u32,
    /// The frames in file order.
    pub frames: Vec<VideoFrame<'a>>,
}

/// An audio stream with the values of its `AUDIO_HDRINFO` table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStream<'a> {
    /// The channel number of the stream.
    pub channel: u8,
    /// The `audio_codec` value: 2 for ADX, 4 for HCA.
    pub audio_codec: u32,
    /// The sample rate in hertz.
    pub sampling_rate: u32,
    /// The channel count.
    pub num_channels: u32,
    /// The chunks of the stream in file order. Their concatenation is the stored audio file.
    pub chunks: Vec<&'a [u8]>,
}

/// The streams of a USM file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usm<'a> {
    /// The video streams.
    pub video: Vec<VideoStream<'a>>,
    /// The audio streams.
    pub audio: Vec<AudioStream<'a>>,
}

struct Chunk<'a> {
    kind: [u8; 4],
    channel: u8,
    data_type: u8,
    time: u32,
    time_scale: u32,
    payload: &'a [u8],
}

fn chunks(data: &[u8]) -> impl Iterator<Item = Result<Chunk<'_>, DecodeError>> {
    let mut position = 0;
    std::iter::from_fn(move || {
        if position >= data.len() {
            return None;
        }
        Some(next_chunk(data, &mut position))
    })
}

fn next_chunk<'a>(data: &'a [u8], position: &mut usize) -> Result<Chunk<'a>, DecodeError> {
    let head = data
        .get(*position..*position + 0x20)
        .ok_or(DecodeError::Truncated {
            what: "USM chunk header",
            needed: *position + 0x20,
            available: data.len(),
        })?;
    let be32 = |at: usize| u32::from_be_bytes([head[at], head[at + 1], head[at + 2], head[at + 3]]);
    let size = be32(4) as usize;
    let end = CHUNK_HEADER + *position + size;
    if end > data.len() {
        return Err(DecodeError::Truncated {
            what: "USM chunk",
            needed: end,
            available: data.len(),
        });
    }
    let offset = usize::from(head[9]);
    let padding = usize::from(u16::from_be_bytes([head[10], head[11]]));
    if offset + padding > size {
        return Err(malformed("chunk padding exceeds the chunk"));
    }
    let chunk = Chunk {
        kind: [head[0], head[1], head[2], head[3]],
        channel: head[12],
        data_type: head[15] & 3,
        time: be32(16),
        time_scale: be32(20),
        payload: &data[*position + CHUNK_HEADER + offset..end - padding],
    };
    *position = end;
    Ok(chunk)
}

fn uint(table: &Table<'_>, name: &str) -> Result<u32, DecodeError> {
    table
        .int(0, name)?
        .map(|value| value as u32)
        .ok_or_else(|| malformed(format!("no {name} value")))
}

fn video_stream<'a>(channel: u8, table: &Table<'_>) -> Result<VideoStream<'a>, DecodeError> {
    Ok(VideoStream {
        channel,
        mpeg_codec: uint(table, "mpeg_codec")?,
        width: uint(table, "width")?,
        height: uint(table, "height")?,
        framerate_n: uint(table, "framerate_n")?,
        framerate_d: uint(table, "framerate_d")?,
        frames: Vec::new(),
    })
}

fn audio_stream<'a>(channel: u8, table: &Table<'_>) -> Result<AudioStream<'a>, DecodeError> {
    Ok(AudioStream {
        channel,
        audio_codec: uint(table, "audio_codec")?,
        sampling_rate: uint(table, "sampling_rate")?,
        num_channels: uint(table, "num_channels")?,
        chunks: Vec::new(),
    })
}

/// Separates the streams of a USM file without decoding them. Bytes after the end of the last
/// stream are not part of the file.
pub fn read_usm(data: &[u8]) -> Result<Usm<'_>, DecodeError> {
    if data.get(..4) != Some(b"CRID") {
        return Err(malformed("no CRID signature"));
    }
    let mut usm = Usm {
        video: Vec::new(),
        audio: Vec::new(),
    };
    let mut seek_points: Vec<(u8, u32)> = Vec::new();
    let mut ended = 0;
    for chunk in chunks(data) {
        let chunk = chunk?;
        match (&chunk.kind, chunk.data_type) {
            (b"CRID", _) => {}
            (b"@SFV" | b"@SFA", DATA_MARKER) if chunk.payload.starts_with(CONTENTS_END) => {
                ended += 1;
                if ended == usm.video.len() + usm.audio.len() {
                    break;
                }
            }
            (b"@SFV", DATA_HEADER) => {
                usm.video
                    .push(video_stream(chunk.channel, &Table::open(chunk.payload)?)?);
            }
            (b"@SFA", DATA_HEADER) => {
                usm.audio
                    .push(audio_stream(chunk.channel, &Table::open(chunk.payload)?)?);
            }
            (b"@SFV", DATA_SEEK) => {
                let table = Table::open(chunk.payload)?;
                for row in 0..table.rows() {
                    let frame = table.int(row, "ofs_frmid")?.unwrap_or(0) as u32;
                    seek_points.push((chunk.channel, frame));
                }
            }
            (b"@SFV", DATA_STREAM) => {
                let stream = usm
                    .video
                    .iter_mut()
                    .find(|stream| stream.channel == chunk.channel)
                    .ok_or_else(|| malformed("a video frame before its header"))?;
                stream.frames.push(VideoFrame {
                    data: chunk.payload,
                    time: chunk.time,
                    time_scale: chunk.time_scale,
                    seek_point: false,
                });
            }
            (b"@SFA", DATA_STREAM) => {
                let stream = usm
                    .audio
                    .iter_mut()
                    .find(|stream| stream.channel == chunk.channel)
                    .ok_or_else(|| malformed("an audio chunk before its header"))?;
                stream.chunks.push(chunk.payload);
            }
            (b"@SFV" | b"@SFA", _) => {}
            (kind, _) => {
                return Err(DecodeError::Unsupported {
                    what: "USM chunk",
                    value: i64::from(u32::from_be_bytes(*kind)),
                });
            }
        }
    }
    for (channel, frame) in seek_points {
        let found = usm
            .video
            .iter_mut()
            .find(|stream| stream.channel == channel)
            .and_then(|stream| stream.frames.get_mut(frame as usize));
        match found {
            Some(frame) => frame.seek_point = true,
            None => return Err(malformed("a seek point names a missing frame")),
        }
    }
    Ok(usm)
}
