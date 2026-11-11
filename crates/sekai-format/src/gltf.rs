//! The JSON part of the glTF 2.0 files that model files are, specified in `docs/spec/model.md`.
//!
//! Only what model files use is modelled. Numbers stored as 32-bit values are `f32`, so they are
//! written as the shortest decimal that reads back to them. Extras that a form defines itself are
//! JSON values, which a form that replaces them can write in the shape it defines.

use std::collections::BTreeMap;

use serde::{Serialize, Serializer};
use serde_json::Value as Json;

use crate::number::Number;

/// The `mode` of a primitive drawn as a list of triangles.
pub const TRIANGLES: u32 = 4;
/// The component type of an 8-bit unsigned integer.
pub const UNSIGNED_BYTE: u32 = 5121;
/// The component type of a 16-bit unsigned integer.
pub const UNSIGNED_SHORT: u32 = 5123;
/// The component type of a 32-bit unsigned integer.
pub const UNSIGNED_INT: u32 = 5125;
/// The component type of a 32-bit float.
pub const FLOAT: u32 = 5126;
/// The buffer view target of vertex attributes.
pub const ARRAY_BUFFER: u32 = 34962;
/// The buffer view target of indices.
pub const ELEMENT_ARRAY_BUFFER: u32 = 34963;
/// The extension for texture coordinate transforms.
pub const KHR_TEXTURE_TRANSFORM: &str = "KHR_texture_transform";

/// A glTF document.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    /// The glTF version.
    pub asset: AssetInfo,
    /// The extensions the file uses.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extensions_used: Vec<&'static str>,
    /// The extensions a reader needs to load the file.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub extensions_required: Vec<&'static str>,
    /// The scene shown first.
    pub scene: usize,
    /// The scenes.
    pub scenes: Vec<Scene>,
    /// The nodes.
    pub nodes: Vec<Node>,
    /// The meshes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub meshes: Vec<Mesh>,
    /// The skins.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skins: Vec<Skin>,
    /// The materials.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<Material>,
    /// The textures.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub textures: Vec<Texture>,
    /// The samplers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub samplers: Vec<Sampler>,
    /// The images.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Image>,
    /// The accessors.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub accessors: Vec<Accessor>,
    /// The buffer views.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub buffer_views: Vec<BufferView>,
    /// The buffers: the binary chunk, when there is one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub buffers: Vec<Buffer>,
}

/// The `asset` object.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssetInfo {
    /// `"2.0"`.
    pub version: &'static str,
    /// The program that wrote the file.
    pub generator: &'static str,
}

/// A scene.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scene {
    /// The name, when it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its root nodes.
    pub nodes: Vec<usize>,
}

/// A node.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Node {
    /// The name.
    pub name: String,
    /// The children.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<usize>,
    /// The translation.
    pub translation: [f32; 3],
    /// The rotation, as a quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
    /// The scale.
    pub scale: [f32; 3],
    /// The mesh it draws.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh: Option<usize>,
    /// The skin of its mesh.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skin: Option<usize>,
    /// The weights of the morph targets of its mesh.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub weights: Vec<f32>,
    /// Its state: the keys model files define, such as `active` and `enabled`, and the keys of the
    /// form that writes the file, by key.
    #[serde(skip_serializing_if = "NodeExtras::is_empty")]
    pub extras: NodeExtras,
}

/// The `extras` of a node, written in key order.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(transparent)]
pub struct NodeExtras(pub BTreeMap<String, Json>);

impl NodeExtras {
    /// The keys of a node, written in name order whatever order they are given in.
    pub fn new(keys: impl IntoIterator<Item = (String, Json)>) -> Self {
        Self(keys.into_iter().collect())
    }

    /// Whether no key is written.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The value of the key `key`, when the node writes it.
    pub fn get(&self, key: &str) -> Option<&Json> {
        self.0.get(key)
    }

    /// Writes `value` under `key`, replacing any value the key had.
    pub fn insert(&mut self, key: &str, value: Json) {
        self.0.insert(key.to_owned(), value);
    }
}

/// A mesh.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Mesh {
    /// The name.
    pub name: String,
    /// The primitives.
    pub primitives: Vec<Primitive>,
    /// The names and full weights of the morph targets.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extras: Option<MeshExtras>,
}

/// The `extras` of a mesh with morph targets.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshExtras {
    /// The name of each target.
    pub target_names: Vec<String>,
    /// The weight at which each target applies fully, in the engine's scale.
    pub full_weights: Vec<f32>,
}

/// A primitive.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Primitive {
    /// The accessor of each vertex attribute.
    pub attributes: BTreeMap<String, usize>,
    /// The accessor of the indices.
    pub indices: usize,
    /// The material.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<usize>,
    /// Always [`TRIANGLES`].
    pub mode: u32,
    /// The accessor of each attribute of each morph target.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub targets: Vec<BTreeMap<String, usize>>,
}

/// A skin.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skin {
    /// The joint nodes.
    pub joints: Vec<usize>,
    /// The accessor of the inverse bind matrices.
    pub inverse_bind_matrices: usize,
    /// The root bone.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extras: Option<SkinExtras>,
}

/// The `extras` of a skin.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkinExtras {
    /// The node of the root bone.
    pub root_bone: usize,
}

/// A material.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    /// The name.
    pub name: String,
    /// The preview for programs that do not read `extras`.
    pub pbr_metallic_roughness: Pbr,
    /// The material as stored.
    pub extras: MaterialExtras,
}

/// `pbrMetallicRoughness`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pbr {
    /// The base colour texture.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_color_texture: Option<TextureInfo>,
    /// The metalness.
    pub metallic_factor: f32,
    /// The roughness.
    pub roughness_factor: f32,
}

