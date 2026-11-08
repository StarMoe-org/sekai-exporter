//! Lists the streams of USM files, for comparing with a reference.
//!
//! Each argument is a USM file. Lines printed: `video <path> <channel> <codec> <width> <height>
//! <rate n> <rate d> <frames> <bytes> <sha256 of frames> <first time> <second time> <time scale>
//! <seek points>` and `audio <path> <channel> <codec> <rate> <channels> <bytes> <sha256>`.
//! With `--write <dir>`, the audio of each stream is also saved as `<file>.<channel>.audio`.

use std::fs;
use std::path::Path;

use sekai_codec::cri::read_usm;
use sha2::{Digest, Sha256};

fn digest(parts: impl Iterator<Item = impl AsRef<[u8]>>) -> (usize, String) {
    let mut hash = Sha256::new();
    let mut total = 0;
    for part in parts {
        hash.update(part.as_ref());
        total += part.as_ref().len();
    }
    let text = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    (total, text)
}

fn main() -> std::io::Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut write = None;
    if args.first().is_some_and(|a| a == "--write") {
        write = Some(args[1].clone());
        args.drain(..2);
    }
    for path in args {
        let data = fs::read(&path)?;
        let usm = match read_usm(&data) {
            Ok(usm) => usm,
            Err(error) => {
                println!("error {path} {error}");
                continue;
            }
        };
        for video in &usm.video {
            let (bytes, sha) = digest(video.frames.iter().map(|f| f.data));
            let time = |i: usize| video.frames.get(i).map_or(0, |f| f.time);
            println!(
                "video {path} {} {} {} {} {} {} {} {bytes} {sha} {} {} {} {}",
                video.channel,
                video.mpeg_codec,
                video.width,
                video.height,
                video.framerate_n,
                video.framerate_d,
                video.frames.len(),
                time(0),
                time(1),
                video.frames.first().map_or(0, |f| f.time_scale),
                video.frames.iter().filter(|f| f.seek_point).count(),
            );
        }
        for audio in &usm.audio {
            let (bytes, sha) = digest(audio.chunks.iter());
            println!(
                "audio {path} {} {} {} {} {bytes} {sha}",
                audio.channel, audio.audio_codec, audio.sampling_rate, audio.num_channels
            );
            if let Some(directory) = &write {
                let name = Path::new(&path).file_name().expect("a file name");
                let target = Path::new(directory).join(format!(
                    "{}.{}.audio",
                    name.to_string_lossy(),
                    audio.channel
                ));
                fs::write(target, audio.chunks.concat())?;
            }
        }
    }
    Ok(())
}
