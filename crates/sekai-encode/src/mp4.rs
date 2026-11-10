//! MP4 files with H.264 video and AAC audio, written by the `ffmpeg` program.
//!
//! The program is the one the environment variable `SEKAI_FFMPEG` names, or `ffmpeg` on the
//! search path. It runs with a fixed thread count, because x264's output depends on that count.

use std::ffi::{OsStr, OsString};
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use sekai_codec::{Movie, Pcm};

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "mp4-h264-aac";

/// The encoder version.
pub const VERSION: u32 = 3;

/// The threads of one x264 encoder. They encode slices of the same frame, so the output does not
/// depend on how the threads are timed.
const VIDEO_THREADS: &str = "2";
/// The longest error text kept from the program.
const MESSAGE_LEN: usize = 400;

static NEXT: AtomicU64 = AtomicU64::new(0);

fn failed(message: impl ToString) -> EncodeError {
    EncodeError::Failed {
        encoder: NAME,
        message: message.to_string(),
    }
}

/// A directory removed when it goes out of scope.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> io::Result<Self> {
        for _ in 0..64 {
            let number = NEXT.fetch_add(1, Ordering::Relaxed);
            let name = format!("sekai-mp4-{}-{number}", std::process::id());
            let path = std::env::temp_dir().join(name);
            match std::fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique scratch directory",
        ))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `pcm` as a WAV file.
