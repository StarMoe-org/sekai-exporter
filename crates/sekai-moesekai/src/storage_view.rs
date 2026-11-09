//! The `storage` view: the path tree of the asset storage, computed from asset documents.
//!
//! A file sits at `<bundle>/<file name>`, where the bundle is the one that holds the root object
//! of its asset. The format of each file is given in `docs/spec/storage-view.md`.

use std::fmt;

use sekai_codec::{Pcm, Surface};
use sekai_convert::Content;
use sekai_encode::{mp3, png};
use sekai_format::AssetDocument;

/// The version of the view. It changes whenever a path or a transform changes.
pub const VERSION: u32 = 2;

/// The bit rate of the MP3 files.
const MP3_KBPS: u32 = 320;

/// How the file of a view is made from a member of its asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transform {
    /// The bytes of the member.
    Copy,
    /// A PNG of the WebP member.
    Png,
    /// A 320 kb/s MP3 of the FLAC member.
    Mp3,
}

impl Transform {
    /// The name and encoder version that identify the output of the transform.
    pub fn identity(self) -> String {
        match self {
            Self::Copy => "copy".to_owned(),
            Self::Png => format!("{} {}", png::NAME, png::VERSION),
            Self::Mp3 => format!("{} {} {MP3_KBPS}", mp3::NAME, mp3::VERSION),
        }
    }

