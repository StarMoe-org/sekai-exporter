//! The `sprite` form and the slices of texture arrays on objects built in memory.

#![cfg(feature = "image")]

use sekai_convert::image::{Sprite, Texture};
use sekai_convert::{
    Asset, Chunk, Content, Context, ConvertError, Converter, Cost, CostClass, Object, ObjectId,
    ObjectIndex, PPtr, Value,
};

/// One file of one bundle, holding the given objects.
struct File(Vec<Object>);

fn id(path_id: i64) -> ObjectId {
    ObjectId {
        bundle: 0,
        file: 0,
        path_id,
    }
}

impl Context for File {
    fn object(&self, id: ObjectId) -> Result<Object, ConvertError> {
        self.0
            .iter()
            .find(|o| o.id == id)
            .cloned()
            .ok_or_else(|| ConvertError::new("missing_object", "no such object"))
    }

    fn resolve(&self, _: ObjectId, reference: PPtr) -> Result<Option<ObjectId>, ConvertError> {
        assert_eq!(reference.file_id, 0);
        Ok((reference.path_id != 0).then(|| id(reference.path_id)))
    }

    fn resource(&self, _: ObjectId, path: &str, _: u64, _: u64) -> Result<Chunk<'_>, ConvertError> {
        Err(ConvertError::new("missing_resource", path))
    }

    fn container(&self, _: ObjectId) -> Result<Vec<(String, ObjectId)>, ConvertError> {
        Ok(Vec::new())
    }
}

fn s(fields: Vec<(&str, Value)>) -> Value {
    Value::Struct(fields.into_iter().map(|(n, v)| (n.into(), v)).collect())
}

fn f(v: f32) -> Value {
    Value::Float(v.into())
}

fn i(v: i64) -> Value {
    Value::Int(v)
}

fn pptr(path_id: i64) -> Value {
    Value::PPtr(PPtr {
        file_id: 0,
        path_id,
    })
}

fn object(path_id: i64, class_id: i32, fields: Value) -> Object {
    Object {
        id: id(path_id),
        class_id,
        class: String::new(),
        script: None,
        fields,
    }
}

fn no_stream() -> Value {
    s(vec![
        ("offset", i(0)),
        ("size", i(0)),
        ("path", Value::String(String::new())),
    ])
}

/// A 4 x 3 RGBA32 texture; stored rows run from the bottom, and the red value of a pixel is
/// `10 * row + column` with row 0 at the top.
fn texture() -> Object {
    let mut data = Vec::new();
    for row in (0..3u8).rev() {
        for column in 0..4u8 {
            data.extend([10 * row + column, 1, 2, 3]);
        }
    }
    object(
        2,
        28,
        s(vec![
            ("m_Width", i(4)),
            ("m_Height", i(3)),
            ("m_TextureFormat", i(4)),
            ("image data", Value::Bytes(data)),
            ("m_StreamData", no_stream()),
        ]),
    )
}

fn rect(x: f32, y: f32, width: f32, height: f32) -> Value {
    s(vec![
        ("x", f(x)),
        ("y", f(y)),
        ("width", f(width)),
        ("height", f(height)),
    ])
}

fn vector(x: f32, y: f32) -> Value {
    s(vec![("x", f(x)), ("y", f(y))])
}

fn key(n: u64) -> Value {
    s(vec![
        ("first", s(vec![("data[0]", Value::UInt(n))])),
        ("second", i(21300000)),
    ])
}

fn sprite(atlas: i64, settings: i64, alpha: i64, sprite_rect: Value) -> Object {
    object(
        1,
        213,
        s(vec![
            ("m_Name", Value::String("icon".into())),
            ("m_Rect", sprite_rect),
            ("m_Pivot", vector(0.5, 0.0)),
            (
                "m_Border",
                s(vec![
                    ("x", f(1.0)),
                    ("y", f(0.0)),
                    ("z", f(1.0)),
                    ("w", f(0.25)),
                ]),
            ),
            ("m_PixelsToUnits", f(100.0)),
            ("m_RenderDataKey", key(7)),
            ("m_SpriteAtlas", pptr(atlas)),
            (
                "m_RD",
                s(vec![
                    ("texture", pptr(2)),
                    ("alphaTexture", pptr(alpha)),
                    ("textureRect", rect(2.0, 1.0, 1.0, 1.0)),
                    ("textureRectOffset", vector(1.0, 0.0)),
                    ("settingsRaw", i(settings)),
                ]),
            ),
        ]),
    )
}

