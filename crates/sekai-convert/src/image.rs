//! Images from `Texture2D`, `Cubemap`, texture arrays, 3D textures and `Sprite`.
//!
//! [`Texture`] produces the `texture` form, specified in `docs/spec/texture.md`, and [`Sprite`]
//! the `sprite` form, specified in `docs/spec/sprite.md`.

mod array;
mod sprite;

pub use sprite::Sprite;

use sekai_codec::texture;
use sekai_unity::Value;

use crate::roots::{self, TEXTURE_2D_ARRAY, TEXTURE_3D};
use crate::{
    Asset, Content, Context, ConvertError, Converter, Cost, CostClass, Member, ObjectId,
    ObjectIndex,
};

/// How many times its decoded size converting and encoding a texture needs: the decoded image
/// and the working copies of the encoder.
const DECODED_FACTOR: u64 = 3;

/// The whole image of a `Texture2D`, the first face of a `Cubemap`, or every slice of a
/// `Texture2DArray` or `Texture3D`: mip level 0, rows from top to bottom.
#[derive(Debug, Clone, Copy, Default)]
pub struct Texture;

impl Converter for Texture {
    fn name(&self) -> &'static str {
        "texture"
    }

    fn version(&self) -> u32 {
        2
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        roots::texture(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let fields = &object.fields;
        if matches!(object.class_id, TEXTURE_3D | TEXTURE_2D_ARRAY) {
            let members = array::slices(root, fields, cx)?;
            return Ok(Some(Asset { members }));
        }
        let width = dimension(fields, "m_Width")?;
        let height = dimension(fields, "m_Height")?;
        let format = field(fields, "m_TextureFormat")?
            .as_i64()
            .and_then(|v| i32::try_from(v).ok())
            .ok_or_else(|| malformed("m_TextureFormat is not a 32-bit integer"))?;
        let inline = field(fields, "image data")?
            .as_bytes()
            .ok_or_else(|| malformed("image data is not a byte array"))?;
        let stream = fields
            .get("m_StreamData")
            .map(StreamData::read)
            .transpose()?
            .filter(|s| !s.path.is_empty());
        if width == 0 || height == 0 {
            // A dynamic font's texture: the engine draws the glyphs into it at run time.
            if inline.is_empty() && stream.is_none() {
                return Ok(None);
            }
            return Err(malformed(format!(
                "a {width} x {height} texture stores image data"
            )));
        }
        let decode = |data: &[u8]| {
            if texture::is_hdr(format) {
                texture::decode_hdr(format, width, height, data).map(Content::HdrImage)
            } else {
                texture::decode(format, width, height, data).map(Content::Image)
            }
        };
        let content = match stream {
            Some(s) => decode(&cx.resource(root, &s.path, s.offset, s.size)?)?,
            None => decode(inline)?,
        };
        Ok(Some(Asset {
            members: vec![Member {
                role: "image".into(),
                content,
            }],
        }))
    }

    fn cost(&self, root: ObjectId, object: &ObjectIndex, cx: &dyn Context) -> Cost {
        Cost {
            class: CostClass::Heavy,
            memory: texture_memory(root, cx)
                .unwrap_or_else(|| Cost::light(u64::from(object.size)).memory),
        }
    }
}

/// The estimated peak memory of converting and encoding texture `id`: its stored image data and
/// [`DECODED_FACTOR`] times the size of its decoded slices.
pub(crate) fn texture_memory(id: ObjectId, cx: &dyn Context) -> Option<u64> {
    let object = cx.object(id).ok()?;
    let fields = &object.fields;
    let width = u64::from(dimension(fields, "m_Width").ok()?);
    let height = u64::from(dimension(fields, "m_Height").ok()?);
    let (depth, pixel, stored) = if matches!(object.class_id, TEXTURE_3D | TEXTURE_2D_ARRAY) {
        let format = fields.get("m_Format")?.as_i64()?;
        (
            u64::from(dimension(fields, "m_Depth").ok()?),
            if format == array::RGBA_HALF { 8 } else { 4 },
            fields.get("m_DataSize")?.as_u64()?,
        )
    } else {
        let format = fields.get("m_TextureFormat")?.as_i64()?;
        let hdr = i32::try_from(format).is_ok_and(texture::is_hdr);
        let stream = fields
            .get("m_StreamData")
            .and_then(|s| StreamData::read(s).ok())
            .filter(|s| !s.path.is_empty());
        let stored = match stream {
            Some(s) => s.size,
            None => fields.get("image data")?.as_bytes()?.len() as u64,
        };
        (1, if hdr { 8 } else { 4 }, stored)
    };
    let decoded = width
        .saturating_mul(height)
        .saturating_mul(depth)
        .saturating_mul(pixel);
    Some(stored.saturating_add(decoded.saturating_mul(DECODED_FACTOR)))
}

/// `m_StreamData`: where a texture's image data is stored outside the object.
struct StreamData {
    offset: u64,
    size: u64,
    path: String,
}

impl StreamData {
    fn read(value: &Value) -> Result<Self, ConvertError> {
        let number = |name: &str| {
            field(value, name)?
                .as_u64()
                .ok_or_else(|| malformed(format!("m_StreamData.{name} is not an unsigned integer")))
        };
        Ok(Self {
            offset: number("offset")?,
            size: number("size")?,
            path: field(value, "path")?
                .as_str()
                .ok_or_else(|| malformed("m_StreamData.path is not a string"))?
                .to_owned(),
        })
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ConvertError> {
    value
        .get(name)
        .ok_or_else(|| malformed(format!("the field {name} is missing")))
}

fn dimension(fields: &Value, name: &str) -> Result<u32, ConvertError> {
    field(fields, name)?
        .as_i64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or_else(|| malformed(format!("{name} is not a size")))
}

fn malformed(message: impl Into<String>) -> ConvertError {
    ConvertError::new("malformed", message)
}
