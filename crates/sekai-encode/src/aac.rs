//! AAC audio tracks through libavcodec, loaded at run time in native builds.
//!
//! The output is the raw frames and decoder configuration that an MP4 audio track stores, so
//! the MP4 writer can place them without parsing a container.
//!
//! The libraries are driven through their options and functions. The few structure fields that
//! have no accessor are read at offsets known per major version on 64-bit platforms, listed in
//! [`LAYOUTS`]; other versions and platforms are reported as unavailable.

#![allow(unsafe_code)]

use std::ffi::{CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::OnceLock;

use libloading::{Library, Symbol};
use sekai_codec::Pcm;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "aac";

/// The encoder version.
pub const VERSION: u32 = 1;

/// The sample format `AV_SAMPLE_FMT_FLTP`.
const SAMPLE_FORMAT: c_int = 8;
/// The error `AVERROR(EAGAIN)`, which means "provide more input".
const AGAIN: c_int = -11;
/// The error `AVERROR_EOF`.
const END: c_int = -541_478_725;
/// The level `AV_LOG_QUIET`; failures are returned as values instead.
const LOG_QUIET: c_int = -8;
/// The encoder delay libavcodec's AAC encoder reports, in samples per channel.
const ENCODER_DELAY: u32 = 1024;
/// The number of plane pointers held inline in `AVFrame::data`.
const MAX_CHANNELS: usize = 8;

/// Where fields live, for one pair of library major versions.
#[derive(Clone, Copy)]
struct Layout {
    avcodec_major: u32,
    avutil_major: u32,
    context_sample_format: usize,
    frame_sample_rate: usize,
    frame_channel_layout: usize,
}

const LAYOUTS: [Layout; 5] = [
    Layout {
        avcodec_major: 59,
        avutil_major: 57,
        context_sample_format: 360,
        frame_sample_rate: 208,
        frame_channel_layout: 448,
    },
    Layout {
        avcodec_major: 60,
        avutil_major: 58,
        context_sample_format: 360,
        frame_sample_rate: 208,
        frame_channel_layout: 448,
    },
    Layout {
        avcodec_major: 61,
        avutil_major: 59,
        context_sample_format: 348,
        frame_sample_rate: 192,
        frame_channel_layout: 408,
    },
    Layout {
        avcodec_major: 62,
        avutil_major: 60,
        context_sample_format: 348,
        frame_sample_rate: 180,
        frame_channel_layout: 384,
    },
    Layout {
        avcodec_major: 63,
        avutil_major: 61,
        context_sample_format: 348,
        frame_sample_rate: 180,
        frame_channel_layout: 384,
    },
];
const FRAME_NB_SAMPLES: usize = 112;
const FRAME_FORMAT: usize = 116;
const PACKET_DATA: usize = 24;
const PACKET_SIZE: usize = 32;
const PARAMETERS_EXTRADATA: usize = 16;
const PARAMETERS_EXTRADATA_SIZE: usize = 24;

/// An encoded AAC-LC track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AacTrack {
    /// The `AudioSpecificConfig` of the track.
    pub config: Vec<u8>,
    /// The raw frames, in order. Every frame holds `samples_per_frame` samples per channel.
    pub frames: Vec<Vec<u8>>,
    /// The samples per channel in one frame.
    pub samples_per_frame: u32,
    /// The samples per channel at the start of the first frame that are encoder delay.
    pub delay: u32,
}

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

type Pointer = *mut c_void;
type VersionFn = unsafe extern "C" fn() -> u32;

struct Api<'a> {
    avcodec: &'a Library,
    avutil: &'a Library,
}

impl<'a> Api<'a> {
    fn codec<T>(&self, name: &[u8]) -> Result<Symbol<'a, T>, EncodeError> {
        // SAFETY: every name is looked up with the signature libavcodec declares for it.
        unsafe { self.avcodec.get(name) }
            .map_err(|error| unavailable(format!("libavcodec has no symbol: {error}")))
    }

    fn util<T>(&self, name: &[u8]) -> Result<Symbol<'a, T>, EncodeError> {
        // SAFETY: every name is looked up with the signature libavutil declares for it.
        unsafe { self.avutil.get(name) }
            .map_err(|error| unavailable(format!("libavutil has no symbol: {error}")))
    }
}