fn atlas(keys: &[u64]) -> Object {
    let entries = keys.iter().map(|&k| (key(k), s(vec![]))).collect();
    object(
        3,
        687078895,
        s(vec![("m_RenderDataMap", Value::Map(entries))]),
    )
}

/// An atlas that holds the sprite with the key 7 at `texture_rect` of the texture.
fn atlas_holding(texture_rect: Value, offset: Value, settings: i64) -> Object {
    let data = s(vec![
        ("texture", pptr(2)),
        ("alphaTexture", pptr(0)),
        ("textureRect", texture_rect),
        ("textureRectOffset", offset),
        ("settingsRaw", i(settings)),
    ]);
    object(
        3,
        687078895,
        s(vec![("m_RenderDataMap", Value::Map(vec![(key(7), data)]))]),
    )
}

fn members(asset: Asset) -> Vec<(String, Content)> {
    asset
        .members
        .into_iter()
        .map(|m| (m.role, m.content))
        .collect()
}

#[test]
fn a_sprite_is_its_rounded_rectangle_and_its_layout() {
    // x 1..3.4 rounds to columns 1 and 2; y 0.5..2.5 rounds to 0..2, the bottom two rows.
    let file = File(vec![
        sprite(3, 64, 0, rect(1.0, 0.5, 2.4, 2.0)),
        texture(),
        atlas(&[8]),
    ]);
    let got = members(Sprite.convert(id(1), &file).unwrap().unwrap());
    let [
        (image_role, Content::Image(image)),
        (layout_role, Content::File { extension, bytes }),
    ] = &got[..]
    else {
        panic!("unexpected members {got:?}");
    };
    assert_eq!(
        (image_role.as_str(), layout_role.as_str()),
        ("image", "sprite")
    );
    assert_eq!((image.width(), image.height()), (2, 2));
    let reds: Vec<u8> = image.pixels().chunks(4).map(|p| p[0]).collect();
    assert_eq!(reds, [11, 12, 21, 22]);
    assert_eq!(extension, "json");
    assert_eq!(
        std::str::from_utf8(bytes).unwrap(),
        "{\"rect\":{\"x\":1,\"y\":0.5,\"width\":2.4,\"height\":2},\
         \"texture_rect\":{\"x\":2,\"y\":1,\"width\":1,\"height\":1},\
         \"texture_rect_offset\":{\"x\":1,\"y\":0},\"pivot\":{\"x\":0.5,\"y\":0},\
         \"pixels_per_unit\":100,\"border\":{\"left\":1,\"bottom\":0,\"right\":1,\"top\":0.25}}"
    );
}

#[test]
fn a_sprite_its_atlas_holds_whole_is_cut_from_the_atlas_texture() {
    let file = File(vec![
        sprite(3, 0, 0, rect(0.0, 0.0, 2.0, 2.0)),
        texture(),
        atlas_holding(rect(1.0, 0.0, 2.0, 2.0), vector(0.0, 0.0), 3),
    ]);
    let got = members(Sprite.convert(id(1), &file).unwrap().unwrap());
    let Content::Image(image) = &got[0].1 else {
        panic!("unexpected members {got:?}");
    };
    assert_eq!((image.width(), image.height()), (2, 2));
    let reds: Vec<u8> = image.pixels().chunks(4).map(|p| p[0]).collect();
    assert_eq!(reds, [11, 12, 21, 22]);
}

#[test]
fn sprites_drawn_from_elsewhere_are_unsupported() {
    let whole = || rect(0.0, 0.0, 4.0, 3.0);
    let cases = [
        File(vec![
            sprite(3, 0, 0, rect(0.0, 0.0, 2.0, 2.0)),
            texture(),
            atlas_holding(rect(1.0, 0.0, 1.5, 2.0), vector(0.0, 0.0), 3),
        ]),
        File(vec![
            sprite(3, 0, 0, rect(0.0, 0.0, 2.0, 2.0)),
            texture(),
            atlas_holding(rect(1.0, 0.0, 2.0, 2.0), vector(1.0, 0.0), 3),
        ]),
        File(vec![
            sprite(3, 0, 0, rect(0.0, 0.0, 2.0, 2.0)),
            texture(),
            atlas_holding(rect(1.0, 0.0, 2.0, 2.0), vector(0.0, 0.0), 11),
        ]),
        File(vec![sprite(0, 1, 0, whole()), texture()]),
        File(vec![sprite(0, 0, 2, whole()), texture()]),
    ];
    for file in cases {
        assert_eq!(
            Sprite.convert(id(1), &file).unwrap_err().code,
            "unsupported"
        );
    }
}

