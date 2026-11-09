//! The slices of a `Texture2DArray` or a `Texture3D`.

use sekai_codec::{HdrSurface, texture};
use sekai_unity::Value;

use super::{StreamData, dimension, field, malformed};
use crate::{Content, Context, ConvertError, Member, ObjectId};

/// `GraphicsFormat.R16G16B16A16_SFloat`.
const RGBA_HALF: i64 = 48;
/// `GraphicsFormat.RGBA_ASTC6X6_UNorm`.
const ASTC_6X6: i64 = 134;
/// `TextureFormat.ASTC_6x6`, which stores the same blocks.
const TEXTURE_ASTC_6X6: i32 = 50;

/// Mip level 0 of every slice, as the members `slice/0`, `slice/1` and so on.
pub(super) fn slices(
    root: ObjectId,
    fields: &Value,
    cx: &dyn Context,
) -> Result<Vec<Member>, ConvertError> {
    let width = dimension(fields, "m_Width")?;
    let height = dimension(fields, "m_Height")?;
    let depth = dimension(fields, "m_Depth")?;
    let format = field(fields, "m_Format")?
        .as_i64()
        .ok_or_else(|| malformed("m_Format is not an integer"))?;
    let size = field(fields, "m_DataSize")?
        .as_u64()
        .ok_or_else(|| malformed("m_DataSize is not a size"))?;
    let inline = field(fields, "image data")?
        .as_bytes()
        .ok_or_else(|| malformed("image data is not a byte array"))?;
    let stream = fields
        .get("m_StreamData")
        .map(StreamData::read)
        .transpose()?
        .filter(|s| !s.path.is_empty());
    let chunk;
    let data: &[u8] = match stream {
        Some(s) => {
            chunk = cx.resource(root, &s.path, s.offset, s.size)?;
            &chunk
        }
        None => inline,
    };
    if depth == 0 || size % u64::from(depth) != 0 || data.len() as u64 != size {
        return Err(malformed(format!(
            "{} bytes of image data, {size} declared, for {depth} slices",
            data.len()
        )));
    }
    let slice = (size / u64::from(depth)) as usize;
    data.chunks(slice)
        .enumerate()
        .map(|(i, bytes)| {
            let content = match format {
                ASTC_6X6 => {
                    Content::Image(texture::decode(TEXTURE_ASTC_6X6, width, height, bytes)?)
                }
                RGBA_HALF => Content::HdrImage(half(width, height, bytes)?),
                _ => {
                    return Err(ConvertError::new(
                        "unsupported",
                        format!("graphics format {format}"),
                    ));
                }
            };
            Ok(Member {
                role: format!("slice/{i}"),
                content,
            })
        })
        .collect()
}

/// RGBA binary16 texels, rows from bottom to top, as an HDR surface.
fn half(width: u32, height: u32, bytes: &[u8]) -> Result<HdrSurface, ConvertError> {
    let row = width as usize * 8;
    let needed = row * height as usize;
    if bytes.len() < needed {
        return Err(ConvertError::new(
            "truncated",
            format!(
                "a {width} x {height} RGBA half slice needs {needed} bytes, {} given",
                bytes.len()
            ),
        ));
    }
    let pixels = bytes[..needed]
        .chunks(row)
        .rev()
        .flat_map(|r| r.chunks(2).map(|v| u16::from_le_bytes([v[0], v[1]])))
        .collect();
    Ok(HdrSurface::new(width, height, pixels)?)
}
