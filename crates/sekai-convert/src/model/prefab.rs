//! The game objects of a prefab, in depth-first order, with their transforms and components.

use std::collections::BTreeMap;

use super::read::{array, components, field, finite, flag, malformed, reference, string, vector3};
use crate::{Context, ConvertError, Object, ObjectId};

/// The class ID of `Transform`.
const TRANSFORM: i32 = 4;
/// The class ID of `RectTransform`.
const RECT_TRANSFORM: i32 = 224;

/// A prefab: a root game object and everything below it.
#[derive(Debug, Clone)]
pub struct Prefab {
    /// The game objects, in depth-first order with the root first.
    pub(super) nodes: Vec<Node>,
    /// The node of each game object and each `Transform`.
    by_object: BTreeMap<ObjectId, usize>,
    /// The node of each other component, and its position among the node's components.
    by_component: BTreeMap<ObjectId, (usize, usize)>,
    /// The first node in depth-first order with each CRC-32 of a path.
    by_path_hash: BTreeMap<u32, usize>,
}

/// A game object of a prefab.
#[derive(Debug, Clone)]
pub struct Node {
    /// The game object.
    pub(super) object: ObjectId,
    /// Its transform.
    pub(super) transform_id: ObjectId,
    /// Every component of `m_Component` that resolves, the transform among them, in stored order.
    pub(super) component_ids: Vec<ObjectId>,
    pub(super) name: String,
    /// The names from below the root down to it, joined by `/`.
    pub(super) path: String,
    pub(super) active: bool,
    pub(super) children: Vec<usize>,
    pub(super) transform: Transform,
    /// Its components other than the transform, in the order of `m_Component`.
    pub(super) components: Vec<Object>,
}

impl Node {
    /// The game object.
    pub fn object(&self) -> ObjectId {
        self.object
    }

    /// The game object's transform.
    pub fn transform(&self) -> ObjectId {
        self.transform_id
    }

    /// Whether the game object is active itself (`m_IsActive`).
    pub fn active(&self) -> bool {
        self.active
    }

    /// The positions of its children in [`Prefab::nodes`], in the order the transform stores them.
    pub fn children(&self) -> &[usize] {
        &self.children
    }

    /// Every component of the game object, the transform among them, in stored order.
    pub fn component_ids(&self) -> &[ObjectId] {
        &self.component_ids
    }

    /// Its components other than the transform, in stored order.
    pub fn components(&self) -> &[Object] {
        &self.components
    }
}

/// The local transform of a game object, in the engine's coordinates.
#[derive(Debug, Clone, Copy)]
pub(super) struct Transform {
    pub(super) position: [f32; 3],
    pub(super) rotation: [f32; 4],
    pub(super) scale: [f32; 3],
}

impl Prefab {
    /// The game objects, in depth-first order with the root first. A node's children are
    /// positions in this list.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// The node of the game object or transform `id`.
    pub fn node(&self, id: ObjectId) -> Option<&Node> {
        self.node_of(id).map(|node| &self.nodes[node])
    }

    /// Reads the prefab whose root is the game object `root`.
    pub fn read(root: ObjectId, cx: &dyn Context) -> Result<Self, ConvertError> {
        let mut prefab = Self {
            nodes: Vec::new(),
            by_object: BTreeMap::new(),
            by_component: BTreeMap::new(),
            by_path_hash: BTreeMap::new(),
        };
        prefab.walk(root, None, cx)?;
        Ok(prefab)
    }

    /// The node of a game object or of its transform.
    pub(super) fn node_of(&self, id: ObjectId) -> Option<usize> {
        self.by_object.get(&id).copied()
    }

    /// The first node whose path has the CRC-32 `hash`.
    pub(super) fn node_at_path_hash(&self, hash: u32) -> Option<usize> {
        self.by_path_hash.get(&hash).copied()
    }

    /// A component other than a transform: its node and the component.
    pub(super) fn component(&self, id: ObjectId) -> Option<(usize, &Object)> {
        let &(node, position) = self.by_component.get(&id)?;
        Some((node, &self.nodes[node].components[position]))
    }

    /// Adds the game object `id`, the child of a node at `parent` or the root, and everything
    /// below it, and returns its node.
    fn walk(
        &mut self,
        id: ObjectId,
        parent: Option<&str>,
        cx: &dyn Context,
    ) -> Result<usize, ConvertError> {
        if self.by_object.contains_key(&id) {
            return Err(malformed("a game object is its own descendant"));
        }
        let object = cx.object(id)?;
        let fields = &object.fields;
        let name = string(fields, "m_Name")?.to_owned();
        let path = match parent {
            None => String::new(),
            Some("") => name.clone(),
            Some(parent) => format!("{parent}/{name}"),
        };
        let mut transform = None;
        let mut others = Vec::new();
        let mut component_ids = Vec::new();
        for entry in array(fields, "m_Component")? {
            let Some(component) = cx.resolve(id, reference(entry, "component")?)? else {
                continue;
            };
            component_ids.push(component);
            let component = cx.object(component)?;
            if !matches!(component.class_id, TRANSFORM | RECT_TRANSFORM) {
                others.push(component);
            } else if transform.replace(component).is_some() {
                return Err(malformed("a game object has two transforms"));
            }
        }
        let transform = transform.ok_or_else(|| malformed("a game object has no transform"))?;
        let node = self.nodes.len();
        self.by_object.insert(id, node);
        self.by_object.insert(transform.id, node);
        self.by_path_hash
            .entry(crc32fast::hash(path.as_bytes()))
            .or_insert(node);
        for (position, component) in others.iter().enumerate() {
            self.by_component.insert(component.id, (node, position));
        }
        self.nodes.push(Node {
            object: id,
            transform_id: transform.id,
            component_ids,
            name,
            path: path.clone(),
            active: flag(fields, "m_IsActive")?,
            children: Vec::new(),
            transform: Transform::read(&transform)?,
            components: others,
        });
        for child in array(&transform.fields, "m_Children")? {
            let child = child
                .as_pptr()
                .ok_or_else(|| malformed("a child is not a reference"))?;
            let Some(child) = cx.resolve(transform.id, child)? else {
                continue;
            };
            let owner = cx
                .resolve(child, reference(&cx.object(child)?.fields, "m_GameObject")?)?
                .ok_or_else(|| malformed("a transform has no game object"))?;
            let child_node = self.walk(owner, Some(&path), cx)?;
            self.nodes[node].children.push(child_node);
        }
        Ok(node)
    }
}

impl Transform {
    fn read(transform: &Object) -> Result<Self, ConvertError> {
        let fields = &transform.fields;
        Ok(Self {
            position: finite(vector3(fields, "m_LocalPosition")?, "a position")?,
            rotation: finite(
                components(field(fields, "m_LocalRotation")?, ["x", "y", "z", "w"])?,
                "a rotation",
            )?,
            scale: finite(vector3(fields, "m_LocalScale")?, "a scale")?,
        })
    }
}
