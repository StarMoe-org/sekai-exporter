//! Materials, and the textures and samplers they use.

use std::collections::{BTreeMap, BTreeSet};

use sekai_format::gltf::{
    self, Keywords, MaterialExtras, Named, Pbr, Sampler, TextureInfo, TextureInfoExtensions,
    TextureProperty, TextureTransform, UnityMaterial,
};
use sekai_format::number::Number;
use serde_json::Value as Json;

use super::config::{MaterialKeys, ModelConfig};
use super::read::{
    array, components, count, field, integer, malformed, pairs, reference, string, unsupported,
};
use crate::image::Texture;
use crate::{Content, Context, ConvertError, Converter, Member, ObjectId, Surface, Value};

/// The class ID of `Texture2D`.
const TEXTURE_2D: i32 = 28;
/// The texture properties that hold the colour of a surface, in order of preference.
const BASE_COLOR: [&str; 2] = ["_MainTex", "_BaseMap"];
/// The sampler filters of glTF.
const NEAREST: u32 = 9728;
const LINEAR: u32 = 9729;
const NEAREST_MIPMAP_NEAREST: u32 = 9984;
const LINEAR_MIPMAP_NEAREST: u32 = 9985;
const LINEAR_MIPMAP_LINEAR: u32 = 9987;
/// The wrap modes of glTF.
const REPEAT: u32 = 10497;
const CLAMP_TO_EDGE: u32 = 33071;
const MIRRORED_REPEAT: u32 = 33648;

/// The materials of a file, and the textures and samplers they use.
#[derive(Debug, Default)]
pub(super) struct Materials {
    materials: Vec<gltf::Material>,
    /// The glTF material of each material object.
    by_object: BTreeMap<ObjectId, usize>,
    textures: Vec<Image>,
    /// The glTF texture of each texture object, or `None` when it has no image in the file.
    by_texture: BTreeMap<ObjectId, Option<usize>>,
    samplers: Vec<Sampler>,
    shaders: BTreeMap<ObjectId, String>,
    /// Whether a base colour texture has a transform.
    transformed: bool,
}

/// A texture property of a material.
struct Slot {
    name: String,
    texture: Option<usize>,
    scale: [f32; 2],
    offset: [f32; 2],
}

impl Slot {
    fn property(self) -> (String, TextureProperty) {
        let property = TextureProperty {
            texture: self.texture,
            scale: self.scale.map(Number),
            offset: self.offset.map(Number),
        };
        (self.name, property)
    }
}

/// The transform of a base colour texture with a tiling or an offset.
fn transform(scale: [f32; 2], offset: [f32; 2]) -> Option<TextureInfoExtensions> {
    (scale != [1.0, 1.0] || offset != [0.0, 0.0]).then(|| TextureInfoExtensions {
        transform: TextureTransform {
            // The first row of a Unity image is at v = 1, and of a glTF image at v = 0.
            offset: [offset[0], 1.0 - scale[1] - offset[1]],
            scale,
        },
    })
}

/// The image of a texture, and how it is sampled.
#[derive(Debug)]
struct Image {
    name: String,
    surface: Surface,
    sampler: usize,
}

