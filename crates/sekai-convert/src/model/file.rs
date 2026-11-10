//! Model files: a prefab as a GLB file and its images.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};

use sekai_format::gltf::{
    self, ARRAY_BUFFER, AssetInfo, Document, MeshExtras, NodeExtras, Primitive, Scene, SkinExtras,
    TRIANGLES,
};

use serde_json::Value as Json;

use super::config::ModelConfig;
use super::glb::{self, Buffers};
use super::material::Materials;
use super::mesh::{self, Colors, Mesh};
use super::prefab::{Node, Prefab};
use super::read::{array, flag, malformed, reference, unsupported};
use crate::{Context, ConvertError, Member, Object, ObjectId, Value};

const MESH_RENDERER: i32 = 23;
const MESH_FILTER: i32 = 33;
const SKINNED_MESH_RENDERER: i32 = 137;

/// A model file and the images it refers to.
#[derive(Debug, Clone)]
pub struct ModelFile {
    /// The GLB file.
    pub glb: Vec<u8>,
    /// Its images, as the members `texture/<n>`.
    pub images: Vec<Member>,
}

/// The model file of `prefab`, specified in `docs/spec/model.md`: one scene, without a name,
/// whose only node is the root.
pub fn model_file(
    prefab: &Prefab,
    config: &ModelConfig<'_>,
    cx: &dyn Context,
) -> Result<ModelFile, ConvertError> {
    write(&[prefab], None, config, cx)
}

/// The model file of several prefabs, specified in `docs/spec/model.md`: one scene for each,
/// named after its root, in the order of `prefabs`, with the scene of `prefabs[shown]` shown
/// first. The nodes of each prefab follow those of the prefabs before it.
pub fn model_scenes(
    prefabs: &[&Prefab],
    shown: usize,
    config: &ModelConfig<'_>,
    cx: &dyn Context,
) -> Result<ModelFile, ConvertError> {
    if shown >= prefabs.len() {
        return Err(malformed("the scene shown first is not one of the scenes"));
    }
    write(prefabs, Some(shown), config, cx)
}

/// The model file of `prefabs`, with named scenes and the scene shown first when `shown` is
/// given, and otherwise with unnamed scenes and the first shown first.
fn write(
    prefabs: &[&Prefab],
    shown: Option<usize>,
    config: &ModelConfig<'_>,
    cx: &dyn Context,
) -> Result<ModelFile, ConvertError> {
    let mut seen = BTreeSet::new();
    for prefab in prefabs {
        if prefab.nodes.iter().any(|node| !seen.insert(node.object)) {
            return Err(malformed("a game object is in two scenes"));
        }
    }
    let mut builder = Builder {
        config,
        cx,
        buffers: Buffers::default(),
        materials: Materials::default(),
        geometry: BTreeMap::new(),
        meshes: Vec::new(),
        mesh_index: BTreeMap::new(),
        skins: Vec::new(),
    };
    let mut nodes = Vec::new();
    let mut scenes = Vec::new();
    for prefab in prefabs {
        let offset = nodes.len();
        for prefab_node in &prefab.nodes {
            let mut node = node(prefab_node, config, cx)?;
            for child in &mut node.children {
                *child += offset;
            }
            if let Some(renderer) = Renderer::find(prefab_node, cx)? {
                builder.attach(&renderer, &mut node, prefab, offset)?;
            }
            nodes.push(node);
        }
        scenes.push(Scene {
            name: shown.map(|_| prefab.nodes[0].name.clone()),
            nodes: vec![offset],
        });
    }
    builder.finish(nodes, scenes, shown.unwrap_or(0))
}

/// The node of a game object, without its mesh.
fn node(
    prefab_node: &Node,
    config: &ModelConfig<'_>,
    cx: &dyn Context,
) -> Result<gltf::Node, ConvertError> {
    let pose = prefab_node.pose();
    let mut extras = match config.node_keys {
        Some(source) => source.keys(prefab_node.object, cx)?,
        None => BTreeMap::new(),
    };
    if !prefab_node.active {
        extras.insert("active".to_owned(), Json::Bool(false));
    }
    Ok(gltf::Node {
        name: prefab_node.name.clone(),
        children: prefab_node.children.clone(),
        translation: pose.translation,
        rotation: pose.rotation,
        scale: pose.scale,
        mesh: None,
        skin: None,
        weights: Vec::new(),
        extras: NodeExtras(extras),
    })
}

/// The renderer of a game object.
struct Renderer<'a> {
    object: &'a Object,
    mesh: Option<ObjectId>,
    skinned: bool,
}

