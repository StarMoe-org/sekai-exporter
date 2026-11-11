//! The `storage` view: the path tree of the asset storage, computed from asset documents.
//!
//! A file sits at `<bundle>/<file name>`, where the bundle is the one that holds the root object
//! of its asset. The format of each file is given in `docs/spec/storage-view.md`.

use std::fmt;

use sekai_codec::Surface;
use sekai_convert::Content;
use sekai_encode::webp;
use sekai_format::AssetDocument;

/// The version of the view. It changes whenever a path or a transform changes.
pub const VERSION: u32 = 2;

/// The libwebp quality factor of the WebP files.
const WEBP_QUALITY: f32 = 75.0;

/// How the file of a view is made from a member of its asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    /// The bytes of the member.
    Copy,
    /// A lossy WebP of the PNG member.
    Webp,
}

impl Transform {
    /// The name and encoder version that identify the output of the transform.
    pub fn identity(self) -> String {
        match self {
            Self::Copy => "copy".to_owned(),
            Self::Webp => format!("{} {} {WEBP_QUALITY}", webp::NAME, webp::VERSION),
        }
    }

    /// The extension of the file the transform writes, or `None` when it is the member's own.
    pub fn extension(self) -> Option<&'static str> {
        match self {
            Self::Copy => None,
            Self::Webp => Some("webp"),
        }
    }

    /// The media type of the file the transform writes, or `None` when it is the member's own.
    pub fn media_type(self) -> Option<&'static str> {
        match self {
            Self::Copy => None,
            Self::Webp => Some("image/webp"),
        }
    }
}

/// One file of the view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewFile {
    /// The path of the file.
    pub path: String,
    /// The index of the member in the asset document.
    pub member: usize,
    /// How the file is made.
    pub transform: Transform,
}

/// A member that could not be transformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewError {
    /// A fixed code.
    pub code: &'static str,
    /// A description for people.
    pub message: String,
}

impl fmt::Display for ViewError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

fn malformed(message: impl fmt::Display) -> ViewError {
    ViewError {
        code: "malformed",
        message: message.to_string(),
    }
}

/// The segment suffix of a movie name: `-001.usm` for the first segment, `.usm` for a whole file.
fn movie_stem(file: &str) -> &str {
    let stem = file.strip_suffix(".usm").unwrap_or(file);
    match stem.rsplit_once('-') {
        Some((base, segment))
            if !base.is_empty()
                && !segment.is_empty()
                && segment.bytes().all(|b| b.is_ascii_digit()) =>
        {
            base
        }
        _ => stem,
    }
}

/// The name of a text file on the storage. A stored `.bytes` file has that extension removed; the
/// charts keep theirs as `.txt`.
fn text_name(bundle: &str, file: &str, extension: &str) -> String {
    match extension {
        "bytes" if bundle.starts_with("music/music_score") => format!("{file}.txt"),
        "bytes" => file.to_owned(),
        extension => format!("{file}.{extension}"),
    }
}

/// Whether a cue is one of the mixes of a full song that the storage does not carry.
fn is_alternative_mix(bundle: &str, cue: &str) -> bool {
    let cue = cue.to_ascii_lowercase();
    bundle.starts_with("music/long") && (cue.ends_with("_vr") || cue.ends_with("_screen"))
}

/// The files of an asset in the view. An asset with an error has none.
pub fn files(document: &AssetDocument) -> Vec<ViewFile> {
    let Some(source) = document.sources.first() else {
        return Vec::new();
    };
    if document.error.is_some() {
        return Vec::new();
    }
    let bundle = source.bundle.as_str();
    let file = document
        .name
        .rsplit_once('/')
        .map_or(document.name.as_str(), |(_, file)| file);
    let at = |name: String| format!("{bundle}/{name}");
    let mut out = Vec::new();
    for (member, m) in document.members.iter().enumerate() {
        let mut add = |path: String, transform| {
            out.push(ViewFile {
                path,
                member,
                transform,
            });
        };
        match (document.form.as_str(), m.role.as_str()) {
            ("texture" | "sprite", "image") if m.extension == "png" => {
                add(at(format!("{file}.png")), Transform::Copy);
                add(at(format!("{file}.webp")), Transform::Webp);
            }
            ("texture", "image") => add(at(format!("{file}.{}", m.extension)), Transform::Copy),
            ("audio", role) => {
                if let Some(cue) = role.strip_prefix("cue/")
                    && !is_alternative_mix(bundle, cue)
                {
                    let cue = cue.replace('/', "_");
                    add(at(format!("{cue}.{}", m.extension)), Transform::Copy);
                }
            }
            ("video", "video") => add(at(format!("{}.mp4", movie_stem(file))), Transform::Copy),
            ("scenario", "scenario") => add(at(format!("{file}.json")), Transform::Copy),
            ("text", "file") => add(at(text_name(bundle, file, &m.extension)), Transform::Copy),
            ("font", _) => add(at(format!("{file}.{}", m.extension)), Transform::Copy),
            _ => {}
        }
    }
    out
}

