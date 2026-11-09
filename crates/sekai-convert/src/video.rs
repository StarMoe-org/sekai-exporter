//! Video from CRI USM files.
//!
//! [`Video`] produces the `video` form, specified in `docs/spec/video.md`.

use sekai_codec::Movie;
use sekai_codec::cri::{AudioStream, VideoStream, decode_adx_available, decode_hca, read_usm};
use sekai_unity::Value;

use crate::{
    Asset, Content, Context, ConvertError, Converter, Cost, CostClass, Member, ObjectId,
    ObjectIndex,
};

/// The class ID of `TextAsset`.
const TEXT_ASSET: i32 = 49;
/// The container path ending of a USM file or of one of its segments.
const SUFFIX: &str = ".usm.bytes";
/// The `mpeg_codec` value of MPEG-1.
const MPEG1: u32 = 1;
/// The `audio_codec` value of ADX.
const ADX: u32 = 2;
/// The `audio_codec` value of HCA.
const HCA: u32 = 4;
/// The start code an MPEG-1 video stream begins with.
const SEQUENCE_HEADER: [u8; 4] = [0, 0, 1, 0xB3];
/// The memory of the encoder process, which runs beside the evaluator.
const ENCODER_MEMORY: u64 = 256 << 20;

/// USM files, as a video with its audio.
#[derive(Debug, Clone, Copy, Default)]
pub struct Video;

/// The `<base>` and number of a container path of the form `<base>-<NNN>.usm.bytes`.
fn segment(path: &str) -> Option<(&str, u32)> {
    let (base, number) = path.strip_suffix(SUFFIX)?.rsplit_once('-')?;
    if number.len() != 3 || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((base, number.parse().ok()?))
}

fn malformed(message: impl Into<String>) -> ConvertError {
    ConvertError::new("malformed", message)
}

fn unsupported(message: impl Into<String>) -> ConvertError {
    ConvertError::new("unsupported", message)
}

fn script(id: ObjectId, cx: &dyn Context) -> Result<Vec<u8>, ConvertError> {
    let object = cx.object(id)?;
    object
        .fields
        .get("m_Script")
        .and_then(Value::as_bytes)
        .map(<[u8]>::to_vec)
        .ok_or_else(|| malformed("m_Script is missing or is not a string"))
}

/// The bytes of the USM file `root` starts, joined from its segments in number order.
fn file(root: ObjectId, cx: &dyn Context) -> Result<Vec<u8>, ConvertError> {
    let entries = cx.container(root)?;
    let own = entries
        .iter()
        .find(|(path, id)| *id == root && path.ends_with(SUFFIX))
        .ok_or_else(|| malformed("the object has no USM container path"))?;
    let Some((base, _)) = segment(&own.0) else {
        return script(root, cx);
    };
    let mut parts: Vec<(u32, ObjectId)> = entries
        .iter()
        .filter_map(|(path, id)| {
            let (other, number) = segment(path)?;
            (other == base).then_some((number, *id))
        })
        .collect();
    parts.sort_by_key(|(number, _)| *number);
    if let Some((expected, _)) = (1u32..)
        .zip(&parts)
        .find(|(expected, (number, _))| expected != number)
    {
        return Err(ConvertError::new(
            "missing_segment",
            format!("segment {expected} of {base} is missing"),
        ));
    }
    let mut data = Vec::new();
    for (_, id) in parts {
        data.extend(script(id, cx)?);
    }
    Ok(data)
}

/// The elementary stream of `stream`, checked to be plain MPEG-1.
fn elementary(stream: &VideoStream<'_>) -> Result<Vec<u8>, ConvertError> {
    if stream.mpeg_codec != MPEG1 {
        return Err(unsupported(format!("video codec {}", stream.mpeg_codec)));
    }
    if !stream
        .frames
        .first()
        .is_some_and(|frame| frame.data.starts_with(&SEQUENCE_HEADER))
    {
        return Err(unsupported(
            "the video does not begin with an MPEG-1 sequence header",
        ));
    }
    Ok(stream.frames.iter().flat_map(|f| f.data).copied().collect())
}

fn audio(stream: &AudioStream<'_>) -> Result<sekai_codec::Pcm, ConvertError> {
    let data = stream.chunks.concat();
    Ok(match stream.audio_codec {
        ADX => decode_adx_available(&data)?,
        HCA => decode_hca(&data, None)?,
        codec => return Err(unsupported(format!("audio codec {codec}"))),
    })
}

impl Converter for Video {
    fn name(&self) -> &'static str {
        "video"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        _script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        object.class_id == TEXT_ASSET
            && container_paths.iter().any(|path| {
                path.ends_with(SUFFIX) && segment(path).is_none_or(|(_, number)| number == 1)
            })
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let data = file(root, cx)?;
        let usm = read_usm(&data)?;
        let [stream] = usm.video.as_slice() else {
            return Err(unsupported(format!("{} video streams", usm.video.len())));
        };
        let track = match usm.audio.as_slice() {
            [] => None,
            [track] => Some(audio(track)?),
            tracks => return Err(unsupported(format!("{} audio tracks", tracks.len()))),
        };
        let movie = Movie {
            stream: elementary(stream)?,
            width: stream.width,
            height: stream.height,
            framerate_n: stream.framerate_n,
            framerate_d: stream.framerate_d,
            audio: track,
        };
        Ok(Some(Asset {
            members: vec![Member {
                role: "video".to_owned(),
                content: Content::Video(movie),
            }],
        }))
    }

    /// Four times the file, taken as the stored size of `root` times its segment count: the
    /// joined file, the elementary stream, the decoded audio and the encoded movie. The encoder
    /// process adds a fixed amount.
    fn cost(&self, root: ObjectId, object: &ObjectIndex, cx: &dyn Context) -> Cost {
        let file = u64::from(object.size).saturating_mul(segments(root, cx).unwrap_or(1));
        Cost {
            class: CostClass::Wide,
            memory: file.saturating_mul(4).saturating_add(ENCODER_MEMORY),
        }
    }
}

/// The number of segments of the USM file `root` starts, from the container of its bundle.
fn segments(root: ObjectId, cx: &dyn Context) -> Option<u64> {
    let entries = cx.container(root).ok()?;
    let (path, _) = entries
        .iter()
        .find(|(path, id)| *id == root && path.ends_with(SUFFIX))?;
    let Some((base, _)) = segment(path) else {
        return Some(1);
    };
    let count = entries
        .iter()
        .filter(|(path, _)| segment(path).is_some_and(|(other, _)| other == base))
        .count();
    Some(count as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_are_numbered_with_three_digits() {
        assert_eq!(segment("a/b-001.usm.bytes"), Some(("a/b", 1)));
        assert_eq!(segment("a/01-016.usm.bytes"), Some(("a/01", 16)));
        assert_eq!(segment("a/b.usm.bytes"), None);
        assert_eq!(segment("a/b-1.usm.bytes"), None);
        assert_eq!(segment("a/b-001.acb.bytes"), None);
    }
}
