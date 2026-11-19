//! The `sprite` form: the region of a texture a `Sprite` shows, and its layout.

use std::fmt::Write as _;

use sekai_codec::{HdrSurface, Surface};
use sekai_unity::Value;

use super::{Texture, texture_memory};
use crate::read::{bytes, count, field, malformed, unsupported};
use crate::roots;
use crate::vertices::{CHANNEL_POSITION, Vertices, channels};
use crate::{
    Asset, Content, Context, ConvertError, Converter, Cost, CostClass, Member, Object, ObjectId,
    ObjectIndex,
};

/// The bit of `settingsRaw` that says a sprite is packed into another texture.
const PACKED: u64 = 1;
/// The bit of `settingsRaw` that says a packed sprite keeps a rectangle of its atlas, rather than
/// a tight outline.
const RECTANGLE_PACKING: u64 = 2;
/// The place and width of the packing rotation of a sprite in `settingsRaw`.
const ROTATION_SHIFT: u32 = 2;
const ROTATION_MASK: u64 = 0xF;
/// The packing rotations that mirror a sprite horizontally and vertically; both together turn it
/// half a turn, and a larger value turns it a quarter turn.
const FLIP_HORIZONTAL: u64 = 1;
const FLIP_VERTICAL: u64 = 2;

/// The image region and layout of a `Sprite`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Sprite;

