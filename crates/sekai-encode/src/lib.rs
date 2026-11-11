//! Encoders that write decoded images and audio in standard formats.
//!
//! The encoders of the canonical forms (PNG, OpenEXR and MP3) are pure Rust and always present.
//! The MP4 encoder runs the `ffmpeg` program. The lossy WebP encoder needs a C library, so it is
//! a feature and is available only in native builds.
//!
//! Every encoder module has the same shape: a [`NAME`](png::NAME) and a
//! [`VERSION`](png::VERSION), which enter the fingerprint of everything it encodes, and an
//! `encode` function from a [`Surface`](sekai_codec::Surface), [`Sound`](sekai_codec::Sound) or
//! [`Pcm`](sekai_codec::Pcm) and the encoder's parameters to bytes. `VERSION` changes only when
//! the output bytes change. Encoding is deterministic: the same input and parameters give the
//! same bytes.

mod error;

pub mod exr;
pub mod mp3;
pub mod mp4;
pub mod png;
#[cfg(feature = "webp")]
pub mod webp;

pub use error::EncodeError;