impl<'a> Renderer<'a> {
    /// The mesh renderer or skinned mesh renderer of `node`, with the mesh it draws.
    fn find(node: &'a Node, cx: &dyn Context) -> Result<Option<Self>, ConvertError> {
        let Some(object) = node
            .components
            .iter()
            .find(|c| matches!(c.class_id, MESH_RENDERER | SKINNED_MESH_RENDERER))
        else {
            return Ok(None);
        };
        let source = if object.class_id == SKINNED_MESH_RENDERER {
            Some(object)
        } else {
            node.components.iter().find(|c| c.class_id == MESH_FILTER)
        };
        let mesh = match source {
            Some(source) => cx.resolve(source.id, reference(&source.fields, "m_Mesh")?)?,
            None => None,
        };
        let skinned = object.class_id == SKINNED_MESH_RENDERER
            && !array(&object.fields, "m_Bones")?.is_empty();
        Ok(Some(Self {
            object,
            mesh,
            skinned,
        }))
    }
}

/// A mesh with its accessors, shared by the glTF meshes that draw it.
struct Geometry {
    mesh: Mesh,
    attributes: BTreeMap<String, usize>,
    /// The accessor of the weights, made when a skin first uses them.
    weights: Option<usize>,
    /// The accessor of the joints for each mapping of bone slots to joints.
    joints: BTreeMap<Vec<u16>, usize>,
    /// The accessor of each submesh.
    indices: Vec<usize>,
    targets: Vec<BTreeMap<String, usize>>,
}

/// What a glTF mesh is made of: a mesh, the material of each primitive, and, when it is
/// skinned, the joint of each bone slot.
type MeshKey = (ObjectId, Vec<Option<usize>>, Option<Vec<u16>>);

/// The mesh `mesh` with its accessors, read when first used. `flip_uv` says whether its texture
/// coordinates are written with `v` flipped.
fn load<'g>(
    geometry: &'g mut BTreeMap<ObjectId, Geometry>,
    mesh: ObjectId,
    flip_uv: bool,
    cx: &dyn Context,
    buffers: &mut Buffers,
) -> Result<&'g mut Geometry, ConvertError> {
    Ok(match geometry.entry(mesh) {
        Entry::Occupied(known) => known.into_mut(),
        Entry::Vacant(slot) => slot.insert(Geometry::new(mesh::read(mesh, cx)?, flip_uv, buffers)?),
    })
}

/// How the bone slots of a skinned renderer's mesh bind to the nodes of the model.
struct Binding {
    /// The joints, in the order of the slots they bind.
    joints: Vec<Joint>,
    /// The joint of each bone slot. A slot that binds no node has joint 0; no vertex weighs on
    /// it.
    slot_joints: Vec<u16>,
}

/// A joint of a skin: its node and the bind pose of its slot.
struct Joint {
    node: usize,
    inverse_bind: [f32; 16],
}

impl Binding {
    /// Binds the bone slots of `mesh` as `slot_nodes` describes, leaving out the slots that
    /// bind no node.
    fn new(
        renderer: &Object,
        mesh: &Mesh,
        prefab: &Prefab,
        cx: &dyn Context,
    ) -> Result<Self, ConvertError> {
        let skin = mesh
            .skin
            .as_ref()
            .ok_or_else(|| unsupported("the mesh of a skinned renderer has no bone weights"))?;
        let stored = array(&renderer.fields, "m_Bones")?
            .iter()
            .map(|bone| bone_node(renderer, bone, prefab, cx))
            .collect::<Result<Vec<_>, _>>()?;
        let nodes = slot_nodes(mesh, &stored, prefab)?;
        if skin.weighed.last().is_some_and(|&last| last >= nodes.len()) {
            return Err(malformed(
                "a vertex is bound to a bone the renderer does not have",
            ));
        }
        let mut binding = Self {
            joints: Vec::new(),
            slot_joints: Vec::with_capacity(nodes.len()),
        };
        for (slot, (node, &inverse_bind)) in nodes.into_iter().zip(&mesh.bind_poses).enumerate() {
            let joint = match node {
                Some(node) => {
                    let joint = u16::try_from(binding.joints.len())
                        .map_err(|_| unsupported("a skin has more than 65536 joints"))?;
                    binding.joints.push(Joint { node, inverse_bind });
                    joint
                }
                None if skin.weighed.contains(&slot) => {
                    return Err(unsupported(
                        "a bone that a vertex weighs on binds no node of the model",
                    ));
                }
                None => 0,
            };
            binding.slot_joints.push(joint);
        }
        Ok(binding)
    }
}

