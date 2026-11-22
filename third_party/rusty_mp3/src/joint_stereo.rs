//! Rewriting a stream's stereo frames in joint stereo mode.
//!
//! Local change: not in the published crate. A frame in stereo mode decodes exactly like the
//! same frame in joint stereo mode with mode extension 0, so the rewrite changes the headers
//! alone, and the checksums that cover them: the frame CRC of a protected frame and the music
//! and tag CRCs of a LAME extension.

use crate::encode::bitstream::{crc16, crc16_arc};
use crate::frame::ChannelMode;
use crate::header::FrameHeader;
use crate::{Error, Result};

/// Joint stereo with neither mid/side nor intensity stereo: left and right, as in stereo mode.
const LEFT_RIGHT: ChannelMode = ChannelMode::JointStereo {
    ms_stereo: false,
    intensity_stereo: false,
};

/// Rewrites every frame of `stream` in stereo mode as joint stereo with mode extension 0 and
/// returns whether one changed. The stream decodes to the same samples.
///
/// Fails when `stream` is not a sequence of whole Layer III frames; it is then left unchanged.
pub fn rewrite(stream: &mut [u8]) -> Result<bool> {
    let mut frames = frames(stream)?;
    let mut changed = false;
    for (at, header) in &mut frames {
        if header.channel_mode == ChannelMode::Stereo {
            header.channel_mode = LEFT_RIGHT;
            write_header(&mut stream[*at..], header);
            changed = true;
        }
    }
    if changed {
        if let Some((_, first)) = frames.first() {
            update_lame_checksums(stream, first);
        }
    }
    Ok(changed)
}

/// The offset and header of each frame of `stream`.
fn frames(stream: &[u8]) -> Result<Vec<(usize, FrameHeader)>> {
    let mut frames = Vec::new();
    let mut at = 0;
    while at < stream.len() {
        let bytes = stream[at..]
            .first_chunk::<4>()
            .ok_or_else(|| Error::invalid("mp3 stream: truncated frame header"))?;
        let header = FrameHeader::parse(*bytes)?;
        let end = at + header.frame_size();
        if end > stream.len() {
            return Err(Error::invalid("mp3 stream: truncated frame"));
        }
        frames.push((at, header));
        at = end;
    }
    Ok(frames)
}

/// Writes `header` over the start of `frame`, and the frame CRC when the frame has one.
fn write_header(frame: &mut [u8], header: &FrameHeader) {
    let bytes = header.to_bytes();
    frame[..4].copy_from_slice(&bytes);
    if header.crc_protected {
        let side_info = &frame[6..6 + header.side_info_len()];
        let crc = crc16([bytes[2], bytes[3]], side_info);
        frame[4..6].copy_from_slice(&crc.to_be_bytes());
    }
}

/// Recomputes the music CRC and the tag CRC of the LAME extension, when the first frame, whose
/// header is `first`, is an Info or Xing frame that has one.
fn update_lame_checksums(stream: &mut [u8], first: &FrameHeader) {
    let Some(lame) = lame_extension(stream, first) else {
        return;
    };
    let music_crc = crc16_arc(0, &stream[first.frame_size()..]);
    stream[lame + 32..lame + 34].copy_from_slice(&music_crc.to_be_bytes());
    let tag_crc = crc16_arc(0, &stream[..lame + 34]);
    stream[lame + 34..lame + 36].copy_from_slice(&tag_crc.to_be_bytes());
}

/// The offset of the LAME extension in the first frame of `stream`, whose header is `first`.
fn lame_extension(stream: &[u8], first: &FrameHeader) -> Option<usize> {
    let tag = 4 + if first.crc_protected { 2 } else { 0 } + first.side_info_len();
    let frame = &stream[..first.frame_size()];
    if !matches!(frame.get(tag..tag + 4), Some(b"Info" | b"Xing")) {
        return None;
    }
    let flags = u32::from_be_bytes(frame.get(tag + 4..tag + 8)?.try_into().ok()?);
    // The fields the flags announce: the frame count, the byte count, the table of contents
    // and the quality.
    let fields: usize = [(1, 4), (2, 4), (4, 100), (8, 4)]
        .iter()
        .filter(|(flag, _)| flags & flag != 0)
        .map(|(_, len)| len)
        .sum();
    let lame = tag + 8 + fields;
    (frame.get(lame..lame + 4)? == b"LAME" && lame + 36 <= frame.len()).then_some(lame)
}