/// The library names tried for each pair of major versions.
fn library_names(layout: &Layout) -> Vec<(String, String)> {
    let (codec, util) = (layout.avcodec_major, layout.avutil_major);
    vec![
        (format!("avcodec-{codec}.dll"), format!("avutil-{util}.dll")),
        (
            format!("libavcodec.so.{codec}"),
            format!("libavutil.so.{util}"),
        ),
        (
            format!("libavcodec.{codec}.dylib"),
            format!("libavutil.{util}.dylib"),
        ),
    ]
}

/// Frees what libavcodec allocated, whatever path leaves the function.
struct Owned<'a> {
    pointer: Pointer,
    free: Symbol<'a, unsafe extern "C" fn(*mut Pointer)>,
}

impl Owned<'_> {
    fn new<'a>(
        pointer: Pointer,
        free: Symbol<'a, unsafe extern "C" fn(*mut Pointer)>,
        what: &str,
    ) -> Result<Owned<'a>, EncodeError> {
        if pointer.is_null() {
            return Err(failed(format!("{what} could not be allocated")));
        }
        Ok(Owned { pointer, free })
    }
}

impl Drop for Owned<'_> {
    fn drop(&mut self) {
        // SAFETY: the pointer came from the allocator this free function belongs to.
        unsafe { (self.free)(&mut self.pointer) };
    }
}

/// Reads a field of a structure at a known offset.
///
/// # Safety
/// `base` must point to a live structure with a field of type `T` at `offset`.
unsafe fn field<T: Copy>(base: Pointer, offset: usize) -> T {
    // SAFETY: guaranteed by the caller.
    unsafe { base.cast::<u8>().add(offset).cast::<T>().read_unaligned() }
}

/// Writes a field of a structure at a known offset.
///
/// # Safety
/// `base` must point to a live structure with a field of type `T` at `offset`.
unsafe fn set_field<T>(base: Pointer, offset: usize, value: T) {
    // SAFETY: guaranteed by the caller.
    unsafe {
        base.cast::<u8>()
            .add(offset)
            .cast::<T>()
            .write_unaligned(value)
    };
}

fn check(result: c_int, what: &str) -> Result<(), EncodeError> {
    if result < 0 {
        return Err(failed(format!("{what} returned {result}")));
    }
    Ok(())
}

fn text(value: &str) -> CString {
    CString::new(value).expect("no NUL in the fixed option names")
}

struct Loaded {
    avcodec: Library,
    avutil: Library,
    layout: Layout,
}

fn open_libraries(libraries: &[(&str, &str)]) -> Result<Loaded, EncodeError> {
    let (avcodec, avutil) = libraries
        .iter()
        .find_map(|(codec, util)| {
            // SAFETY: loading runs the initializers of libavcodec and libavutil, which only set
            // up their own state.
            let avcodec = unsafe { Library::new(*codec) }.ok()?;
            // SAFETY: as above.
            let avutil = unsafe { Library::new(*util) }.ok()?;
            Some((avcodec, avutil))
        })
        .ok_or_else(|| {
            let tried: Vec<&str> = libraries.iter().map(|pair| pair.0).collect();
            unavailable(format!(
                "no libavcodec library found, tried {}",
                tried.join(", ")
            ))
        })?;
    let api = Api {
        avcodec: &avcodec,
        avutil: &avutil,
    };
    let layout = find_layout(&api)?;
    let set_log_level: Symbol<'_, unsafe extern "C" fn(c_int)> = api.util(b"av_log_set_level\0")?;
    // SAFETY: the function only stores the level once when the library is opened.
    unsafe { set_log_level(LOG_QUIET) };
    Ok(Loaded {
        avcodec,
        avutil,
        layout,
    })
}

