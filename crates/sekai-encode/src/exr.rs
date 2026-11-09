//! OpenEXR, the canonical form of HDR images. Written in pure Rust, so native and WebAssembly
//! builds give the same bytes.

use std::io::Cursor;

use exr::prelude::{
    AnyChannel, AnyChannels, Blocks, Compression, Encoding, FlatSamples, Image, Layer,
    LayerAttributes, LineOrder, WritableImage, f16,
};
use sekai_codec::HdrSurface;

use crate::EncodeError;

/// The encoder name.
pub const NAME: &str = "exr";

/// The encoder version.
pub const VERSION: u32 = 1;

/// Writes `surface` as a scan line OpenEXR with half-float RGBA channels and ZIP compression.
///
/// The values are stored exactly as they are in the surface.
pub fn encode(surface: &HdrSurface) -> Result<Vec<u8>, EncodeError> {
    let plane = |channel: usize| -> Vec<f16> {
        surface.pixels()[channel..]
            .iter()
            .step_by(4)
            .map(|&bits| f16::from_bits(bits))
            .collect()
    };
    let channels = AnyChannels::sort(smallvec::smallvec![
        AnyChannel::new("R", FlatSamples::F16(plane(0))),
        AnyChannel::new("G", FlatSamples::F16(plane(1))),
        AnyChannel::new("B", FlatSamples::F16(plane(2))),
        AnyChannel::new("A", FlatSamples::F16(plane(3))),
    ]);
    let encoding = Encoding {
        compression: Compression::ZIP16,
        blocks: Blocks::ScanLines,
        line_order: LineOrder::Increasing,
    };
    let size = (surface.width() as usize, surface.height() as usize);
    let layer = Layer::new(size, LayerAttributes::default(), encoding, channels);
    let mut out = Vec::new();
    Image::from_layer(layer)
        .write()
        .non_parallel()
        .to_buffered(Cursor::new(&mut out))
        .map_err(|e| EncodeError::Failed {
            encoder: NAME,
            message: e.to_string(),
        })?;
    Ok(out)
}