/// The node of each bone slot: the node whose path has the slot's bone name hash, or else the
/// node of the slot's stored bone. A node binds one slot only, and binding by hash comes first.
fn slot_nodes(
    mesh: &Mesh,
    stored: &[Option<usize>],
    prefab: &Prefab,
) -> Result<Vec<Option<usize>>, ConvertError> {
    let slots = mesh.bind_poses.len();
    let by_hash: Vec<Option<usize>> = if mesh.bone_hashes.len() == slots && slots > 0 {
        let node = |&hash: &u32| prefab.node_at_path_hash(hash);
        mesh.bone_hashes.iter().map(node).collect()
    } else if stored.len() == slots {
        vec![None; slots]
    } else {
        return Err(malformed(format!(
            "{} bones and {slots} bind poses",
            stored.len()
        )));
    };
    let mut nodes = vec![None; slots];
    let mut taken = BTreeSet::new();
    for candidates in [&by_hash[..], stored] {
        for (node, &candidate) in nodes.iter_mut().zip(candidates) {
            if node.is_none()
                && let Some(candidate) = candidate
                && taken.insert(candidate)
            {
                *node = Some(candidate);
            }
        }
    }
    Ok(nodes)
}

/// The node of a stored bone reference, if it is set and a node of the model.
fn bone_node(
    renderer: &Object,
    stored: &Value,
    prefab: &Prefab,
    cx: &dyn Context,
) -> Result<Option<usize>, ConvertError> {
    let pointer = stored
        .as_pptr()
        .ok_or_else(|| malformed("a bone is not a reference"))?;
    Ok(cx
        .resolve(renderer.id, pointer)?
        .and_then(|bone| prefab.node_of(bone)))
}

struct Builder<'a> {
    config: &'a ModelConfig<'a>,
    cx: &'a dyn Context,
    buffers: Buffers,
    materials: Materials,
    geometry: BTreeMap<ObjectId, Geometry>,
    meshes: Vec<gltf::Mesh>,
    mesh_index: BTreeMap<MeshKey, usize>,
    skins: Vec<gltf::Skin>,
}