impl Converter for Sprite {
    fn name(&self) -> &'static str {
        "sprite"
    }

    fn version(&self) -> u32 {
        2
    }

    fn revision(&self) -> u32 {
        1
    }

    fn encoders(&self) -> crate::Encoders {
        crate::Encoders::TEXTURE
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
        let atlas = atlas_of(root, fields, cx)?;
        let placement = Placement::find(root, fields, atlas.as_ref())?;
        if placement.reference("alphaTexture", cx)?.is_some() {
            return Err(unsupported("a sprite with a separate alpha texture"));
        }
        let texture = placement
            .reference("texture", cx)?
            .ok_or_else(|| malformed("the sprite has no texture"))?;
        let image = match Texture.convert(texture, cx)? {
            Some(Asset { members }) => members.into_iter().next().map(|m| m.content),
            None => None,
        };
        let layout = Layout {
            rect: Rect::read(field(fields, "m_Rect")?, "m_Rect")?,
            texture_rect: Rect::read(field(placement.data, "textureRect")?, "textureRect")?,
            texture_rect_offset: vector(
                field(placement.data, "textureRectOffset")?,
                "textureRectOffset",
            )?,
            pivot: vector(field(fields, "m_Pivot")?, "m_Pivot")?,
            pixels_per_unit: float(field(fields, "m_PixelsToUnits")?, "m_PixelsToUnits")?,
            border: border(field(fields, "m_Border")?)?,
        };
        let cut = placement.cut(&layout, field(fields, "m_RD")?)?;
        Ok(Some(Asset {
            members: vec![
                Member {
                    role: "image".into(),
                    content: match image {
                        Some(Content::Image(image)) => {
                            let (w, h, pixels) =
                                cut.image(image.width(), image.height(), image.pixels())?;
                            Content::Image(Surface::new(w, h, pixels)?)
                        }
                        Some(Content::HdrImage(image)) => {
                            let (w, h, pixels) =
                                cut.image(image.width(), image.height(), image.pixels())?;
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

/// The atlas of a sprite.
fn atlas_of(
    root: ObjectId,
    fields: &Value,
    cx: &dyn Context,
) -> Result<Option<Object>, ConvertError> {
    pptr(fields, "m_SpriteAtlas", root, cx)?
        .map(|atlas| cx.object(atlas))
        .transpose()
}

/// The render data that places a sprite in a texture: the entry of its atlas when the atlas lists
/// the sprite, and otherwise its own.
struct Placement<'a> {
    /// The object whose references `data` holds.
    owner: ObjectId,
    data: &'a Value,
    in_atlas: bool,
}

impl<'a> Placement<'a> {
    fn find(
        root: ObjectId,
        fields: &'a Value,
        atlas: Option<&'a Object>,
    ) -> Result<Self, ConvertError> {
        if let Some(atlas) = atlas {
            let key = field(fields, "m_RenderDataKey")?;
            let entries = field(&atlas.fields, "m_RenderDataMap")?
                .as_map()
                .ok_or_else(|| malformed("m_RenderDataMap is not a map"))?;
            if let Some((_, data)) = entries.iter().find(|(k, _)| k == key) {
                return Ok(Self {
                    owner: atlas.id,
                    data,
                    in_atlas: true,
                });
            }
        }
        Ok(Self {
            owner: root,
            data: field(fields, "m_RD")?,
            in_atlas: false,
        })
    }

    fn reference(&self, name: &str, cx: &dyn Context) -> Result<Option<ObjectId>, ConvertError> {
        pptr(self.data, name, self.owner, cx)
    }

    /// How the image of the sprite that `layout` places is cut from the texture. An atlas holds
    /// a sprite whole at `texture_rect`, or trimmed of its transparent margins to `texture_rect`,
    /// which then lies at `texture_rect_offset` in the sprite; it may also mirror or turn it. For
    /// an atlas that packs tight outlines, the mesh of the sprite's own render data `own` says
    /// which pixels are the sprite's.
    fn cut(&self, layout: &Layout, own: &Value) -> Result<Cut, ConvertError> {
        let settings = field(self.data, "settingsRaw")?
            .as_u64()
            .ok_or_else(|| malformed("settingsRaw is not an integer"))?;
        if !self.in_atlas {
            return if settings & PACKED != 0 {
                Err(unsupported("a sprite packed into another texture"))
            } else {
                Ok(Cut::Whole(layout.rect))
            };
        }
        let rotation = settings >> ROTATION_SHIFT & ROTATION_MASK;
        let (rect, texture_rect) = (layout.rect, layout.texture_rect);
        let same_size = (rect.width, rect.height) == (texture_rect.width, texture_rect.height);
        if rotation == 0 && same_size && layout.texture_rect_offset == [0.0, 0.0] {
            return Ok(Cut::Whole(texture_rect));
        }
        if settings & RECTANGLE_PACKING != 0 {
            return if rotation == 0 {
                Ok(Cut::Trimmed {
                    region: texture_rect,
                    offset: layout.texture_rect_offset,
                    sprite: rect,
                })
            } else {
                Err(unsupported("a sprite rotated in a rectangle-packing atlas"))
            };
        }
        if rotation > FLIP_HORIZONTAL | FLIP_VERTICAL {
            return Err(unsupported("a sprite turned a quarter turn in its atlas"));
        }
        Ok(Cut::Outline(Outline {
            triangles: mesh_triangles(own, layout)?,
            size: rounded_size(rect),
            texture_rect,
            offset: layout.texture_rect_offset,
            flip: [
                rotation & FLIP_HORIZONTAL != 0,
                rotation & FLIP_VERTICAL != 0,
            ],
        }))
    }
}

/// How the image of a sprite is cut from its texture.
enum Cut {
    /// The image is the region of the texture.
    Whole(Rect),
    /// A sprite that an atlas packing rectangles trimmed of transparent margins: the region of
    /// the texture, at `offset` from the bottom left of a transparent image of the size of
    /// `sprite`.
    Trimmed {
        region: Rect,
        offset: [f32; 2],
        sprite: Rect,
    },
    /// A sprite of an atlas that packs tight outlines.
    Outline(Outline),
}

impl Cut {
    /// The image of the sprite cut from an RGBA image of `width` x `height` pixels, given as its
    /// channel values with rows from top to bottom.
    fn image<T: Copy + Default>(
        &self,
        width: u32,
        height: u32,
        pixels: &[T],
    ) -> Result<(u32, u32, Vec<T>), ConvertError> {
        match self {
            Cut::Whole(region) => crop(width, height, pixels, region),
            Cut::Trimmed {
                region,
                offset,
                sprite,
            } => place(crop(width, height, pixels, region)?, *offset, sprite),
            Cut::Outline(outline) => outline.draw(width, height, pixels),
        }
    }
}

/// A sprite of an atlas that packs tight outlines. The box around its mesh, `texture_rect`, may
/// also hold pixels of other sprites, so its image holds only the pixels its mesh covers.
struct Outline {
    /// The triangles of the mesh, in pixels from the bottom left of the sprite.
    triangles: Vec<[[f64; 2]; 3]>,
    /// The width and height of the image.
    size: [usize; 2],
    texture_rect: Rect,
    /// Where `texture_rect` lies in the sprite, from its bottom left.
    offset: [f32; 2],
    /// Whether the atlas mirrors the sprite horizontally and vertically within `texture_rect`.
    flip: [bool; 2],
}

impl Outline {
    /// The image of the sprite drawn from an RGBA texture of `width` x `height` pixels, given as
    /// its channel values with rows from top to bottom: transparent black, except each pixel
    /// whose centre lies in or on a triangle of the mesh, which is the texture pixel under that
    /// centre.
    fn draw<T: Copy + Default>(
        &self,
        width: u32,
        height: u32,
        pixels: &[T],
    ) -> Result<(u32, u32, Vec<T>), ConvertError> {
        let [w, h] = self.size;
        let mut out = vec![T::default(); w * h * 4];
        for triangle in &self.triangles {
            for [x, y] in covered(triangle, self.size) {
                let [u, v] = self.texel([x as f64 + 0.5, y as f64 + 0.5]);
                if !(0.0 <= u && u < f64::from(width) && 0.0 <= v && v < f64::from(height)) {
                    return Err(malformed(format!(
                        "the mesh reaches ({u}, {v}), outside the {width} x {height} texture"
                    )));
                }
                let from = ((height as usize - 1 - v as usize) * width as usize + u as usize) * 4;
                let to = ((h - 1 - y) * w + x) * 4;
                out[to..to + 4].copy_from_slice(&pixels[from..from + 4]);
            }
        }
        Ok((w as u32, h as u32, out))
    }

    /// The column and row, from the bottom left, of the texture pixel under the point `point` of
    /// the sprite.
    fn texel(&self, point: [f64; 2]) -> [f64; 2] {
        let r = self.texture_rect;
        let axis = |p: f64, offset: f32, start: f32, size: f32, flip: bool| {
            let within = p - f64::from(offset);
            let within = if flip {
                f64::from(size) - within
            } else {
                within
            };
            (f64::from(start) + within).floor()
        };
        [
            axis(point[0], self.offset[0], r.x, r.width, self.flip[0]),
            axis(point[1], self.offset[1], r.y, r.height, self.flip[1]),
        ]
    }
}

/// The pixels of an image of `size` whose centres lie in or on `triangle`, as columns and rows
/// from the bottom left. A triangle without area covers none.
fn covered(triangle: &[[f64; 2]; 3], size: [usize; 2]) -> impl Iterator<Item = [usize; 2]> + '_ {
    let [a, b, c] = *triangle;
    // Twice the signed area of the triangle `p q r`.
    let area = |p: [f64; 2], q: [f64; 2], r: [f64; 2]| {
        (q[0] - p[0]) * (r[1] - p[1]) - (q[1] - p[1]) * (r[0] - p[0])
    };
    let whole = area(a, b, c);
    // The pixels whose centres lie in the bounding box of the triangle, within the image.
    let span = move |axis: usize| {
        let low = a[axis].min(b[axis]).min(c[axis]) - 0.5;
        let high = a[axis].max(b[axis]).max(c[axis]) - 0.5;
        let start = low.ceil().max(0.0) as usize;
        let end = (high.floor() + 1.0).clamp(0.0, size[axis] as f64) as usize;
        start..end
    };
    let rows = if whole == 0.0 { 0..0 } else { span(1) };
    rows.flat_map(move |y| span(0).map(move |x| [x, y]))
        .filter(move |&[x, y]| {
            let p = [x as f64 + 0.5, y as f64 + 0.5];
            // On the side of each edge that the opposite corner is on, or on the edge.
            [area(a, b, p), area(b, c, p), area(c, a, p)]
                .iter()
                .all(|&part| part * whole >= 0.0)
        })
}

/// The triangles of the mesh of the render data `own`, in pixels from the bottom left of the
/// sprite that `layout` places. A position of the mesh is in units from the pivot.
fn mesh_triangles(own: &Value, layout: &Layout) -> Result<Vec<[[f64; 2]; 3]>, ConvertError> {
    let vertex_data = field(own, "m_VertexData")?;
    let vertices = Vertices::new(
        bytes(vertex_data, "m_DataSize")?,
        count(vertex_data, "m_VertexCount")?,
        channels(vertex_data)?,
    )?;
    let positions = vertices
        .floats::<3>(CHANNEL_POSITION, "positions")?
        .ok_or_else(|| malformed("the sprite's mesh has no positions"))?;
    let pixels_per_unit = f64::from(layout.pixels_per_unit);
    let pivot = [
        f64::from(layout.pivot[0]) * f64::from(layout.rect.width),
        f64::from(layout.pivot[1]) * f64::from(layout.rect.height),
    ];
    let corner = |index: &[u8; 2]| -> Result<[f64; 2], ConvertError> {
        let [x, y, _] = positions
            .get(usize::from(u16::from_le_bytes(*index)))
            .ok_or_else(|| malformed("an index of the sprite's mesh has no vertex"))?;
        Ok([
            f64::from(*x) * pixels_per_unit + pivot[0],
            f64::from(*y) * pixels_per_unit + pivot[1],
        ])
    };
    let (indices, odd_byte) = bytes(own, "m_IndexBuffer")?.as_chunks::<2>();
    let (triangles, rest) = indices.as_chunks::<3>();
    if !odd_byte.is_empty() || !rest.is_empty() {
        return Err(malformed("the sprite's mesh is not a list of triangles"));
    }
    triangles
        .iter()
        .map(|[a, b, c]| Ok([corner(a)?, corner(b)?, corner(c)?]))
        .collect()
}

/// The width and height of `rect`, each rounded to the nearest whole pixel, ties to even.
fn rounded_size(rect: Rect) -> [usize; 2] {
    [rect.width, rect.height].map(|size| f64::from(size).round_ties_even().max(0.0) as usize)
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
            "the rectangle ({x0}, {y0}) to ({x1}, {y1}) is not inside the {w} x {h} texture"
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

/// The RGBA image `image`, rows from top to bottom, in a transparent image of the size of `rect`
/// with its bottom left corner at `offset` from the bottom left, both rounded to whole pixels as
/// [`crop`] rounds.
fn place<T: Copy + Default>(
    (width, height, pixels): (u32, u32, Vec<T>),
    offset: [f32; 2],
    rect: &Rect,
) -> Result<(u32, u32, Vec<T>), ConvertError> {
    let [outer_width, outer_height] = rounded_size(*rect);
    let [x, y] = offset.map(|value| f64::from(value).round_ties_even());
    let (width, height) = (width as usize, height as usize);
    let fits = 0.0 <= x
        && 0.0 <= y
        && x as usize + width <= outer_width
        && y as usize + height <= outer_height;
    if !fits {
        return Err(malformed(format!(
            "the {width} x {height} region at ({x}, {y}) is not inside the {outer_width} x \
             {outer_height} sprite"
        )));
    }
    let (x, y) = (x as usize, y as usize);
    let mut out = vec![T::default(); outer_width * outer_height * 4];
    let top = outer_height - y - height;
    for (row, line) in pixels.chunks(width * 4).enumerate() {
        let start = ((top + row) * outer_width + x) * 4;
        out[start..start + width * 4].copy_from_slice(line);
    }
    Ok((outer_width as u32, outer_height as u32, out))
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
    let atlas = atlas_of(root, &object.fields, cx).ok()?;
    let placement = Placement::find(root, &object.fields, atlas.as_ref()).ok()?;
    let texture = placement.reference("texture", cx).ok()??;
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
    fn a_trimmed_region_is_placed_in_a_transparent_image_of_the_sprite() {
        // A 2 x 1 region 1 pixel from the left and 1 from the bottom of a 4 x 3 sprite.
        let region = (2, 1, vec![1u8, 1, 1, 1, 2, 2, 2, 2]);
        let sprite = rect(0.0, 0.0, 4.0, 3.0);
        let (w, h, out) = place(region.clone(), [1.0, 1.0], &sprite).unwrap();
        assert_eq!((w, h), (4, 3));
        let reds: Vec<u8> = out.chunks(4).map(|p| p[0]).collect();
        assert_eq!(reds, [0, 0, 0, 0, 0, 1, 2, 0, 0, 0, 0, 0]);
        assert!(out.chunks(4).all(|p| p[0] != 0 || p == [0, 0, 0, 0]));
        assert!(place(region, [3.0, 0.0], &sprite).is_err());
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