/// The pixels of a canonical image, which is an 8-bit RGBA PNG.
fn decode_png(bytes: &[u8]) -> Result<Surface, ViewError> {
    let mut reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .map_err(malformed)?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| malformed("the image is too large"))?;
    let mut pixels = vec![0; size];
    let frame = reader.next_frame(&mut pixels).map_err(malformed)?;
    if (frame.color_type, frame.bit_depth) != (png::ColorType::Rgba, png::BitDepth::Eight) {
        return Err(malformed("the image is not 8-bit RGBA"));
    }
    pixels.truncate(frame.buffer_size());
    Surface::new(frame.width, frame.height, pixels).map_err(malformed)
}

fn encoded(e: sekai_encode::EncodeError) -> ViewError {
    ViewError {
        code: e.code(),
        message: e.to_string(),
    }
}

/// Makes the file of a view from a member that is still decoded, with the bytes [`apply`] gives
/// for the member's file. `None` if the transform does not read this kind of content.
pub fn apply_decoded(
    transform: Transform,
    content: &Content,
) -> Option<Result<Vec<u8>, ViewError>> {
    match (transform, content) {
        (Transform::Webp, Content::Image(surface)) => {
            Some(webp::encode(surface, WEBP_QUALITY).map_err(encoded))
        }
        _ => None,
    }
}