impl Materials {
    /// The glTF material of the material `id`.
    pub(super) fn material(
        &mut self,
        id: ObjectId,
        config: &ModelConfig<'_>,
        cx: &dyn Context,
    ) -> Result<usize, ConvertError> {
        if let Some(&known) = self.by_object.get(&id) {
            return Ok(known);
        }
        let object = cx.object(id)?;
        let fields = &object.fields;
        let saved = field(fields, "m_SavedProperties")?;
        let mut slots = Vec::new();
        for (name, env) in named(pairs(saved, "m_TexEnvs")?)? {
            let texture = match cx.resolve(id, reference(env, "m_Texture")?)? {
                Some(texture) => self.texture(texture, cx)?,
                None => None,
            };
            slots.push(Slot {
                name,
                texture,
                scale: components(field(env, "m_Scale")?, ["x", "y"])?,
                offset: components(field(env, "m_Offset")?, ["x", "y"])?,
            });
        }
        let base_color_texture = BASE_COLOR.iter().find_map(|base| {
            let slot = slots.iter().find(|slot| slot.name == *base)?;
            Some(TextureInfo {
                index: slot.texture?,
                extensions: transform(slot.scale, slot.offset),
            })
        });
        self.transformed |= base_color_texture
            .as_ref()
            .is_some_and(|info| info.extensions.is_some());
        let unity = UnityMaterial {
            shader: self.shader(id, fields, cx)?,
            render_queue: integer(fields, "m_CustomRenderQueue")?,
            keywords: Keywords {
                valid: strings(fields, "m_ValidKeywords")?,
                invalid: strings(fields, "m_InvalidKeywords")?,
            },
            textures: Named(slots.into_iter().map(Slot::property).collect()),
            floats: properties(saved, "m_Floats", |v| {
                v.as_f64()
                    .map(|v| Number(v as f32))
                    .ok_or_else(|| malformed("a float property is not a number"))
            })?,
            // Engines from before integer properties do not store the list.
            ints: match saved.get("m_Ints") {
                Some(_) => properties(saved, "m_Ints", |v| {
                    v.as_i64()
                        .ok_or_else(|| malformed("an integer property is not an integer"))
                })?,
                None => Named(Vec::new()),
            },
            colors: properties(saved, "m_Colors", |v| {
                Ok(components(v, ["r", "g", "b", "a"])?.map(Number))
            })?,
            further: further_keys(id, config.material_keys, cx)?,
        };
        self.materials.push(gltf::Material {
            name: string(fields, "m_Name")?.to_owned(),
            pbr_metallic_roughness: Pbr {
                base_color_texture,
                metallic_factor: 0.0,
                roughness_factor: 1.0,
            },
            extras: MaterialExtras { unity },
        });
        let index = self.materials.len() - 1;
        self.by_object.insert(id, index);
        Ok(index)
    }

    /// The glTF texture of the texture `id`: `None` when it is not a `Texture2D`, holds no image,
    /// or holds a high dynamic range image.
    fn texture(&mut self, id: ObjectId, cx: &dyn Context) -> Result<Option<usize>, ConvertError> {
        if let Some(&known) = self.by_texture.get(&id) {
            return Ok(known);
        }
        let object = cx.object(id)?;
        let surface = if object.class_id == TEXTURE_2D {
            Texture
                .convert(id, cx)?
                .and_then(|asset| asset.members.into_iter().next())
                .and_then(|member| match member.content {
                    Content::Image(surface) => Some(surface),
                    _ => None,
                })
        } else {
            None
        };
        let index = match surface {
            Some(surface) => {
                let sampler = sampler(&object.fields)?;
                let sampler = match self.samplers.iter().position(|&s| s == sampler) {
                    Some(known) => known,
                    None => {
                        self.samplers.push(sampler);
                        self.samplers.len() - 1
                    }
                };
                self.textures.push(Image {
                    name: string(&object.fields, "m_Name")?.to_owned(),
                    surface,
                    sampler,
                });
                Some(self.textures.len() - 1)
            }
            None => None,
        };
        self.by_texture.insert(id, index);
        Ok(index)
    }

    /// The name of the shader of the material `from`.
    fn shader(
        &mut self,
        from: ObjectId,
        fields: &Value,
        cx: &dyn Context,
    ) -> Result<Option<String>, ConvertError> {
        let Some(id) = cx.resolve(from, reference(fields, "m_Shader")?)? else {
            return Ok(None);
        };
        if let Some(known) = self.shaders.get(&id) {
            return Ok(Some(known.clone()));
        }
        let shader = cx.object(id)?;
        let name = string(field(&shader.fields, "m_ParsedForm")?, "m_Name")?.to_owned();
        self.shaders.insert(id, name.clone());
        Ok(Some(name))
    }

