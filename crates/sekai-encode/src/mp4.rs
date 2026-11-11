//! MP4 files with H.264 video and AAC audio, written by the `ffmpeg` program.
//!
//! The program is the one the environment variable `SEKAI_FFMPEG` names, or `ffmpeg` on the
//! search path. It reads the video stream from a pipe and runs with fixed thread counts: one to
//! decode, and a fixed number to encode, because x264's output depends on that number.

use std::ffi::{OsStr, OsString};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use sekai_codec::{Movie, Pcm};

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "mp4-h264-aac";

/// The encoder version.
pub const VERSION: u32 = 4;

/// The threads that decode the video stream. The stream decodes to the same frames whatever the
/// count, and one keeps up with the encoder.
const DECODE_THREADS: &str = "1";
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

/// The arguments that write `output` from the stream on standard input and the optional WAV at
/// `audio`.
fn arguments(movie: &Movie, audio: Option<&Path>, output: &Path) -> Vec<OsString> {
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
            "-threads",
            DECODE_THREADS,
            "-i",
            "pipe:0",
        ],
    );
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
            "veryfast",
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
    let audio = scratch.0.join("audio.wav");
    if let Some(pcm) = &movie.audio {
        std::fs::write(&audio, wav(pcm)?).map_err(failed)?;
    }
    // The output is a file: the index that `+faststart` moves to the front is written last.
    let output = scratch.0.join("output.mp4");
    let args = arguments(
        movie,
        movie.audio.as_ref().map(|_| audio.as_path()),
        &output,
    );
    let result = run(program, args, &movie.stream)?;
    if !result.status.success() {
        let text = String::from_utf8_lossy(&result.stderr);
        return Err(failed(text.chars().take(MESSAGE_LEN).collect::<String>()));
    }
    std::fs::read(&output).map_err(failed)
}

/// Runs `program` with `args`, writes `input` to its standard input and waits for it.
fn run(
    program: &OsStr,
    args: Vec<OsString>,
    input: &[u8],
) -> Result<std::process::Output, EncodeError> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                EncodeError::Unavailable {
                    encoder: NAME,
                    message: format!("{} was not found", program.display()),
                }
            } else {
                failed(e)
            }
        })?;
    let mut stdin = child.stdin.take().expect("standard input is piped");
    std::thread::scope(|s| {
        // Standard input closes when the writer ends, which ends the stream.
        let writer = s.spawn(move || stdin.write_all(input));
        let output = child.wait_with_output().map_err(failed)?;
        let written = writer.join().expect("writing to the pipe does not panic");
        // A program that fails stops reading and breaks the pipe; its own error tells why.
        match written {
            Err(e) if output.status.success() => Err(failed(e)),
            _ => Ok(output),
        }
    })
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
        arguments(movie, audio, Path::new("o"))
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
    fn the_stream_is_read_from_a_pipe_by_one_decoder_thread() {
        let args = strings(&movie(640, false), None);
        let input = args.iter().position(|a| a == "pipe:0").unwrap();
        assert_eq!(args[input - 3..input], ["-threads", "1", "-i"]);
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
