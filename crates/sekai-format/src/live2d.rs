//! Live2D motion documents, specified in `docs/spec/live2d.md`.

use serde::{Deserialize, Serialize};

use crate::clip::ClipDocument;

/// The value of the `format` key.
pub const FORMAT: &str = "live2d-motion";
/// The version of the document format.
pub const VERSION: u32 = 1;

/// One motion of a Live2D model or motion set: its clip and the fade times the game gives it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionDocument {
    /// Always [`FORMAT`].
    pub format: String,
    /// The version of the document format.
    pub version: u32,
    /// The fade-in time in seconds, when the game lists the motion with metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<f32>,
    /// The fade-out time in seconds, when the game lists the motion with metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<f32>,
    /// The clip.
    pub clip: ClipDocument,
}

impl MotionDocument {
    /// The bytes of the document.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// Reads a document.
    pub fn from_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

/// The CRC-32 of `"Value"`: the attribute of a curve that drives a parameter.
pub const PARAMETER_ATTRIBUTE: u32 = 0xDCB6_7730;
/// The CRC-32 of `"Opacity"`: the attribute of a curve that drives the opacity of a part.
pub const PART_ATTRIBUTE: u32 = 0x8C40_54FA;