    /// Moves the materials, textures, samplers and images into `document` and returns the images
    /// as members `texture/<n>`, which the file names as the PNG images `texture/<n>.png`.
    pub(super) fn finish(self, document: &mut gltf::Document) -> Vec<Member> {
        document.materials = self.materials;
        document.samplers = self.samplers;
        if self.transformed {
            document.extensions_used.push(gltf::KHR_TEXTURE_TRANSFORM);
        }
        let mut members = Vec::with_capacity(self.textures.len());
        for (index, image) in self.textures.into_iter().enumerate() {
            let role = format!("texture/{index}");
            document.textures.push(gltf::Texture {
                sampler: image.sampler,
                source: index,
            });
            document.images.push(gltf::Image {
                name: image.name,
                uri: format!("{role}.png"),
            });
            members.push(Member {
                role,
                content: Content::Image(image.surface),
            });
        }
        members
    }
}

/// The keys of a material's extras that say what the material stores.
const STORED_KEYS: [&str; 7] = [
    "shader",
    "renderQueue",
    "keywords",
    "textures",
    "floats",
    "ints",
    "colors",
];

/// The keys the form adds to the extras of the material `id`, which may not reuse the name of a
/// key that says what the material stores.
fn further_keys(
    id: ObjectId,
    keys: Option<&dyn MaterialKeys>,
    cx: &dyn Context,
) -> Result<BTreeMap<String, Json>, ConvertError> {
    let Some(keys) = keys else {
        return Ok(BTreeMap::new());
    };
    let keys = keys.keys(id, cx)?;
    match keys.keys().find(|key| STORED_KEYS.contains(&key.as_str())) {
        Some(key) => Err(malformed(format!(
            "a form's material key reuses the name {key}"
        ))),
        None => Ok(keys),
    }
}

/// The sampler of a texture, from its stored filter and wrap modes.
fn sampler(texture: &Value) -> Result<Sampler, ConvertError> {
    let settings = field(texture, "m_TextureSettings")?;
    let mipmapped = count(texture, "m_MipCount")? > 1;
    let (mag_filter, min_filter) = match (integer(settings, "m_FilterMode")?, mipmapped) {
        (0, false) => (NEAREST, NEAREST),
        (0, true) => (NEAREST, NEAREST_MIPMAP_NEAREST),
        (1 | 2, false) => (LINEAR, LINEAR),
        (1, true) => (LINEAR, LINEAR_MIPMAP_NEAREST),
        (2, true) => (LINEAR, LINEAR_MIPMAP_LINEAR),
        (mode, _) => return Err(unsupported(format!("filter mode {mode}"))),
    };
    let wrap = |name| match integer(settings, name)? {
        0 => Ok(REPEAT),
        1 => Ok(CLAMP_TO_EDGE),
        2 => Ok(MIRRORED_REPEAT),
        mode => Err(unsupported(format!("wrap mode {mode}"))),
    };
    Ok(Sampler {
        mag_filter,
        min_filter,
        wrap_s: wrap("m_WrapU")?,
        wrap_t: wrap("m_WrapV")?,
    })
}

/// The saved properties of a material, with each value read by `value`.
fn properties<T>(
    saved: &Value,
    name: &str,
    value: impl Fn(&Value) -> Result<T, ConvertError>,
) -> Result<Named<T>, ConvertError> {
    named(pairs(saved, name)?)?
        .into_iter()
        .map(|(name, stored)| Ok((name, value(stored)?)))
        .collect::<Result<_, _>>()
        .map(Named)
}

/// The pairs of a property map by name. A name stored more than once keeps its first value.
fn named(stored: &[(Value, Value)]) -> Result<Vec<(String, &Value)>, ConvertError> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::with_capacity(stored.len());
    for (key, value) in stored {
        let key = key
            .as_str()
            .ok_or_else(|| malformed("a property name is not a string"))?;
        if seen.insert(key) {
            out.push((key.to_owned(), value));
        }
    }
    Ok(out)
}

fn strings(fields: &Value, name: &str) -> Result<Vec<String>, ConvertError> {
    array(fields, name)?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| malformed(format!("an element of {name} is not a string")))
        })
        .collect()
}