fn wav(pcm: &Pcm) -> Result<Vec<u8>, EncodeError> {
    let channels = pcm.channels();
    let rate = pcm.sample_rate();
    let block_align = channels
        .checked_mul(2)
        .ok_or_else(|| failed("WAV block alignment overflows"))?;
    let byte_rate = rate
        .checked_mul(u32::from(block_align))
        .ok_or_else(|| failed("WAV byte rate overflows"))?;
    let size = pcm
        .samples()
        .len()
        .checked_mul(2)
        .and_then(|bytes| u32::try_from(bytes).ok())
        .ok_or_else(|| failed("WAV audio exceeds 4 GiB"))?;
    let riff_size = 36u32
        .checked_add(size)
        .ok_or_else(|| failed("WAV RIFF size overflows"))?;
    let mut out = Vec::with_capacity(44 + size as usize);
    out.extend(b"RIFF");
    out.extend(riff_size.to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(channels.to_le_bytes());
    out.extend(rate.to_le_bytes());
    out.extend(byte_rate.to_le_bytes());
    out.extend(block_align.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(size.to_le_bytes());
    for sample in pcm.samples() {
        out.extend(sample.to_le_bytes());
    }
    Ok(out)
}

/// The arguments that write `output` from the stream at `video` and the optional WAV at `audio`.
fn arguments(movie: &Movie, video: &Path, audio: Option<&Path>, output: &Path) -> Vec<OsString> {
    fn add(args: &mut Vec<OsString>, values: &[&str]) {
        args.extend(values.iter().map(OsString::from));
    }

    let mut args = Vec::new();
    let framerate = format!("{}/{}", movie.framerate_n, movie.framerate_d);
    add(
        &mut args,
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "mpegvideo",
            "-framerate",
            &framerate,
            "-i",
        ],
    );
    args.push(video.into());
    if let Some(audio) = audio {
        add(&mut args, &["-i"]);
        args.push(audio.into());
    }
    add(&mut args, &["-map", "0:v:0"]);
    if audio.is_some() {
        add(&mut args, &["-map", "1:a:0", "-c:a", "aac", "-b:a", "192k"]);
    }
    if !movie.width.is_multiple_of(2) || !movie.height.is_multiple_of(2) {
        add(&mut args, &["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2"]);
    }
    add(
        &mut args,
        &[
            "-fps_mode",
            "passthrough",
            "-c:v",
            "libx264",
            "-preset",
            "medium",
            "-b:v",
            "4M",
            "-g",
            "12",
            "-bf",
            "2",
            "-pix_fmt",
            "yuv420p",
            "-threads",
            VIDEO_THREADS,
            "-x264-params",
            "sliced-threads=1",
            "-map_metadata",
            "-1",
            "-fflags",
            "+bitexact",
            "-flags:v",
            "+bitexact",
            "-flags:a",
            "+bitexact",
            "-movflags",
            "+faststart",
        ],
    );
    args.push(output.into());
    args
}

#[cfg(not(target_arch = "wasm32"))]
fn program() -> OsString {
    std::env::var_os("SEKAI_FFMPEG").unwrap_or_else(|| "ffmpeg".into())
}

/// Encodes `movie` with the `ffmpeg` program.
#[cfg(not(target_arch = "wasm32"))]
pub fn encode(movie: &Movie) -> Result<Vec<u8>, EncodeError> {
    encode_with(&program(), movie)
}

/// Reports that this target cannot run the `ffmpeg` program.
#[cfg(target_arch = "wasm32")]
pub fn encode(_movie: &Movie) -> Result<Vec<u8>, EncodeError> {
    Err(EncodeError::Unavailable {
        encoder: NAME,
        message: "this target cannot run programs".to_owned(),
    })
}

/// Encodes `movie` with `program`.
pub fn encode_with(program: &OsStr, movie: &Movie) -> Result<Vec<u8>, EncodeError> {
    let scratch = Scratch::new().map_err(failed)?;
    let video = scratch.0.join("video.m2v");
    std::fs::write(&video, &movie.stream).map_err(failed)?;
    let audio = scratch.0.join("audio.wav");
    if let Some(pcm) = &movie.audio {
        std::fs::write(&audio, wav(pcm)?).map_err(failed)?;
    }
    let output = scratch.0.join("output.mp4");
    let args = arguments(
        movie,
        &video,
        movie.audio.as_ref().map(|_| audio.as_path()),
        &output,
    );
    let result = Command::new(program).args(args).output().map_err(|e| {
        if e.kind() == io::ErrorKind::NotFound {
            EncodeError::Unavailable {
                encoder: NAME,
                message: format!("{} was not found", program.display()),
            }
        } else {
            failed(e)
        }
    })?;
    if !result.status.success() {
        let text = String::from_utf8_lossy(&result.stderr);
        return Err(failed(text.chars().take(MESSAGE_LEN).collect::<String>()));
    }
    std::fs::read(&output).map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn movie(width: u32, audio: bool) -> Movie {
        Movie {
            stream: vec![1, 2, 3],
            width,
            height: 360,
            framerate_n: 30000,
            framerate_d: 1001,
            audio: audio.then(|| Pcm::new(48000, 2, vec![1, -1]).unwrap()),
        }
    }

    fn strings(movie: &Movie, audio: Option<&Path>) -> Vec<String> {
        arguments(movie, Path::new("v"), audio, Path::new("o"))
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_frame_rate_and_codecs_are_fixed_by_the_arguments() {
        let args = strings(&movie(640, true), Some(Path::new("a")));
        assert!(args.iter().any(|a| a == "30000/1001"));
        assert!(args.iter().any(|a| a == "libx264"));
        assert!(args.iter().any(|a| a == "aac"));
        assert!(args.iter().any(|a| a == "+faststart"));
        assert!(args.iter().any(|a| a == "sliced-threads=1"));
        assert!(!args.iter().any(|a| a == "-vf"));
    }

    #[test]
    fn a_silent_movie_has_one_input() {
        let args = strings(&movie(640, false), None);
        assert_eq!(args.iter().filter(|a| *a == "-i").count(), 1);
        assert!(!args.iter().any(|a| a == "aac"));
    }

    #[test]
    fn odd_sizes_are_padded() {
        assert!(strings(&movie(641, false), None).iter().any(|a| a == "-vf"));
    }

    #[test]
    fn a_missing_program_is_unavailable() {
        let error = encode_with(OsStr::new("sekai-no-such-program"), &movie(640, false));
        assert!(matches!(error, Err(EncodeError::Unavailable { .. })));
    }

    #[test]
    fn wav_has_the_header_of_its_samples() {
        let bytes = wav(&Pcm::new(48000, 2, vec![1, -1]).unwrap()).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 48);
        assert_eq!(&bytes[44..], [1, 0, 0xFF, 0xFF]);
    }

    #[test]
    fn wav_rejects_overflowing_byte_rate() {
        let pcm = Pcm::new(u32::MAX, 2, vec![0, 0]).unwrap();
        assert!(matches!(wav(&pcm), Err(EncodeError::Failed { .. })));
    }
}