/// A reference to a texture.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextureInfo {
    /// The texture.
    pub index: usize,
    /// The transform of the texture coordinates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extensions: Option<TextureInfoExtensions>,
}

/// The extensions of a texture reference.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextureInfoExtensions {
    /// The transform of the texture coordinates.
    #[serde(rename = "KHR_texture_transform")]
    pub transform: TextureTransform,
}

/// `KHR_texture_transform`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextureTransform {
    /// The offset.
    pub offset: [f32; 2],
    /// The scale.
    pub scale: [f32; 2],
}

/// The `extras` of a material.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MaterialExtras {
    /// The material as stored, and the keys the form that writes the file adds.
    pub unity: UnityMaterial,
}

/// A material as stored.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UnityMaterial {
    /// The shader's name.
    pub shader: Option<String>,
    /// `m_CustomRenderQueue`.
    pub render_queue: i64,
    /// The keywords.
    pub keywords: Keywords,
    /// Each texture property.
    pub textures: Named<TextureProperty>,
    /// Each float property.
    pub floats: Named<Number>,
    /// Each integer property.
    pub ints: Named<i64>,
    /// Each colour property, as `[r, g, b, a]`.
    pub colors: Named<[Number; 4]>,
    /// Further keys by name, as the form that writes the file defines them.
    #[serde(flatten)]
    pub further: BTreeMap<String, Json>,
}

/// The keywords of a material.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Keywords {
    /// `m_ValidKeywords`.
    pub valid: Vec<String>,
    /// `m_InvalidKeywords`.
    pub invalid: Vec<String>,
}

/// A texture property of a material.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TextureProperty {
    /// The glTF texture.
    pub texture: Option<usize>,
    /// The tiling.
    pub scale: [Number; 2],
    /// The offset.
    pub offset: [Number; 2],
}

/// Values by name, written as a JSON object in this order.
#[derive(Debug, Clone, PartialEq)]
pub struct Named<T>(pub Vec<(String, T)>);

impl<T: Serialize> Serialize for Named<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_map(self.0.iter().map(|(name, value)| (name, value)))
    }
}

/// A texture.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Texture {
    /// The sampler.
    pub sampler: usize,
    /// The image.
    pub source: usize,
}

/// A sampler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sampler {
    /// The magnification filter.
    pub mag_filter: u32,
    /// The minification filter.
    pub min_filter: u32,
    /// The wrap mode of `u`.
    pub wrap_s: u32,
    /// The wrap mode of `v`.
    pub wrap_t: u32,
}

/// An image outside the file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Image {
    /// The name.
    pub name: String,
    /// The path relative to the file.
    pub uri: String,
}

/// An accessor.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Accessor {
    /// The view of its values, absent when they are all zero or sparse.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buffer_view: Option<usize>,
    /// The type of each component.
    pub component_type: u32,
    /// Whether integers are read as fractions of their range.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub normalized: bool,
    /// The number of elements.
    pub count: usize,
    /// `"SCALAR"`, `"VEC2"` and so on.
    #[serde(rename = "type")]
    pub kind: &'static str,
    /// The smallest value of each component.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub min: Vec<f32>,
    /// The largest value of each component.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub max: Vec<f32>,
    /// The elements that differ from zero.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sparse: Option<Sparse>,
}

/// The stored elements of a sparse accessor.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sparse {
    /// The number of stored elements.
    pub count: usize,
    /// Their indices.
    pub indices: SparseIndices,
    /// Their values.
    pub values: SparseValues,
}

/// The indices of a sparse accessor.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SparseIndices {
    /// Their view.
    pub buffer_view: usize,
    /// Their type.
    pub component_type: u32,
}

/// The values of a sparse accessor.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SparseValues {
    /// Their view.
    pub buffer_view: usize,
}

/// A buffer view.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BufferView {
    /// Always 0, the binary chunk.
    pub buffer: usize,
    /// Its start in the buffer.
    pub byte_offset: usize,
    /// Its length.
    pub byte_length: usize,
    /// What it holds, when it holds vertex attributes or indices.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<u32>,
}

/// A buffer.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Buffer {
    /// Its length.
    pub byte_length: usize,
}

impl Document {
    /// The JSON of the document.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_node_without_extras_writes_none() {
        let node = Node {
            name: "Hip".into(),
            children: vec![1],
            translation: [0.0, 1.5, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
            mesh: None,
            skin: None,
            weights: Vec::new(),
            extras: NodeExtras::default(),
        };
        assert_eq!(
            serde_json::to_string(&node).unwrap(),
            r#"{"name":"Hip","children":[1],"translation":[0.0,1.5,0.0],"rotation":[0.0,0.0,0.0,1.0],"scale":[1.0,1.0,1.0]}"#
        );
    }

    #[test]
    fn node_extras_are_written_in_key_order() {
        let extras = NodeExtras::new([
            (
                "sourceObject".to_owned(),
                serde_json::json!({ "gameObjectId": "7" }),
            ),
            ("active".to_owned(), Json::Bool(false)),
            ("fixtureViewRoot".to_owned(), Json::Bool(true)),
        ]);
        assert_eq!(
            serde_json::to_string(&extras).unwrap(),
            r#"{"active":false,"fixtureViewRoot":true,"sourceObject":{"gameObjectId":"7"}}"#
        );
    }

    #[test]
    fn material_properties_keep_their_order_and_name_values_json_cannot_hold() {
        let floats = Named(vec![
            ("_Z".to_owned(), Number(0.5)),
            ("_A".to_owned(), Number(f32::NAN)),
        ]);
        assert_eq!(
            serde_json::to_string(&floats).unwrap(),
            r#"{"_Z":0.5,"_A":"NaN"}"#
        );
    }
}
