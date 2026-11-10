//! Shaders: the properties a shader declares and the state each of its passes draws in.
//!
//! A shader stores its parsed form in `m_ParsedForm` as a `SerializedShader`: its declared
//! properties in `m_PropInfo`, and its subshaders, each with its tags and its passes. The state of
//! a pass (`m_State`) holds the pass's name, its tags and its render state.

use sekai_format::number::Number;
use serde_json::{Map, Value as Json, json};

use super::read::{array, field, integer, malformed, number, pairs, string};
use crate::{Context, ConvertError, ObjectId, Value};

/// The class ID of `Shader`.
const SHADER: i32 = 48;
/// The `m_Type` of a texture property.
const TEXTURE_PROPERTY: i64 = 4;
/// The name a render state value stores when it binds no property.
const UNBOUND: &str = "<noninit>";

/// The parsed form of a shader.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedShader {
    /// `m_ParsedForm.m_Name`.
    pub name: String,
    /// The properties it declares, in declared order.
    pub properties: Vec<ShaderProperty>,
    /// Its subshaders, in stored order.
    pub sub_shaders: Vec<SubShader>,
}

/// A property a shader declares.
#[derive(Debug, Clone, PartialEq)]
pub struct ShaderProperty {
    /// `m_Name`.
    pub name: String,
    /// The first component of `m_DefValue`, the value of a number property.
    pub default: f32,
    /// For a texture property, the name of its default texture (`m_DefTexture.m_DefaultName`).
    pub default_texture: Option<String>,
}

/// A subshader: its tags and its passes.
#[derive(Debug, Clone, PartialEq)]
pub struct SubShader {
    /// `m_Tags`.
    pub tags: Vec<(String, String)>,
    /// `m_Passes`, as their states store them.
    pub passes: Vec<Pass>,
}

/// A pass, as its state stores it.
#[derive(Debug, Clone, PartialEq)]
pub struct Pass {
    /// `m_State.m_Name`.
    pub name: String,
    /// `m_State.m_Tags`.
    pub tags: Vec<(String, String)>,
    /// The faces it culls.
    pub culling: StateValue,
    /// Its depth test.
    pub z_test: StateValue,
    /// Whether it writes depth.
    pub z_write: StateValue,
    /// The blend state of render target 0 (`rtBlend0`).
    pub blend: Blend,
}

/// The blend state of a render target.
#[derive(Debug, Clone, PartialEq)]
pub struct Blend {
    /// The colour channels it writes.
    pub col_mask: StateValue,
    /// The source factor of the colour.
    pub src_blend: StateValue,
    /// The destination factor of the colour.
    pub dest_blend: StateValue,
    /// The source factor of the alpha.
    pub src_blend_alpha: StateValue,
    /// The destination factor of the alpha.
    pub dest_blend_alpha: StateValue,
    /// The operation that blends the colour.
    pub blend_op: StateValue,
    /// The operation that blends the alpha.
    pub blend_op_alpha: StateValue,
}

/// A value of a render state (`SerializedShaderFloatValue`).
#[derive(Debug, Clone, PartialEq)]
pub enum StateValue {
    /// A value compiled into the pass, which no material property changes.
    Fixed(f32),
    /// The material property `name`, or the `default` the shader declares for it when the
    /// material does not store it. `compiled` is the value compiled into the pass.
    Property {
        /// The property.
        name: String,
        /// The default the shader declares for it, or `None` when it does not declare it.
        default: Option<f32>,
        /// The value compiled into the pass.
        compiled: f32,
    },
}

impl ParsedShader {
    /// Reads the parsed form of the shader `id`.
    pub fn read(id: ObjectId, cx: &dyn Context) -> Result<Self, ConvertError> {
        let shader = cx.object(id)?;
        if shader.class_id != SHADER {
            return Err(malformed(format!("object {} is not a Shader", id.path_id)));
        }
        let form = field(&shader.fields, "m_ParsedForm")?;
        let properties = array(field(form, "m_PropInfo")?, "m_Props")?
            .iter()
            .map(ShaderProperty::read)
            .collect::<Result<Vec<_>, _>>()?;
        let sub_shaders = array(form, "m_SubShaders")?
            .iter()
            .map(|sub_shader| SubShader::read(sub_shader, &properties))
            .collect::<Result<_, _>>()?;
        Ok(Self {
            name: string(form, "m_Name")?.to_owned(),
            properties,
            sub_shaders,
        })
    }

    /// The name of the default texture of each texture property, by property name.
    pub fn texture_defaults_json(&self) -> Json {
        let defaults = self.properties.iter().filter_map(|property| {
            let texture = property.default_texture.as_ref()?;
            Some((property.name.clone(), Json::from(texture.as_str())))
        });
        Json::Object(defaults.collect())
    }

    /// One row for each pass, in subshader and pass order: `name`, `lightMode` (the pass's
    /// `LightMode` tag, or `null`), `subShaderIndex`, `passIndex`, `subShaderTags`, `tags` and
    /// `renderState`.
    pub fn passes_json(&self) -> Vec<Json> {
        let mut rows = Vec::new();
        for (sub_shader_index, sub_shader) in self.sub_shaders.iter().enumerate() {
            for (pass_index, pass) in sub_shader.passes.iter().enumerate() {
                rows.push(json!({
                    "name": pass.name,
                    "lightMode": pass.light_mode(),
                    "subShaderIndex": sub_shader_index,
                    "passIndex": pass_index,
                    "subShaderTags": tags_json(&sub_shader.tags),
                    "tags": tags_json(&pass.tags),
                    "renderState": pass.render_state_json(),
                }));
            }
        }
        rows
    }
}

