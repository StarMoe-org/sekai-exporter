//! The `sprite` form: the region of a texture a `Sprite` shows, and its layout.

use std::fmt::Write as _;

use sekai_codec::{HdrSurface, Surface};
use sekai_unity::Value;

use super::{Texture, field, malformed, texture_memory};
use crate::roots;
use crate::{
    Asset, Content, Context, ConvertError, Converter, Cost, CostClass, Member, ObjectId,
    ObjectIndex,
};

/// The image region and layout of a `Sprite`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sprite;

impl Converter for Sprite {
    fn name(&self) -> &'static str {
        "sprite"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        roots::sprite(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let fields = &object.fields;
        let rd = field(fields, "m_RD")?;
        check_atlas(root, fields, cx)?;
        let settings = field(rd, "settingsRaw")?
            .as_u64()
            .ok_or_else(|| malformed("m_RD.settingsRaw is not an integer"))?;
        if settings & 1 != 0 {
            return Err(unsupported("a sprite packed into another texture"));
        }
        if pptr(rd, "alphaTexture", root, cx)?.is_some() {
            return Err(unsupported("a sprite with a separate alpha texture"));
        }
        let texture =
            pptr(rd, "texture", root, cx)?.ok_or_else(|| malformed("the sprite has no texture"))?;
        let image = match Texture.convert(texture, cx)? {
            Some(Asset { members }) => members.into_iter().next().map(|m| m.content),
            None => None,
        };
        let texture_rect = Rect::read(field(rd, "textureRect")?, "m_RD.textureRect")?;
        let layout = Layout {
            rect: Rect::read(field(fields, "m_Rect")?, "m_Rect")?,
            texture_rect,
            texture_rect_offset: vector(field(rd, "textureRectOffset")?, "m_RD.textureRectOffset")?,
            pivot: vector(field(fields, "m_Pivot")?, "m_Pivot")?,
            pixels_per_unit: float(field(fields, "m_PixelsToUnits")?, "m_PixelsToUnits")?,
            border: border(field(fields, "m_Border")?)?,
        };
        Ok(Some(Asset {
            members: vec![
                Member {
                    role: "image".into(),
                    content: match image {
                        Some(Content::Image(image)) => {
                            let (w, h, pixels) =
                                crop(image.width(), image.height(), image.pixels(), &texture_rect)?;
                            Content::Image(Surface::new(w, h, pixels)?)
                        }
                        Some(Content::HdrImage(image)) => {
                            let (w, h, pixels) =
                                crop(image.width(), image.height(), image.pixels(), &texture_rect)?;
                            Content::HdrImage(HdrSurface::new(w, h, pixels)?)
                        }
                        _ => return Err(malformed("the sprite's texture holds no image")),
                    },
                },
                Member {
                    role: "sprite".into(),
                    content: Content::File {
                        extension: "json".into(),
                        bytes: layout.to_json().into_bytes(),
                    },
                },
            ],
        }))
    }

    fn cost(&self, root: ObjectId, object: &ObjectIndex, cx: &dyn Context) -> Cost {
        Cost {
            class: CostClass::Heavy,
            memory: sprite_memory(root, cx)
                .unwrap_or_else(|| Cost::light(u64::from(object.size)).memory),
        }
    }
}

/// Fails when the sprite's atlas holds render data for it: the sprite is then drawn from the
/// atlas texture, which this form does not read.
fn check_atlas(root: ObjectId, fields: &Value, cx: &dyn Context) -> Result<(), ConvertError> {
    let Some(atlas) = pptr(fields, "m_SpriteAtlas", root, cx)? else {
        return Ok(());
    };
    let atlas = cx.object(atlas)?;
    let entries = field(&atlas.fields, "m_RenderDataMap")?
        .as_map()
        .ok_or_else(|| malformed("m_RenderDataMap is not a map"))?;
    let key = field(fields, "m_RenderDataKey")?;
    if entries.iter().any(|(k, _)| k == key) {
        return Err(unsupported("a sprite drawn from its atlas texture"));
    }
    Ok(())
}

/// The rounded region `rect` covers in an RGBA image of `width` x `height` pixels, given as its
/// channel values with rows from top to bottom, while `rect` counts from the bottom.
fn crop<T: Copy>(
    width: u32,
    height: u32,
    pixels: &[T],
    rect: &Rect,
) -> Result<(u32, u32, Vec<T>), ConvertError> {
    // Each edge is rounded to the nearest pixel boundary, ties to even, after adding in double
    // precision.
    let edges = |start: f32, size: f32| {
        let start = f64::from(start);
        (
            start.round_ties_even(),
            (start + f64::from(size)).round_ties_even(),
        )
    };
    let (x0, x1) = edges(rect.x, rect.width);
    let (y0, y1) = edges(rect.y, rect.height);
    let (w, h) = (f64::from(width), f64::from(height));
    if !(0.0 <= x0 && x0 < x1 && x1 <= w && 0.0 <= y0 && y0 < y1 && y1 <= h) {
        return Err(malformed(format!(
            "the texture rectangle ({x0}, {y0}) to ({x1}, {y1}) is not inside the {w} x {h} texture"
        )));
    }
    let (x0, x1, y0, y1) = (x0 as usize, x1 as usize, y0 as usize, y1 as usize);
    let stride = width as usize * 4;
    let top = height as usize - y1;
    let out = pixels[top * stride..(top + y1 - y0) * stride]
        .chunks(stride)
        .flat_map(|row| &row[x0 * 4..x1 * 4])
        .copied()
        .collect();
    Ok(((x1 - x0) as u32, (y1 - y0) as u32, out))
}

