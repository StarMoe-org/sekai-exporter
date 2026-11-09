//! `TextAsset` contents and fonts.
//!
//! [`Text`] produces the `text` form and [`Font`] the `font` form, specified in
//! `docs/spec/text.md` and `docs/spec/font.md`.

use sekai_unity::Value;

use crate::{Asset, Content, Context, ConvertError, Converter, Member, ObjectId, ObjectIndex};

/// The class ID of `TextAsset`.
const TEXT_ASSET: i32 = 49;
/// The class ID of `Font`.
const FONT: i32 = 128;

/// Container path endings of text assets that hold CRI audio or video, which other forms
/// convert.
const MEDIA_SUFFIXES: &[&str] = &[".acb.bytes", ".usm.bytes"];

/// The bytes a `TextAsset` stores, as they are.
#[derive(Debug, Clone, Copy, Default)]
pub struct Text;

impl Converter for Text {
    fn name(&self) -> &'static str {
        "text"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        _script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        object.class_id == TEXT_ASSET
            && !container_paths.iter().any(|path| {
                let path = path.to_ascii_lowercase();
                MEDIA_SUFFIXES.iter().any(|suffix| path.ends_with(suffix))
            })
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let bytes = field(&object.fields, "m_Script")?
            .as_bytes()
            .ok_or_else(|| malformed("m_Script is not a string"))?
            .to_vec();
        let path = cx
            .container(root)?
            .into_iter()
            .find_map(|(path, id)| (id == root).then_some(path));
        Ok(Some(Asset {
            members: vec![Member {
                role: "file".into(),
                content: Content::File {
                    extension: extension(path.as_deref()),
                    bytes,
                },
            }],
        }))
    }
}

/// The extension of the file a text asset was made from: the last extension of its first
/// container path, in lower case, or `bytes` when it has none.
fn extension(container_path: Option<&str>) -> String {
    let name = container_path.map_or("", |p| p.rsplit('/').next().unwrap_or(p));
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext.to_ascii_lowercase(),
        _ => "bytes".into(),
    }
}

/// The font file a `Font` stores.
#[derive(Debug, Clone, Copy, Default)]
pub struct Font;

impl Converter for Font {
    fn name(&self) -> &'static str {
        "font"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        _script: Option<&str>,
        _container_paths: &[String],
    ) -> bool {
        object.class_id == FONT
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let data = field(&object.fields, "m_FontData")?
            .as_bytes()
            .ok_or_else(|| malformed("m_FontData is not a byte array"))?;
        if data.is_empty() {
            // A font that refers to a font installed on the device.
            return Ok(None);
        }
        let extension = match data.get(..4) {
            Some([0, 1, 0, 0] | b"true") => "ttf",
            Some(b"OTTO") => "otf",
            _ => {
                return Err(ConvertError::new(
                    "unsupported",
                    "the font data is neither TrueType nor OpenType",
                ));
            }
        };
        Ok(Some(Asset {
            members: vec![Member {
                role: "font".into(),
                content: Content::File {
                    extension: extension.into(),
                    bytes: data.to_vec(),
                },
            }],
        }))
    }
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ConvertError> {
    value
        .get(name)
        .ok_or_else(|| malformed(format!("the field {name} is missing")))
}

fn malformed(message: impl Into<String>) -> ConvertError {
    ConvertError::new("malformed", message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_extension_is_the_last_one_of_the_container_path() {
        assert_eq!(extension(Some("assets/a/b/c.atlas.txt")), "txt");
        assert_eq!(extension(Some("assets/a/b/c.JSON")), "json");
        assert_eq!(extension(Some("assets/a.b/c")), "bytes");
        assert_eq!(extension(Some("assets/a/.hidden")), "bytes");
        assert_eq!(extension(None), "bytes");
    }
}