    /// The extension of the file the transform writes, or `None` when it is the member's own.
    pub fn extension(self) -> Option<&'static str> {
        match self {
            Self::Copy => None,
            Self::Png => Some("png"),
            Self::Mp3 => Some("mp3"),
        }
    }

    /// The media type of the file the transform writes, or `None` when it is the member's own.
    pub fn media_type(self) -> Option<&'static str> {
        match self {
            Self::Copy => None,
            Self::Png => Some("image/png"),
            Self::Mp3 => Some("audio/mpeg"),
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

/// Whether the asset gives up a path that it shares with another one. A texture keeps the path
/// over a sprite of the same name.
pub fn yields_path(document: &AssetDocument) -> bool {
    document.form != "texture"
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
    let file = match document.form.as_str() {
        "texture" => file.strip_suffix("@Texture2D").unwrap_or(file),
        _ => file,
    };
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
            ("texture" | "sprite", "image") if m.extension == "webp" => {
                add(at(format!("{file}.webp")), Transform::Copy);
                add(at(format!("{file}.png")), Transform::Png);
            }
            ("texture", _) => add(at(format!("{file}.{}", m.extension)), Transform::Copy),
            ("audio", role) => {
                if let Some(cue) = role.strip_prefix("cue/")
                    && !is_alternative_mix(bundle, cue)
                {
                    add(at(format!("{}.mp3", cue.replace('/', "_"))), Transform::Mp3);
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

fn decode_webp(bytes: &[u8]) -> Result<Surface, ViewError> {
    let mut decoder =
        image_webp::WebPDecoder::new(std::io::Cursor::new(bytes)).map_err(malformed)?;
    let (width, height) = decoder.dimensions();
    let size = decoder
        .output_buffer_size()
        .ok_or_else(|| malformed("the image is too large"))?;
    let mut pixels = vec![0; size];
    decoder.read_image(&mut pixels).map_err(malformed)?;
    if !decoder.has_alpha() {
        pixels = pixels
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 0xff])
            .collect();
    }
    Surface::new(width, height, pixels).map_err(malformed)
}

fn decode_flac(bytes: &[u8]) -> Result<Pcm, ViewError> {
    let mut reader = claxon::FlacReader::new(bytes).map_err(malformed)?;
    let info = reader.streaminfo();
    if info.bits_per_sample != 16 {
        return Err(malformed(format!(
            "{} bits per sample",
            info.bits_per_sample
        )));
    }
    let samples = reader
        .samples()
        .map(|s| s.map(|s| s as i16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(malformed)?;
    let channels = u16::try_from(info.channels).map_err(malformed)?;
    Pcm::new(info.sample_rate, channels, samples).map_err(malformed)
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
        (Transform::Png, Content::Image(surface)) => Some(png::encode(surface).map_err(encoded)),
        (Transform::Mp3, Content::Audio(pcm)) => Some(mp3::encode(pcm, MP3_KBPS).map_err(encoded)),
        _ => None,
    }
}

/// Makes the file of a view from the bytes of its member. `Transform::Copy` is not applied.
pub fn apply(transform: Transform, member: &[u8]) -> Result<Vec<u8>, ViewError> {
    match transform {
        Transform::Copy => Ok(member.to_vec()),
        Transform::Png => png::encode(&decode_webp(member)?).map_err(encoded),
        Transform::Mp3 => mp3::encode(&decode_flac(member)?, MP3_KBPS).map_err(encoded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekai_encode::webp_lossless;
    use sekai_format::asset::{MemberFile, SourceObject};

    #[test]
    fn a_decoded_image_gives_the_png_of_its_webp() {
        let pixels = (0..16 * 4).map(|v| (v * 3) as u8).collect();
        let surface = Surface::new(4, 4, pixels).unwrap();
        let webp = webp_lossless::encode(&surface).unwrap();
        let direct = apply_decoded(Transform::Png, &Content::Image(surface))
            .unwrap()
            .unwrap();
        assert_eq!(direct, apply(Transform::Png, &webp).unwrap());
    }

    #[test]
    fn a_transform_does_not_read_other_content() {
        let pcm = Pcm::new(48000, 1, vec![0; 4]).unwrap();
        assert!(apply_decoded(Transform::Png, &Content::Audio(pcm)).is_none());
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
    fn images_have_a_webp_and_a_png() {
        let d = document(
            "character/member/a/card_normal",
            "texture",
            "character/member/a",
            &[("image", "webp")],
        );
        assert_eq!(
            paths(&d),
            [
                (
                    "character/member/a/card_normal.webp".to_owned(),
                    Transform::Copy
                ),
                (
                    "character/member/a/card_normal.png".to_owned(),
                    Transform::Png
                ),
            ]
        );
    }

    #[test]
    fn a_texture_takes_the_path_of_the_name_it_shares_with_a_sprite() {
        let texture = document("b/a@Texture2D", "texture", "b", &[("image", "webp")]);
        let sprite = document("b/a", "sprite", "b", &[("image", "webp")]);
        assert_eq!(files(&texture)[0].path, "b/a.webp");
        assert_eq!(files(&sprite)[0].path, "b/a.webp");
        assert!(!yields_path(&texture));
        assert!(yields_path(&sprite));
    }

    #[test]
    fn sprites_use_their_image_only() {
        let d = document(
            "b/s",
            "sprite",
            "b",
            &[("image", "webp"), ("sprite", "json")],
        );
        assert_eq!(files(&d).len(), 2);
        assert!(files(&d).iter().all(|f| f.member == 0));
    }

    #[test]
    fn each_cue_is_an_mp3_in_the_bundle() {
        let d = document(
            "sound/voice/a/deep/a.acb",
            "audio",
            "sound/voice/a",
            &[("cue/one", "flac"), ("cue/two/2", "flac")],
        );
        assert_eq!(
            paths(&d),
            [
                ("sound/voice/a/one.mp3".to_owned(), Transform::Mp3),
                ("sound/voice/a/two_2.mp3".to_owned(), Transform::Mp3),
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
                ("cue/a", "flac"),
                ("cue/a_VR", "flac"),
                ("cue/a_SCREEN", "flac"),
            ],
        );
        assert_eq!(
            paths(&d),
            [("music/long/a/a.mp3".to_owned(), Transform::Mp3)]
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
        let mut d = document("a/b", "texture", "a", &[("image", "webp")]);
        d.error = Some(sekai_format::asset::AssetError {
            code: "x".to_owned(),
            message: String::new(),
        });
        assert!(files(&d).is_empty());
    }

    #[test]
    fn a_png_has_the_pixels_of_the_webp() {
        let pixels = vec![10, 20, 30, 40, 50, 60, 70, 255];
        let surface = Surface::new(2, 1, pixels.clone()).unwrap();
        let webp = sekai_encode::webp_lossless::encode(&surface).unwrap();
        let png_bytes = apply(Transform::Png, &webp).unwrap();
        let decoder = ::png::Decoder::new(std::io::Cursor::new(png_bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut out = vec![0; reader.output_buffer_size().unwrap()];
        reader.next_frame(&mut out).unwrap();
        assert_eq!(out, pixels);
    }

    #[test]
    fn flac_is_decoded_to_its_samples() {
        let pcm = Pcm::new(48_000, 2, vec![1, -1, 300, -300]).unwrap();
        let flac = sekai_encode::flac::encode(&pcm).unwrap();
        let decoded = decode_flac(&flac).unwrap();
        assert_eq!(decoded.samples(), pcm.samples());
        assert_eq!(decoded.sample_rate(), 48_000);
        assert_eq!(decoded.channels(), 2);
    }

    #[test]
    fn broken_members_are_named_errors() {
        assert_eq!(
            apply(Transform::Png, b"nope").unwrap_err().code,
            "malformed"
        );
        assert_eq!(
            apply(Transform::Mp3, b"nope").unwrap_err().code,
            "malformed"
        );
    }
}