#[test]
fn a_rectangle_outside_the_texture_is_malformed() {
    let file = File(vec![sprite(0, 0, 0, rect(2.0, 0.0, 3.0, 1.0)), texture()]);
    assert_eq!(Sprite.convert(id(1), &file).unwrap_err().code, "malformed");
}

fn array(class_id: i32, format: i64, width: i64, height: i64, depth: i64, data: Vec<u8>) -> Object {
    object(
        5,
        class_id,
        s(vec![
            ("m_Format", i(format)),
            ("m_Width", i(width)),
            ("m_Height", i(height)),
            ("m_Depth", i(depth)),
            ("m_DataSize", i(data.len() as i64)),
            ("image data", Value::Bytes(data)),
            ("m_StreamData", no_stream()),
        ]),
    )
}

#[test]
fn every_slice_of_a_half_float_3d_texture_is_a_member() {
    // 1 x 2 x 2: slice z, row y (from the bottom) holds the value z * 2 + y in every channel.
    let mut data = Vec::new();
    for value in 0..4u16 {
        data.extend([value; 4].iter().flat_map(|v| v.to_le_bytes()));
    }
    let file = File(vec![array(117, 48, 1, 2, 2, data)]);
    let got = members(Texture.convert(id(5), &file).unwrap().unwrap());
    let slices: Vec<(&str, Vec<u16>)> = got
        .iter()
        .map(|(role, content)| match content {
            Content::HdrImage(image) => (role.as_str(), image.pixels().to_vec()),
            other => panic!("not HDR: {other:?}"),
        })
        .collect();
    assert_eq!(
        slices,
        [
            ("slice/0", [[1u16; 4], [0; 4]].concat()),
            ("slice/1", [[3u16; 4], [2; 4]].concat()),
        ]
    );
}

#[test]
fn an_astc_texture_array_decodes_each_slice() {
    // One 6 x 6 block per slice: a void-extent block of one LDR colour.
    let block = |r: u16| {
        let mut b = vec![0xFC, 0xFD, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        for c in [r, 0, 0, 0xFFFF] {
            b.extend(c.to_le_bytes());
        }
        b
    };
    let data = [block(0x1000), block(0x8000)].concat();
    let file = File(vec![array(187, 134, 6, 6, 2, data)]);
    let got = members(Texture.convert(id(5), &file).unwrap().unwrap());
    let firsts: Vec<(&str, &[u8])> = got
        .iter()
        .map(|(role, content)| match content {
            Content::Image(image) => (role.as_str(), &image.pixels()[..4]),
            other => panic!("not an image: {other:?}"),
        })
        .collect();
    assert_eq!(
        firsts,
        [
            ("slice/0", &[0x10, 0, 0, 255][..]),
            ("slice/1", &[0x80, 0, 0, 255][..])
        ]
    );
}

#[test]
fn texture_arrays_of_other_formats_or_sizes_are_errors() {
    let file = File(vec![array(187, 8, 1, 1, 1, vec![0; 4])]);
    assert_eq!(
        Texture.convert(id(5), &file).unwrap_err().code,
        "unsupported"
    );
    let file = File(vec![array(117, 48, 1, 1, 3, vec![0; 16])]);
    assert_eq!(Texture.convert(id(5), &file).unwrap_err().code, "malformed");
}

fn index(path_id: i64, class_id: i32, size: u32) -> ObjectIndex {
    ObjectIndex {
        path_id,
        class_id,
        class: String::new(),
        name: None,
        script: None,
        script_name: None,
        references: Vec::new(),
        size,
        sha256: [0; 32],
        error: None,
    }
}

#[test]
fn texture_costs_come_from_the_header() {
    let file = File(vec![
        sprite(0, 0, 0, rect(0.0, 0.0, 4.0, 3.0)),
        texture(),
        array(187, 48, 2, 2, 3, vec![0; 96]),
    ]);
    // 48 stored bytes, and three times the 48 bytes of the decoded 4 x 3 image.
    let heavy = |memory| Cost {
        class: CostClass::Heavy,
        memory,
    };
    assert_eq!(
        Texture.cost(id(2), &index(2, 28, 1000), &file),
        heavy(48 + 3 * 48)
    );
    assert_eq!(
        Sprite.cost(id(1), &index(1, 213, 500), &file),
        heavy(48 + 3 * 48)
    );
    // Half-float slices take eight bytes a texel.
    assert_eq!(
        Texture.cost(id(5), &index(5, 187, 10), &file),
        heavy(96 + 3 * 2 * 2 * 3 * 8)
    );
    // An object that cannot be read is estimated from its stored size.
    assert_eq!(
        Texture.cost(id(9), &index(9, 28, 100), &file),
        heavy(4 * 100)
    );
}