/// The layout written as the `sprite` member.
struct Layout {
    rect: Rect,
    texture_rect: Rect,
    texture_rect_offset: [f32; 2],
    pivot: [f32; 2],
    pixels_per_unit: f32,
    /// Left, bottom, right, top.
    border: [f32; 4],
}

impl Layout {
    fn to_json(&self) -> String {
        let mut s = String::from("{");
        let _ = write!(s, "\"rect\":{},", self.rect.json());
        let _ = write!(s, "\"texture_rect\":{},", self.texture_rect.json());
        let [x, y] = self.texture_rect_offset;
        let _ = write!(s, "\"texture_rect_offset\":{{\"x\":{x},\"y\":{y}}},");
        let [x, y] = self.pivot;
        let _ = write!(s, "\"pivot\":{{\"x\":{x},\"y\":{y}}},");
        let _ = write!(s, "\"pixels_per_unit\":{},", self.pixels_per_unit);
        let [l, b, r, t] = self.border;
        let _ = write!(
            s,
            "\"border\":{{\"left\":{l},\"bottom\":{b},\"right\":{r},\"top\":{t}}}"
        );
        s.push('}');
        s
    }
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl Rect {
    fn read(value: &Value, name: &str) -> Result<Self, ConvertError> {
        let get = |k: &str| float(field(value, k)?, name);
        Ok(Self {
            x: get("x")?,
            y: get("y")?,
            width: get("width")?,
            height: get("height")?,
        })
    }

    fn json(&self) -> String {
        format!(
            "{{\"x\":{},\"y\":{},\"width\":{},\"height\":{}}}",
            self.x, self.y, self.width, self.height
        )
    }
}

fn vector(value: &Value, name: &str) -> Result<[f32; 2], ConvertError> {
    Ok([
        float(field(value, "x")?, name)?,
        float(field(value, "y")?, name)?,
    ])
}

fn border(value: &Value) -> Result<[f32; 4], ConvertError> {
    let get = |k: &str| float(field(value, k)?, "m_Border");
    Ok([get("x")?, get("y")?, get("z")?, get("w")?])
}

/// A stored `float`, which must be finite to be written as JSON.
fn float(value: &Value, name: &str) -> Result<f32, ConvertError> {
    value
        .as_f64()
        .map(|v| v as f32)
        .filter(|v| v.is_finite())
        .ok_or_else(|| malformed(format!("{name} holds a value that is not a finite number")))
}

/// The estimated peak memory of a sprite: that of decoding and encoding its whole texture, which
/// also covers the cropped image.
fn sprite_memory(root: ObjectId, cx: &dyn Context) -> Option<u64> {
    let object = cx.object(root).ok()?;
    let rd = object.fields.get("m_RD")?;
    let texture = pptr(rd, "texture", root, cx).ok()??;
    texture_memory(texture, cx)
}

/// The object a reference field of `value` points to.
fn pptr(
    value: &Value,
    name: &str,
    from: ObjectId,
    cx: &dyn Context,
) -> Result<Option<ObjectId>, ConvertError> {
    let reference = field(value, name)?
        .as_pptr()
        .ok_or_else(|| malformed(format!("{name} is not a reference")))?;
    cx.resolve(from, reference)
}

fn unsupported(what: &str) -> ConvertError {
    ConvertError::new("unsupported", what)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn the_crop_counts_rows_from_the_bottom_and_rounds_edges_to_even() {
        // 4 x 3, rows from top to bottom; the value of a pixel is 10 * row + column.
        let pixels = (0..3u8)
            .flat_map(|r| (0..4u8).flat_map(move |c| [10 * r + c, 0, 0, 255]))
            .collect();
        let pixels: Vec<u8> = pixels;
        // Edges 0.5 and 2.5 round to 0 and 2, so columns 0 and 1; the bottom two rows are rows
        // 1 and 2 from the top.
        let (w, h, out) = crop(4, 3, &pixels, &rect(0.5, 0.0, 2.0, 1.6)).unwrap();
        assert_eq!((w, h), (2, 2));
        let reds: Vec<u8> = out.chunks(4).map(|p| p[0]).collect();
        assert_eq!(reds, [10, 11, 20, 21]);
        assert!(crop(4, 3, &pixels, &rect(3.0, 0.0, 2.0, 1.0)).is_err());
        assert!(crop(4, 3, &pixels, &rect(1.0, 1.0, 0.2, 1.0)).is_err());
    }

    #[test]
    fn the_layout_is_compact_json_with_shortest_floats() {
        let layout = Layout {
            rect: rect(0.0, 0.0, 2048.0, 894.99994),
            texture_rect: rect(39.076122, 19.042908, 1926.8477, 48.880974),
            texture_rect_offset: [39.076122, 19.042908],
            pivot: [0.5, 0.0],
            pixels_per_unit: 0.60625,
            border: [1.0, 2.0, 3.0, 4.0],
        };
        assert_eq!(
            layout.to_json(),
            "{\"rect\":{\"x\":0,\"y\":0,\"width\":2048,\"height\":894.99994},\
             \"texture_rect\":{\"x\":39.076122,\"y\":19.042908,\"width\":1926.8477,\"height\":48.880974},\
             \"texture_rect_offset\":{\"x\":39.076122,\"y\":19.042908},\
             \"pivot\":{\"x\":0.5,\"y\":0},\"pixels_per_unit\":0.60625,\
             \"border\":{\"left\":1,\"bottom\":2,\"right\":3,\"top\":4}}"
        );
    }
}