/// Encodes `pcm` as a constant-bitrate AAC-LC track.
pub fn encode(pcm: &Pcm, bitrate_kbps: u32) -> Result<AacTrack, EncodeError> {
    static DEFAULT: OnceLock<Result<Loaded, EncodeError>> = OnceLock::new();
    let loaded = DEFAULT
        .get_or_init(|| {
            let names: Vec<(String, String)> = LAYOUTS.iter().flat_map(library_names).collect();
            let pairs: Vec<(&str, &str)> = names
                .iter()
                .map(|(codec, util)| (codec.as_str(), util.as_str()))
                .collect();
            open_libraries(&pairs)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let api = Api {
        avcodec: &loaded.avcodec,
        avutil: &loaded.avutil,
    };
    run(&api, loaded.layout, pcm, bitrate_kbps)
}

/// Encodes as in [`encode`], loading libavcodec and libavutil from the first pair that exists.
pub fn encode_with(
    libraries: &[(&str, &str)],
    pcm: &Pcm,
    bitrate_kbps: u32,
) -> Result<AacTrack, EncodeError> {
    let loaded = open_libraries(libraries)?;
    let api = Api {
        avcodec: &loaded.avcodec,
        avutil: &loaded.avutil,
    };
    run(&api, loaded.layout, pcm, bitrate_kbps)
}

fn find_layout(api: &Api<'_>) -> Result<Layout, EncodeError> {
    if size_of::<usize>() != 8 {
        return Err(unavailable(
            "the structure layouts are known for 64-bit builds".to_owned(),
        ));
    }
    let codec_version: Symbol<'_, VersionFn> = api.codec(b"avcodec_version\0")?;
    let util_version: Symbol<'_, VersionFn> = api.util(b"avutil_version\0")?;
    // SAFETY: both functions take no arguments and return a number.
    let (codec_major, util_major) = unsafe { (codec_version() >> 16, util_version() >> 16) };
    LAYOUTS
        .iter()
        .find(|l| l.avcodec_major == codec_major && l.avutil_major == util_major)
        .copied()
        .ok_or_else(|| {
            unavailable(format!(
                "libavcodec {codec_major} with libavutil {util_major} is not a supported version"
            ))
        })
}

fn run(
    api: &Api<'_>,
    layout: Layout,
    pcm: &Pcm,
    bitrate_kbps: u32,
) -> Result<AacTrack, EncodeError> {
    let channels = usize::from(pcm.channels());
    if !(1..=MAX_CHANNELS).contains(&channels) {
        return Err(failed(format!("AAC cannot write {channels} channels")));
    }
    let find_encoder: Symbol<'_, unsafe extern "C" fn(*const c_char) -> Pointer> =
        api.codec(b"avcodec_find_encoder_by_name\0")?;
    let name = text("aac");
    // SAFETY: the name is a valid C string.
    let codec = unsafe { find_encoder(name.as_ptr()) };
    if codec.is_null() {
        return Err(unavailable("libavcodec has no aac encoder".to_owned()));
    }
    let alloc_context: Symbol<'_, unsafe extern "C" fn(Pointer) -> Pointer> =
        api.codec(b"avcodec_alloc_context3\0")?;
    // SAFETY: the codec pointer is valid.
    let context = Owned::new(
        unsafe { alloc_context(codec) },
        api.codec(b"avcodec_free_context\0")?,
        "the codec context",
    )?;

    let set_int: Symbol<'_, unsafe extern "C" fn(Pointer, *const c_char, i64, c_int) -> c_int> =
        api.util(b"av_opt_set_int\0")?;
    let set_text: Symbol<
        '_,
        unsafe extern "C" fn(Pointer, *const c_char, *const c_char, c_int) -> c_int,
    > = api.util(b"av_opt_set\0")?;
    let set_layout: Symbol<
        '_,
        unsafe extern "C" fn(Pointer, *const c_char, *const u8, c_int) -> c_int,
    > = api.util(b"av_opt_set_chlayout\0")?;
    let default_layout: Symbol<'_, unsafe extern "C" fn(*mut u8, c_int)> =
        api.util(b"av_channel_layout_default\0")?;
    let copy_layout: Symbol<'_, unsafe extern "C" fn(*mut u8, *const u8) -> c_int> =
        api.util(b"av_channel_layout_copy\0")?;

    let mut channel_layout = [0u8; 64];
    // SAFETY: the buffer is larger than `AVChannelLayout`.
    unsafe { default_layout(channel_layout.as_mut_ptr(), channels as c_int) };
    // SAFETY: the option names are C strings and the context is live.
    unsafe {
        let name = text("b");
        check(
            set_int(
                context.pointer,
                name.as_ptr(),
                i64::from(bitrate_kbps) * 1000,
                0,
            ),
            "setting the bitrate",
        )?;
        let name = text("ar");
        check(
            set_int(
                context.pointer,
                name.as_ptr(),
                i64::from(pcm.sample_rate()),
                0,
            ),
            "setting the sample rate",
        )?;
        let name = text("threads");
        check(
            set_int(context.pointer, name.as_ptr(), 1, 0),
            "setting the threads",
        )?;
        let name = text("ch_layout");
        check(
            set_layout(context.pointer, name.as_ptr(), channel_layout.as_ptr(), 0),
            "setting the channel layout",
        )?;
        let (name, value) = (text("flags"), text("+bitexact"));
        check(
            set_text(context.pointer, name.as_ptr(), value.as_ptr(), 0),
            "setting the flags",
        )?;
    }

    // SAFETY: the context is live and the offset is that of this version.
    unsafe { set_field(context.pointer, layout.context_sample_format, SAMPLE_FORMAT) };

    let open: Symbol<'_, unsafe extern "C" fn(Pointer, Pointer, *mut Pointer) -> c_int> =
        api.codec(b"avcodec_open2\0")?;
    // SAFETY: the context and codec are valid, and no option dictionary is passed.
    check(
        unsafe { open(context.pointer, codec, ptr::null_mut()) },
        "avcodec_open2",
    )?;

    let get_int: Symbol<
        '_,
        unsafe extern "C" fn(Pointer, *const c_char, c_int, *mut i64) -> c_int,
    > = api.util(b"av_opt_get_int\0")?;
    let mut frame_size = 0i64;
    let name = text("frame_size");
    // SAFETY: the name is a C string and the output is a live integer.
    check(
        unsafe { get_int(context.pointer, name.as_ptr(), 0, &mut frame_size) },
        "reading the frame size",
    )?;
    let frame_size = usize::try_from(frame_size)
        .ok()
        .filter(|&size| size > 0)
        .ok_or_else(|| failed(format!("the encoder reported frame size {frame_size}")))?;

    let config = read_config(api, context.pointer)?;

    let alloc_frame: Symbol<'_, unsafe extern "C" fn() -> Pointer> =
        api.util(b"av_frame_alloc\0")?;
    let get_buffer: Symbol<'_, unsafe extern "C" fn(Pointer, c_int) -> c_int> =
        api.util(b"av_frame_get_buffer\0")?;
    let writable: Symbol<'_, unsafe extern "C" fn(Pointer) -> c_int> =
        api.util(b"av_frame_make_writable\0")?;
    // SAFETY: the allocator takes no arguments.
    let frame = Owned::new(
        unsafe { alloc_frame() },
        api.util(b"av_frame_free\0")?,
        "the frame",
    )?;
    // SAFETY: the frame was allocated by `av_frame_alloc`; the offsets are those of this version.
    unsafe {
        set_field(frame.pointer, FRAME_NB_SAMPLES, frame_size as c_int);
        set_field(frame.pointer, FRAME_FORMAT, SAMPLE_FORMAT);
        set_field(
            frame.pointer,
            layout.frame_sample_rate,
            pcm.sample_rate() as c_int,
        );
        let target = frame.pointer.cast::<u8>().add(layout.frame_channel_layout);
        check(
            copy_layout(target, channel_layout.as_ptr()),
            "copying the channel layout",
        )?;
        check(get_buffer(frame.pointer, 0), "av_frame_get_buffer")?;
    }

    let alloc_packet: Symbol<'_, unsafe extern "C" fn() -> Pointer> =
        api.codec(b"av_packet_alloc\0")?;
    let packet = Owned::new(
        // SAFETY: the allocator takes no arguments.
        unsafe { alloc_packet() },
        api.codec(b"av_packet_free\0")?,
        "the packet",
    )?;
    let send: Symbol<'_, unsafe extern "C" fn(Pointer, Pointer) -> c_int> =
        api.codec(b"avcodec_send_frame\0")?;
    let receive: Symbol<'_, unsafe extern "C" fn(Pointer, Pointer) -> c_int> =
        api.codec(b"avcodec_receive_packet\0")?;
    let unref: Symbol<'_, unsafe extern "C" fn(Pointer)> = api.codec(b"av_packet_unref\0")?;

    let mut frames = Vec::new();
    let drain = |frames: &mut Vec<Vec<u8>>| -> Result<(), EncodeError> {
        loop {
            // SAFETY: the context is open and the packet is live.
            let result = unsafe { receive(context.pointer, packet.pointer) };
            if result == AGAIN || result == END {
                return Ok(());
            }
            check(result, "avcodec_receive_packet")?;
            // SAFETY: a received packet holds `size` bytes at `data`, at these fixed offsets.
            unsafe {
                let data: *const u8 = field(packet.pointer, PACKET_DATA);
                let size: c_int = field(packet.pointer, PACKET_SIZE);
                frames.push(std::slice::from_raw_parts(data, size as usize).to_vec());
                unref(packet.pointer);
            }
        }
    };
    for block in pcm.samples().chunks(frame_size * channels) {
        // SAFETY: the frame is live.
        check(unsafe { writable(frame.pointer) }, "av_frame_make_writable")?;
        let length = block.len() / channels;
        // SAFETY: `data[channel]` holds `frame_size` floats for each channel, and the frame
        // reports the shortened length of the last block.
        unsafe {
            set_field(frame.pointer, FRAME_NB_SAMPLES, length as c_int);
            for channel in 0..channels {
                let plane: *mut f32 = field(frame.pointer, channel * size_of::<usize>());
                for (index, samples) in block.chunks_exact(channels).enumerate() {
                    plane
                        .add(index)
                        .write(f32::from(samples[channel]) / 32768.0);
                }
            }
        }
        // SAFETY: the context is open and the frame is valid for this configuration.
        check(
            unsafe { send(context.pointer, frame.pointer) },
            "avcodec_send_frame",
        )?;
        drain(&mut frames)?;
    }
    // SAFETY: a null frame asks the encoder to flush.
    check(
        unsafe { send(context.pointer, ptr::null_mut()) },
        "flushing the encoder",
    )?;
    drain(&mut frames)?;

    Ok(AacTrack {
        config,
        frames,
        samples_per_frame: frame_size as u32,
        delay: ENCODER_DELAY,
    })
}

fn read_config(api: &Api<'_>, context: Pointer) -> Result<Vec<u8>, EncodeError> {
    let alloc: Symbol<'_, unsafe extern "C" fn() -> Pointer> =
        api.codec(b"avcodec_parameters_alloc\0")?;
    let from_context: Symbol<'_, unsafe extern "C" fn(Pointer, Pointer) -> c_int> =
        api.codec(b"avcodec_parameters_from_context\0")?;
    let parameters = Owned::new(
        // SAFETY: the allocator takes no arguments.
        unsafe { alloc() },
        api.codec(b"avcodec_parameters_free\0")?,
        "the parameters",
    )?;
    // SAFETY: both pointers are live.
    check(
        unsafe { from_context(parameters.pointer, context) },
        "avcodec_parameters_from_context",
    )?;
    // SAFETY: the offsets of `extradata` and `extradata_size` are those of the known versions.
    let config = unsafe {
        let data: *const u8 = field(parameters.pointer, PARAMETERS_EXTRADATA);
        let size: c_int = field(parameters.pointer, PARAMETERS_EXTRADATA_SIZE);
        if data.is_null() || size <= 0 {
            return Err(failed("the encoder wrote no configuration".to_owned()));
        }
        std::slice::from_raw_parts(data, size as usize).to_vec()
    };
    Ok(config)
}
