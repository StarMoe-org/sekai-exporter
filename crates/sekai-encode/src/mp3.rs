//! MP3 through LAME, loaded at run time in native builds.

#![allow(unsafe_code)]

use std::ffi::{CStr, c_char, c_int, c_short, c_uchar, c_void};
use std::sync::OnceLock;

use libloading::{Library, Symbol};
use sekai_codec::Pcm;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "mp3";

/// The encoder version.
pub const VERSION: u32 = 2;

/// The library names tried in order.
const LIBRARIES: [&str; 6] = [
    "libmp3lame-0.dll",
    "libmp3lame.dll",
    "libmp3lame.so.0",
    "libmp3lame.so",
    "libmp3lame.0.dylib",
    "libmp3lame.dylib",
];
const MINIMUM_VERSION: (u32, u32) = (3, 100);
const QUALITY: c_int = 2;
const MODE_JOINT_STEREO: c_int = 1;
const MODE_MONO: c_int = 3;
const VBR_OFF: c_int = 0;
/// Samples per channel handed to LAME at a time.
const BLOCK: usize = 16384;
/// The room `lame_encode_flush` may need.
const FLUSH_ROOM: usize = 7200;

type Handle = *mut c_void;
type SetInt = unsafe extern "C" fn(Handle, c_int) -> c_int;
type Init = unsafe extern "C" fn() -> Handle;
type Finish = unsafe extern "C" fn(Handle) -> c_int;
type Version = unsafe extern "C" fn() -> *const c_char;
type EncodeBuffer = unsafe extern "C" fn(
    Handle,
    *const c_short,
    *const c_short,
    c_int,
    *mut c_uchar,
    c_int,
) -> c_int;
type Flush = unsafe extern "C" fn(Handle, *mut c_uchar, c_int) -> c_int;
type TagFrame = unsafe extern "C" fn(*mut c_void, *mut u8, usize) -> usize;

fn unavailable(message: String) -> EncodeError {
    EncodeError::Unavailable {
        encoder: NAME,
        message,
    }
}

fn failed(message: String) -> EncodeError {
    EncodeError::Failed {
        encoder: NAME,
        message,
    }
}

fn symbol<'a, T>(library: &'a Library, name: &[u8]) -> Result<Symbol<'a, T>, EncodeError> {
    // SAFETY: each name is looked up with the signature LAME declares for it in `lame.h`.
    unsafe { library.get(name) }
        .map_err(|error| unavailable(format!("LAME has no symbol: {error}")))
}

fn load(libraries: &[&str]) -> Result<Library, EncodeError> {
    libraries
        .iter()
        // SAFETY: loading runs the library's initializers; LAME's only set up its own state.
        .find_map(|name| unsafe { Library::new(*name) }.ok())
        .ok_or_else(|| {
            unavailable(format!(
                "no LAME library found, tried {}",
                libraries.join(", ")
            ))
        })
}

fn check_version(library: &Library) -> Result<String, EncodeError> {
    let version: Symbol<'_, Version> = symbol(library, b"get_lame_version\0")?;
    // SAFETY: the function returns a static NUL-terminated string.
    let text = unsafe { CStr::from_ptr(version()) }
        .to_string_lossy()
        .into_owned();
    let mut parts = text.split(['.', ' ']).map(|part| part.parse::<u32>().ok());
    let major = parts.next().flatten();
    let minor = parts.next().flatten();
    match (major, minor) {
        (Some(major), Some(minor)) if (major, minor) >= MINIMUM_VERSION => Ok(text),
        _ => Err(unavailable(format!(
            "LAME {text} is older than {}.{}",
            MINIMUM_VERSION.0, MINIMUM_VERSION.1
        ))),
    }
}

/// A LAME encoder state, closed when dropped.
struct State<'a> {
    handle: Handle,
    close: Symbol<'a, Finish>,
}

impl Drop for State<'_> {
    fn drop(&mut self) {
        // SAFETY: the handle came from `lame_init` and is closed only here.
        unsafe { (self.close)(self.handle) };
    }
}

fn configure(
    library: &Library,
    handle: Handle,
    name: &[u8],
    value: c_int,
) -> Result<(), EncodeError> {
    let setter: Symbol<'_, SetInt> = symbol(library, name)?;
    // SAFETY: the handle is valid, and the setter only stores the value.
    if unsafe { setter(handle, value) } < 0 {
        let name = String::from_utf8_lossy(&name[..name.len() - 1]).into_owned();
        return Err(failed(format!("{name} rejected {value}")));
    }
    Ok(())
}

fn open_library(libraries: &[&str]) -> Result<(Library, String), EncodeError> {
    let library = load(libraries)?;
    let version = check_version(&library)?;
    Ok((library, version))
}

fn default_library() -> &'static Result<(Library, String), EncodeError> {
    static DEFAULT: OnceLock<Result<(Library, String), EncodeError>> = OnceLock::new();
    DEFAULT.get_or_init(|| open_library(&LIBRARIES))
}

