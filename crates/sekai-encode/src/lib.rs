//! Encoders that write decoded images and audio in standard formats.
//!
//! The encoders of the canonical forms (PNG, lossless WebP and FLAC) are pure Rust and always present. Encoders that need a C library are features and are available only in native builds.

#[cfg(feature = "aac")]
pub mod aac;
pub mod flac;
#[cfg(feature = "mp3")]
pub mod mp3;
pub mod png;
#[cfg(feature = "webp")]
pub mod webp;
pub mod webp_lossless;
