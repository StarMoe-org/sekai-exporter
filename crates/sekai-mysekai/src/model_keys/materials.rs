//! The keys a MySekai model adds to the extras of a material: the default texture of each property its
//! shader declares, and the passes of that shader with their render state.

use std::collections::BTreeMap;

use sekai_convert::model::{MaterialKeys, ParsedShader};
use sekai_convert::{Context, ConvertError, ObjectId};
use serde_json::Value as Json;

use crate::scripts::malformed;
use crate::stored::field;

/// The keys of the materials of a MySekai model.
pub(crate) struct ShaderKeys;

impl MaterialKeys for ShaderKeys {
    fn keys(
        &self,
        material: ObjectId,
        cx: &dyn Context,
    ) -> Result<BTreeMap<String, Json>, ConvertError> {
        let fields = &cx.object(material)?.fields;
        let stored = field(fields, "m_Shader")?
            .as_pptr()
            .ok_or_else(|| malformed("m_Shader is not a reference"))?;
        let Some(shader) = cx.resolve(material, stored)? else {
            return Ok(BTreeMap::new());
        };
        let parsed = ParsedShader::read(shader, cx)?;
        Ok(BTreeMap::from([
            (
                "shaderTextureDefaults".to_owned(),
                parsed.texture_defaults_json(),
            ),
            ("shaderPasses".to_owned(), Json::Array(parsed.passes_json())),
        ]))
    }
}
