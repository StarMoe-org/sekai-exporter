//! The converter interface, the writers and the general-purpose converters.
//!
//! A converter is a pure function from a root object, its closure and the table rows it reads to an asset. It does not read files, choose output paths or write to the catalog.

#[cfg(feature = "animation")]
pub mod animation;
#[cfg(feature = "audio")]
pub mod audio;
#[cfg(feature = "image")]
pub mod image;
#[cfg(feature = "live2d")]
pub mod live2d;
#[cfg(feature = "model")]
pub mod model;
#[cfg(feature = "particles")]
pub mod particles;
#[cfg(feature = "text")]
pub mod text;
#[cfg(feature = "timeline")]
pub mod timeline;
#[cfg(feature = "ugui")]
pub mod ugui;
#[cfg(feature = "video")]
pub mod video;