impl Builder<'_> {
    /// Gives `node` the mesh, skin and state of `renderer`, a renderer of `prefab`, whose nodes
    /// follow `offset` nodes of the file.
    fn attach(
        &mut self,
        renderer: &Renderer<'_>,
        node: &mut gltf::Node,
        prefab: &Prefab,
        offset: usize,
    ) -> Result<(), ConvertError> {
        let fields = &renderer.object.fields;
        if !flag(fields, "m_Enabled")? {
            node.extras.insert("enabled", Json::Bool(false));
        }
        let Some(mesh) = renderer.mesh else {
            return Ok(());
        };
        let mut materials = Vec::new();
        for material in array(fields, "m_Materials")? {
            let material = material
                .as_pptr()
                .ok_or_else(|| malformed("a material is not a reference"))?;
            materials.push(match self.cx.resolve(renderer.object.id, material)? {
                Some(material) => Some(self.materials.material(material, self.config, self.cx)?),
                None => None,
            });
        }
        let geometry = load(
            &mut self.geometry,
            mesh,
            self.config.flip_uv,
            self.cx,
            &mut self.buffers,
        )?;
        let binding = if renderer.skinned {
            Some(Binding::new(
                renderer.object,
                &geometry.mesh,
                prefab,
                self.cx,
            )?)
        } else {
            None
        };
        let (slot_joints, joints) = binding
            .map(|binding| (binding.slot_joints, binding.joints))
            .unzip();
        node.mesh = Some(self.mesh(mesh, materials, slot_joints)?);
        if let Some(joints) = joints {
            node.skin = Some(self.skin(renderer.object, &joints, prefab, offset)?);
        }
        let geometry = &self.geometry[&mesh];
        if renderer.object.class_id == SKINNED_MESH_RENDERER && !geometry.mesh.targets.is_empty() {
            node.weights = weights(fields, &geometry.mesh)?;
        }
        Ok(())
    }

    /// The glTF mesh of `mesh` drawn with `materials`, skinned when the joint of each bone slot
    /// is given.
    fn mesh(
        &mut self,
        mesh: ObjectId,
        materials: Vec<Option<usize>>,
        slot_joints: Option<Vec<u16>>,
    ) -> Result<usize, ConvertError> {
        let key = (mesh, materials, slot_joints);
        if let Some(&known) = self.mesh_index.get(&key) {
            return Ok(known);
        }
        let geometry = load(
            &mut self.geometry,
            mesh,
            self.config.flip_uv,
            self.cx,
            &mut self.buffers,
        )?;
        let (_, materials, slot_joints) = &key;
        let mut attributes = geometry.attributes.clone();
        if let Some(slot_joints) = slot_joints {
            let (joints, weights) = geometry.skin_accessors(slot_joints, &mut self.buffers)?;
            attributes.insert("JOINTS_0".into(), joints);
            attributes.insert("WEIGHTS_0".into(), weights);
        }
        let primitive = |submesh: usize, material: Option<usize>| {
            // A submesh without triangles adds no primitive.
            (!geometry.mesh.submeshes[submesh].is_empty()).then(|| Primitive {
                attributes: attributes.clone(),
                indices: geometry.indices[submesh],
                material,
                mode: TRIANGLES,
                targets: geometry.targets.clone(),
            })
        };
        let submeshes = geometry.indices.len();
        let mut primitives: Vec<Primitive> = (0..submeshes)
            .filter_map(|submesh| primitive(submesh, materials.get(submesh).copied().flatten()))
            .collect();
        // A material beyond the submeshes draws the last submesh again.
        if let Some(last) = submeshes.checked_sub(1) {
            for &material in materials.iter().skip(submeshes) {
                primitives.extend(primitive(last, material));
            }
        }
        let targets = &geometry.mesh.targets;
        self.meshes.push(gltf::Mesh {
            name: geometry.mesh.name.clone(),
            primitives,
            extras: (!targets.is_empty()).then(|| MeshExtras {
                target_names: targets.iter().map(|t| t.name.clone()).collect(),
                full_weights: targets.iter().map(|t| t.full_weight).collect(),
            }),
        });
        let index = self.meshes.len() - 1;
        self.mesh_index.insert(key, index);
        Ok(index)
    }

    /// The skin of a skinned renderer with `joints`.
    fn skin(
        &mut self,
        renderer: &Object,
        joints: &[Joint],
        prefab: &Prefab,
        offset: usize,
    ) -> Result<usize, ConvertError> {
        let poses: Vec<[f32; 16]> = joints.iter().map(|joint| joint.inverse_bind).collect();
        let inverse_bind_matrices = self.buffers.floats(&poses, None, false);
        let root_bone = match renderer.fields.get("m_RootBone") {
            Some(stored) => bone_node(renderer, stored, prefab, self.cx)?,
            None => None,
        };
        self.skins.push(gltf::Skin {
            joints: joints.iter().map(|joint| offset + joint.node).collect(),
            inverse_bind_matrices,
            extras: root_bone.map(|root_bone| SkinExtras {
                root_bone: offset + root_bone,
            }),
        });
        Ok(self.skins.len() - 1)
    }

    fn finish(
        self,
        nodes: Vec<gltf::Node>,
        scenes: Vec<Scene>,
        shown: usize,
    ) -> Result<ModelFile, ConvertError> {
        let mut document = Document {
            asset: AssetInfo {
                version: "2.0",
                generator: "sekai-exporter",
            },
            extensions_used: Vec::new(),
            extensions_required: Vec::new(),
            scene: shown,
            scenes,
            nodes,
            meshes: self.meshes,
            skins: self.skins,
            materials: Vec::new(),
            textures: Vec::new(),
            samplers: Vec::new(),
            images: Vec::new(),
            accessors: Vec::new(),
            buffer_views: Vec::new(),
            buffers: Vec::new(),
        };
        let images = self
            .materials
            .finish(self.config.declare_webp, &mut document);
        let bin = self.buffers.finish(&mut document);
        let json = document
            .to_bytes()
            .map_err(|e| malformed(format!("the glTF document cannot be written: {e}")))?;
        Ok(ModelFile {
            glb: glb::container(&json, &bin)?,
            images,
        })
    }
}

