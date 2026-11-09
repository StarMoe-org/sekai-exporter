//! OpenEXR output keeps every value exactly and does not change between runs.

use std::io::Cursor;

use exr::prelude::{FlatSamples, ReadChannels, ReadLayers, read};
use sekai_codec::HdrSurface;
use sekai_encode::exr::encode;
use sha2::{Digest, Sha256};

fn surface() -> HdrSurface {
    let (width, height) = (9, 5);
    let pixels = (0..width * height * 4)
        .map(|i| (i * 2311 % 0x7C00) as u16)
        .collect();
    HdrSurface::new(width, height, pixels).unwrap()
}

#[test]
fn every_value_is_kept() {
    let surface = surface();
    let bytes = encode(&surface).unwrap();
    let image = read()
        .no_deep_data()
        .largest_resolution_level()
        .all_channels()
        .first_valid_layer()
        .all_attributes()
        .from_buffered(Cursor::new(&bytes))
        .unwrap();
    let channels = &image.layer_data.channel_data.list;
    assert_eq!(channels.len(), 4);
    for (name, offset) in [("R", 0), ("G", 1), ("B", 2), ("A", 3)] {
        let channel = channels
            .iter()
            .find(|channel| channel.name.to_string() == name)
            .unwrap();
        let FlatSamples::F16(samples) = &channel.sample_data else {
            panic!("channel {name} is not half-float");
        };
        let expected: Vec<u16> = surface.pixels()[offset..]
            .iter()
            .step_by(4)
            .copied()
            .collect();
        let found: Vec<u16> = samples.iter().map(|value| value.to_bits()).collect();
        assert_eq!(found, expected, "channel {name}");
    }
}

#[test]
fn the_output_bytes_are_fixed() {
    let surface = surface();
    let bytes = encode(&surface).unwrap();
    assert_eq!(bytes, encode(&surface).unwrap());
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        digest,
        "d91eebf16800ff41e2ca51330dc50c087f630dfa7665869017ba33ec79bb53ec"
    );
}