/// The version string reported by the host `libmp3lame` library, when present.
pub fn library_version() -> Option<&'static str> {
    default_library()
        .as_ref()
        .ok()
        .map(|(_, version)| version.as_str())
}

/// Writes `pcm` as a constant-bitrate MP3 stream.
pub fn encode(pcm: &Pcm, bitrate_kbps: u32) -> Result<Vec<u8>, EncodeError> {
    let (library, _) = default_library().as_ref().map_err(Clone::clone)?;
    run(library, pcm, bitrate_kbps)
}

/// Writes `pcm` as in [`encode`], loading LAME from the first of `libraries` that exists.
pub fn encode_with(
    libraries: &[&str],
    pcm: &Pcm,
    bitrate_kbps: u32,
) -> Result<Vec<u8>, EncodeError> {
    let (library, _) = open_library(libraries)?;
    run(&library, pcm, bitrate_kbps)
}

fn run(library: &Library, pcm: &Pcm, bitrate_kbps: u32) -> Result<Vec<u8>, EncodeError> {
    let channels = usize::from(pcm.channels());
    if !(1..=2).contains(&channels) {
        return Err(failed(format!("LAME cannot write {channels} channels")));
    }
    let init: Symbol<'_, Init> = symbol(library, b"lame_init\0")?;
    let state = State {
        // SAFETY: `lame_init` takes no arguments and returns an owned state.
        handle: unsafe { init() },
        close: symbol(library, b"lame_close\0")?,
    };
    let handle = state.handle;
    if handle.is_null() {
        return Err(failed("lame_init returned nothing".to_owned()));
    }
    let rate = c_int::try_from(pcm.sample_rate()).map_err(|e| failed(e.to_string()))?;
    let bitrate = c_int::try_from(bitrate_kbps).map_err(|e| failed(e.to_string()))?;
    let mode = if channels == 1 {
        MODE_MONO
    } else {
        MODE_JOINT_STEREO
    };
    for (name, value) in [
        (&b"lame_set_num_channels\0"[..], channels as c_int),
        (b"lame_set_in_samplerate\0", rate),
        (b"lame_set_out_samplerate\0", rate),
        (b"lame_set_brate\0", bitrate),
        (b"lame_set_mode\0", mode),
        (b"lame_set_VBR\0", VBR_OFF),
        (b"lame_set_bWriteVbrTag\0", 1),
        (b"lame_set_quality\0", QUALITY),
    ] {
        configure(library, handle, name, value)?;
    }
    let init_params: Symbol<'_, Finish> = symbol(library, b"lame_init_params\0")?;
    // SAFETY: the handle is valid and fully configured.
    if unsafe { init_params(handle) } < 0 {
        return Err(failed(format!(
            "LAME rejected {rate} Hz at {bitrate_kbps} kbps"
        )));
    }

    let encode_buffer: Symbol<'_, EncodeBuffer> = symbol(library, b"lame_encode_buffer\0")?;
    let flush: Symbol<'_, Flush> = symbol(library, b"lame_encode_flush\0")?;
    let left: Vec<i16> = pcm.samples().iter().step_by(channels).copied().collect();
    let right: Vec<i16> = pcm
        .samples()
        .iter()
        .skip(channels - 1)
        .step_by(channels)
        .copied()
        .collect();
    let mut out = Vec::new();
    let mut buffer = vec![0u8; BLOCK + BLOCK / 4 + FLUSH_ROOM];
    for (left, right) in left.chunks(BLOCK).zip(right.chunks(BLOCK)) {
        // SAFETY: both channels hold `left.len()` samples, and the buffer has the room LAME
        // documents for a block of this size.
        let written = unsafe {
            encode_buffer(
                handle,
                left.as_ptr(),
                right.as_ptr(),
                left.len() as c_int,
                buffer.as_mut_ptr(),
                buffer.len() as c_int,
            )
        };
        let written = usize::try_from(written)
            .map_err(|_| failed(format!("lame_encode_buffer returned {written}")))?;
        out.extend_from_slice(&buffer[..written]);
    }
    // SAFETY: the buffer has at least the room `lame_encode_flush` documents.
    let written = unsafe { flush(handle, buffer.as_mut_ptr(), buffer.len() as c_int) };
    let written = usize::try_from(written)
        .map_err(|_| failed(format!("lame_encode_flush returned {written}")))?;
    out.extend_from_slice(&buffer[..written]);

    // The first frame of the stream is reserved for the tag that tells players how many samples
    // to skip at each end.
    let tag: Symbol<'_, TagFrame> = symbol(library, b"lame_get_lametag_frame\0")?;
    // SAFETY: the buffer is writable for the length passed with it.
    let written = unsafe { tag(handle, buffer.as_mut_ptr(), buffer.len()) };
    if written == 0 || written > out.len() {
        return Err(failed(format!("lame_get_lametag_frame returned {written}")));
    }
    out[..written].copy_from_slice(&buffer[..written]);
    Ok(out)
}