impl Geometry {
    /// Adds the vertex attributes, indices and morph targets of `mesh`.
    fn new(mesh: Mesh, flip_uv: bool, buffers: &mut Buffers) -> Result<Self, ConvertError> {
        if mesh.positions.is_empty() {
            return Err(unsupported("the mesh has no vertices"));
        }
        let vertex = Some(ARRAY_BUFFER);
        let mut attributes = BTreeMap::new();
        attributes.insert(
            "POSITION".into(),
            buffers.floats(&mesh.positions, vertex, true),
        );
        if let Some(normals) = &mesh.normals {
            attributes.insert("NORMAL".into(), buffers.floats(normals, vertex, false));
        }
        if let Some(tangents) = &mesh.tangents {
            attributes.insert("TANGENT".into(), buffers.floats(tangents, vertex, false));
        }
        match &mesh.colors {
            Some(Colors::Float(colors)) => {
                attributes.insert("COLOR_0".into(), buffers.floats(colors, vertex, false));
            }
            Some(Colors::Bytes(colors)) => {
                attributes.insert("COLOR_0".into(), buffers.colors(colors));
            }
            None => {}
        }
        for (set, uvs) in mesh.uvs.iter().enumerate() {
            let uvs = written_uvs(uvs, flip_uv);
            attributes.insert(
                format!("TEXCOORD_{set}"),
                buffers.floats(&uvs, vertex, false),
            );
        }
        let indices = mesh
            .submeshes
            .iter()
            .map(|submesh| buffers.indices(submesh))
            .collect();
        let count = mesh.positions.len();
        let targets = mesh
            .targets
            .iter()
            .map(|target| {
                // A sparse accessor holds the offsets that are not zero.
                let offsets = |of: fn(&mesh::Delta) -> [f32; 3]| -> Vec<(u32, [f32; 3])> {
                    target
                        .deltas
                        .iter()
                        .map(|d| (d.vertex, of(d)))
                        .filter(|(_, offset)| *offset != [0.0; 3])
                        .collect()
                };
                let mut accessors = BTreeMap::new();
                accessors.insert(
                    "POSITION".into(),
                    buffers.sparse(count, &offsets(|d| d.position), true),
                );
                if mesh.normals.is_some() {
                    accessors.insert(
                        "NORMAL".into(),
                        buffers.sparse(count, &offsets(|d| d.normal), false),
                    );
                }
                if mesh.tangents.is_some() {
                    accessors.insert(
                        "TANGENT".into(),
                        buffers.sparse(count, &offsets(|d| d.tangent), false),
                    );
                }
                accessors
            })
            .collect();
        Ok(Self {
            mesh,
            attributes,
            weights: None,
            joints: BTreeMap::new(),
            indices,
            targets,
        })
    }

    /// The accessors of the joints, each bone slot replaced by its joint, and of the weights.
    fn skin_accessors(
        &mut self,
        slot_joints: &[u16],
        buffers: &mut Buffers,
    ) -> Result<(usize, usize), ConvertError> {
        let skin = self
            .mesh
            .skin
            .as_ref()
            .ok_or_else(|| unsupported("the mesh of a skinned renderer has no bone weights"))?;
        let weights = *self
            .weights
            .get_or_insert_with(|| buffers.floats(&skin.weights, Some(ARRAY_BUFFER), false));
        if let Some(&known) = self.joints.get(slot_joints) {
            return Ok((known, weights));
        }
        // Every slot a vertex weighs on is a slot of the binding, and a slot without weight
        // is stored as 0.
        let joints: Vec<[u16; 4]> = skin
            .joints
            .iter()
            .map(|vertex| vertex.map(|slot| slot_joints[usize::from(slot)]))
            .collect();
        let accessor = buffers.joints(&joints);
        self.joints.insert(slot_joints.to_vec(), accessor);
        Ok((accessor, weights))
    }
}

/// Texture coordinates as written: with `v` flipped, for images stored top row first, when
/// `flip_uv` is set, and as stored otherwise.
fn written_uvs(uvs: &[[f32; 2]], flip_uv: bool) -> Vec<[f32; 2]> {
    if flip_uv {
        uvs.iter().map(|&[u, v]| [u, 1.0 - v]).collect()
    } else {
        uvs.to_vec()
    }
}

/// The morph target weights of a skinned renderer: each channel's stored weight, 0 when the
/// list ends before it, divided by the channel's full weight.
fn weights(renderer: &Value, mesh: &Mesh) -> Result<Vec<f32>, ConvertError> {
    let stored = array(renderer, "m_BlendShapeWeights")?;
    if stored.len() > mesh.targets.len() {
        return Err(malformed(
            "a renderer has more blend shape weights than channels",
        ));
    }
    mesh.targets
        .iter()
        .enumerate()
        .map(|(channel, target)| {
            let weight = match stored.get(channel) {
                Some(w) => w
                    .as_f64()
                    .ok_or_else(|| malformed("a blend shape weight is not a number"))?
                    as f32,
                None => 0.0,
            };
            let value = weight / target.full_weight;
            if value.is_finite() {
                Ok(value)
            } else {
                Err(malformed(
                    "a blend shape weight is not finite after scaling",
                ))
            }
        })
        .collect()
}
