//! Images from `Texture2D`, `Sprite` and sprite atlases.
//!
//! [`Texture`] produces the `texture` form, specified in `docs/spec/texture.md`.

use sekai_codec::texture;
use sekai_unity::Value;

use crate::{Asset, Content, Context, ConvertError, Converter, Member, ObjectId, ObjectIndex};

/// The class ID of `Texture2D`.
const TEXTURE_2D: i32 = 28;
/// The class ID of `Cubemap`.
const CUBEMAP: i32 = 89;

/// The whole image of a `Texture2D`, or the first face of a `Cubemap`: mip level 0 of the first
/// slice, rows from top to bottom.
#[derive(Debug, Clone, Copy, Default)]
pub struct Texture;

impl Converter for Texture {
    fn name(&self) -> &'static str {
        "texture"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(&self, object: &ObjectIndex, _container_paths: &[String]) -> bool {
        matches!(object.class_id, TEXTURE_2D | CUBEMAP)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let fields = &object.fields;
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
        let surface = match stream {
            Some(s) => {
                let bytes = cx.resource(root, &s.path, s.offset, s.size)?;
                texture::decode(format, width, height, &bytes)?
            }
            None => texture::decode(format, width, height, inline)?,
        };
        Ok(Some(Asset {
            members: vec![Member {
                role: "image".into(),
                content: Content::Image(surface),
            }],
        }))
    }
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