impl ShaderProperty {
    fn read(property: &Value) -> Result<Self, ConvertError> {
        let default_texture = match integer(property, "m_Type")? {
            TEXTURE_PROPERTY => {
                let texture = field(property, "m_DefTexture")?;
                Some(string(texture, "m_DefaultName")?.to_owned())
            }
            _ => None,
        };
        Ok(Self {
            name: string(property, "m_Name")?.to_owned(),
            default: number(property, "m_DefValue[0]")?,
            default_texture,
        })
    }
}

impl SubShader {
    fn read(sub_shader: &Value, properties: &[ShaderProperty]) -> Result<Self, ConvertError> {
        Ok(Self {
            tags: tags(sub_shader)?,
            passes: array(sub_shader, "m_Passes")?
                .iter()
                .map(|pass| Pass::read(field(pass, "m_State")?, properties))
                .collect::<Result<_, _>>()?,
        })
    }
}

impl Pass {
    /// Reads a pass from its state (`SerializedShaderState`).
    fn read(state: &Value, properties: &[ShaderProperty]) -> Result<Self, ConvertError> {
        let value = |name| StateValue::read(field(state, name)?, properties);
        Ok(Self {
            name: string(state, "m_Name")?.to_owned(),
            tags: tags(state)?,
            culling: value("culling")?,
            z_test: value("zTest")?,
            z_write: value("zWrite")?,
            blend: Blend::read(field(state, "rtBlend0")?, properties)?,
        })
    }

    /// The `LightMode` tag, whose name, like every tag name, ignores case.
    fn light_mode(&self) -> Option<&str> {
        self.tags
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("LightMode"))
            .map(|(_, value)| value.as_str())
    }

    fn render_state_json(&self) -> Json {
        json!({
            "culling": self.culling.json(),
            "zTest": self.z_test.json(),
            "zWrite": self.z_write.json(),
            "blend": self.blend.json(),
        })
    }
}

impl Blend {
    fn read(blend: &Value, properties: &[ShaderProperty]) -> Result<Self, ConvertError> {
        let value = |name| StateValue::read(field(blend, name)?, properties);
        Ok(Self {
            col_mask: value("colMask")?,
            src_blend: value("srcBlend")?,
            dest_blend: value("destBlend")?,
            src_blend_alpha: value("srcBlendAlpha")?,
            dest_blend_alpha: value("destBlendAlpha")?,
            blend_op: value("blendOp")?,
            blend_op_alpha: value("blendOpAlpha")?,
        })
    }

    fn json(&self) -> Json {
        json!({
            "colMask": self.col_mask.json(),
            "srcBlend": self.src_blend.json(),
            "destBlend": self.dest_blend.json(),
            "srcBlendAlpha": self.src_blend_alpha.json(),
            "destBlendAlpha": self.dest_blend_alpha.json(),
            "blendOp": self.blend_op.json(),
            "blendOpAlpha": self.blend_op_alpha.json(),
        })
    }
}

impl StateValue {
    /// Reads a value that binds the property its `name` stores, or no property when the name is
    /// `<noninit>`.
    fn read(value: &Value, properties: &[ShaderProperty]) -> Result<Self, ConvertError> {
        let compiled = number(value, "val")?;
        let name = string(value, "name")?;
        if name == UNBOUND {
            return Ok(Self::Fixed(compiled));
        }
        let declared = properties.iter().find(|property| property.name == name);
        Ok(Self::Property {
            name: name.to_owned(),
            default: declared.map(|property| property.default),
            compiled,
        })
    }

    /// `{ "name", "default", "val" }` for a property, without `default` when the shader does not
    /// declare the property; `{ "name": "<noninit>", "val" }` for a fixed value.
    fn json(&self) -> Json {
        match self {
            Self::Fixed(compiled) => json!({ "name": UNBOUND, "val": Number(*compiled) }),
            Self::Property {
                name,
                default: Some(default),
                compiled,
            } => json!({ "name": name, "default": Number(*default), "val": Number(*compiled) }),
            Self::Property {
                name,
                default: None,
                compiled,
            } => json!({ "name": name, "val": Number(*compiled) }),
        }
    }
}

/// The tags of a `SerializedTagMap` stored in `m_Tags`, in stored order.
fn tags(holder: &Value) -> Result<Vec<(String, String)>, ConvertError> {
    pairs(field(holder, "m_Tags")?, "tags")?
        .iter()
        .map(|(name, value)| match (name.as_str(), value.as_str()) {
            (Some(name), Some(value)) => Ok((name.to_owned(), value.to_owned())),
            _ => Err(malformed("a tag is not a pair of strings")),
        })
        .collect()
}

fn tags_json(tags: &[(String, String)]) -> Json {
    let tags = tags
        .iter()
        .map(|(name, value)| (name.clone(), Json::from(value.as_str())));
    Json::Object(tags.collect::<Map<_, _>>())
}
