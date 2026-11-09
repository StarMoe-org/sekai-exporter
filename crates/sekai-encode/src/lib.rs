//! Encoders that write decoded images and audio in standard formats.
//!
//! The encoders of the canonical forms (PNG, lossless WebP, OpenEXR and FLAC) are pure Rust and always
//! present. The MP4 encoder runs the `ffmpeg` program. Encoders that need a C library are features and are available only in native builds.
//!
//! Every encoder module has the same shape: a [`NAME`](png::NAME) and a
//! [`VERSION`](png::VERSION), which enter the fingerprint of everything it encodes, and an
//! `encode` function from a [`Surface`](sekai_codec::Surface) or [`Pcm`](sekai_codec::Pcm) and
//! the encoder's parameters to bytes. `VERSION` changes only when the output bytes change.
//! Encoding is deterministic: the same input and parameters give the same bytes.

mod error;

#[cfg(feature = "aac")]
pub mod aac;
pub mod exr;
pub mod flac;
#[cfg(feature = "mp3")]
pub mod mp3;
pub mod mp4;
pub mod png;
#[cfg(feature = "webp")]
pub mod webp;
pub mod webp_lossless;

pub use error::EncodeError;