/// Makes the file of a view from the bytes of its member. `Transform::Copy` is not applied.
pub fn apply(transform: Transform, member: &[u8]) -> Result<Vec<u8>, ViewError> {
    match transform {
        Transform::Copy => Ok(member.to_vec()),
        Transform::Webp => webp::encode(&decode_png(member)?, WEBP_QUALITY).map_err(encoded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekai_format::asset::{MemberFile, SourceObject};

    #[test]
    fn a_decoded_image_gives_the_webp_of_its_png() {
        let pixels = (0..16 * 4).map(|v| (v * 3) as u8).collect();
        let surface = Surface::new(4, 4, pixels).unwrap();
        let png = sekai_encode::png::encode(&surface).unwrap();
        let direct = apply_decoded(Transform::Webp, &Content::Image(surface))
            .unwrap()
            .unwrap();
        assert_eq!(direct, apply(Transform::Webp, &png).unwrap());
        assert_eq!((&direct[..4], &direct[8..12]), (&b"RIFF"[..], &b"WEBP"[..]));
    }

    #[test]
    fn a_transform_does_not_read_other_content() {
        let pcm = sekai_codec::Pcm::new(48000, 1, vec![0; 4]).unwrap();
        let sound = Content::Audio(sekai_codec::Sound::Pcm(pcm));
        assert!(apply_decoded(Transform::Webp, &sound).is_none());
    }

    fn document(name: &str, form: &str, bundle: &str, members: &[(&str, &str)]) -> AssetDocument {
        AssetDocument {
            format: sekai_format::asset::FORMAT.to_owned(),
            version: 1,
            name: name.to_owned(),
            form: form.to_owned(),
            form_version: 1,
            fingerprint: String::new(),
            sources: vec![SourceObject {
                bundle: bundle.to_owned(),
                path_id: 1,
                class: "Object".to_owned(),
            }],
            members: members
                .iter()
                .map(|(role, extension)| MemberFile {
                    role: (*role).to_owned(),
                    sha256: String::new(),
                    size: 0,
                    extension: (*extension).to_owned(),
                    media_type: String::new(),
                })
                .collect(),
            resources: Vec::new(),
            references: Vec::new(),
            error: None,
        }
    }

    fn paths(document: &AssetDocument) -> Vec<(String, Transform)> {
        files(document)
            .into_iter()
            .map(|f| (f.path, f.transform))
            .collect()
    }

    #[test]
    fn images_have_a_png_and_a_webp() {
        let d = document(
            "character/member/a/card_normal",
            "texture",
            "character/member/a",
            &[("image", "png")],
        );
        assert_eq!(
            paths(&d),
            [
                (
                    "character/member/a/card_normal.png".to_owned(),
                    Transform::Copy
                ),
                (
                    "character/member/a/card_normal.webp".to_owned(),
                    Transform::Webp
                ),
            ]
        );
    }

    #[test]
    fn sprites_use_their_image_only() {
        let d = document(
            "b/s",
            "sprite",
            "b",
            &[("image", "png"), ("sprite", "json")],
        );
        assert_eq!(files(&d).len(), 2);
        assert!(files(&d).iter().all(|f| f.member == 0));
    }

    #[test]
    fn texture_slices_are_not_part_of_the_view() {
        let slices = document(
            "b/lut",
            "texture",
            "b",
            &[("slice/0", "png"), ("slice/1", "png")],
        );
        assert!(files(&slices).is_empty());

        let hdr = document("b/sky", "texture", "b", &[("image", "exr")]);
        assert_eq!(paths(&hdr), [("b/sky.exr".to_owned(), Transform::Copy)]);
    }

    #[test]
    fn each_cue_is_its_mp3_in_the_bundle() {
        let d = document(
            "sound/voice/a/deep/a.acb",
            "audio",
            "sound/voice/a",
            &[("cue/one", "mp3"), ("cue/two/2", "mp3")],
        );
        assert_eq!(
            paths(&d),
            [
                ("sound/voice/a/one.mp3".to_owned(), Transform::Copy),
                ("sound/voice/a/two_2.mp3".to_owned(), Transform::Copy),
            ]
        );
    }

    #[test]
    fn alternative_mixes_of_full_songs_are_left_out() {
        let d = document(
            "music/long/a/a.acb",
            "audio",
            "music/long/a",
            &[
                ("cue/a", "mp3"),
                ("cue/a_VR", "mp3"),
                ("cue/a_SCREEN", "mp3"),
            ],
        );
        assert_eq!(
            paths(&d),
            [("music/long/a/a.mp3".to_owned(), Transform::Copy)]
        );
    }

    #[test]
    fn movies_lose_their_segment_suffix() {
        let d = document("m/x/x-001.usm", "video", "m/x", &[("video", "mp4")]);
        assert_eq!(paths(&d), [("m/x/x.mp4".to_owned(), Transform::Copy)]);
        let d = document("m/x/x-y.usm", "video", "m/x", &[("video", "mp4")]);
        assert_eq!(paths(&d), [("m/x/x-y.mp4".to_owned(), Transform::Copy)]);
    }

    #[test]
    fn text_files_follow_their_extension() {
        let chart = document(
            "music/music_score/1/expert",
            "text",
            "music/music_score/1",
            &[("file", "bytes")],
        );
        assert_eq!(paths(&chart)[0].0, "music/music_score/1/expert.txt");
        let plain = document("a/b.json", "text", "a", &[("file", "bytes")]);
        assert_eq!(paths(&plain)[0].0, "a/b.json");
        let named = document("a/b", "text", "a", &[("file", "txt")]);
        assert_eq!(paths(&named)[0].0, "a/b.txt");
    }

    #[test]
    fn an_asset_with_an_error_has_no_files() {
        let mut d = document("a/b", "texture", "a", &[("image", "png")]);
        d.error = Some(sekai_format::asset::AssetError {
            code: "x".to_owned(),
            message: String::new(),
        });
        assert!(files(&d).is_empty());
    }

    #[test]
    fn a_canonical_png_is_decoded_to_its_pixels() {
        let pixels = vec![10, 20, 30, 40, 50, 60, 70, 255];
        let surface = Surface::new(2, 1, pixels.clone()).unwrap();
        let png = sekai_encode::png::encode(&surface).unwrap();
        assert_eq!(decode_png(&png).unwrap().pixels(), pixels);
    }

    #[test]
    fn a_broken_member_is_a_named_error() {
        assert_eq!(
            apply(Transform::Webp, b"nope").unwrap_err().code,
            "malformed"
        );
    }
}
