//! The overhead items of the characters.
//!
//! [`Emoticon`] produces the `emoticon` form and [`Emoticons`] the `emoticons` form, both
//! specified in `docs/spec/emoticons.md`.

use std::collections::{BTreeMap, HashMap, VecDeque};

use crate::roots::emoticon;
use sekai_convert::animation::clip_document;
use sekai_convert::tables::{TableContext, TableConverter, TablePlan};
use sekai_convert::{
    Asset, Context, ConvertError, Converter, Object, ObjectId, ObjectIndex, PPtr, Value,
};
use sekai_format::clip::{ClipDocument, Curve};
use serde::Serialize;
use serde_json::{Map, Value as Json, json};

use crate::particle;
use crate::scripts::{DOCUMENT, NoAssets, OneAsset, document, malformed};
use crate::stored::field;

const FORM: &str = "emoticon";
const NAME: &str = "mysekai-runtime/emoticons/emoticons";
const PACKAGE_PREFIX: &str = "mysekai/effect/emoticon/";

const TEXTURE_EXTENSION: &str = "png";
const LOOP_END_FLAG: &str = "LOOP_END_FLAG";
/// The most frames an overhead item's resampled clip may hold.
const MAX_FRAMES: u32 = 100_000;

const TRANSFORM: i32 = 4;
const ANIMATOR: i32 = 95;
const MONO_BEHAVIOUR: i32 = 114;
const TEXTURE_2D: i32 = 28;
const PARTICLE_SYSTEM: i32 = 198;
const PARTICLE_RENDERER: i32 = 199;
const SPRITE_RENDERER: i32 = 212;
const RECT_TRANSFORM: i32 = 224;

const SPRITE_VIEW: &str = "Assembly-CSharp/Sekai.Mysekai.AvatarEmoticonSpriteView";
const PARTICLE_VIEW: &str = "Assembly-CSharp/Sekai.Mysekai.AvatarEmoticonParticleView";

const ANCHORS: &[&str] = &["Face", "Spine", "Hips"];
const CLIP_PHASES: &[&str] = &["start", "loop", "end"];
/// The name and the number of components of each animated property of a `Transform`.
const TRANSFORM_PROPERTIES: &[(u32, &str, u8)] = &[
    (1, "position", 3),
    (2, "rotation", 4),
    (3, "scale", 3),
    (4, "eulerAngles", 3),
];
/// The floating-point material properties a runtime draws with.
const MATERIAL_FLOATS: &[&str] = &[
    "_BaseMapMode",
    "_BaseMapRotation",
    "_BaseMapRotationCoord",
    "_BaseMapRotationEnabled",
    "_Cull",
    "_SoftParticlesEnabled",
    "_ZOffset",
    "_ZTest",
    "_ZWrite",
];

const OBJECT_NOT_MODELLED: &str = "object type not modelled";
const NON_TRANSFORM_BINDING: &str = "non-transform binding";
const UNKNOWN_ATTRIBUTE: &str = "unmodelled transform attribute";
const UNRESOLVED_PATH: &str = "path hash unresolved";
const NO_TEXTURE_FILE: &str = "texture has no path in its package";

/// One overhead item as a JSON document.
#[derive(Debug, Clone, Copy, Default)]
pub struct Emoticon;

impl Converter for Emoticon {
    fn name(&self) -> &'static str {
        FORM
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        emoticon(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        document(&Walk::new(cx).item(root)?).map(Some)
    }
}

/// Every overhead item of a version in one document.
#[derive(Debug, Clone, Copy, Default)]
pub struct Emoticons;

impl TableConverter for Emoticons {
    fn name(&self) -> &'static str {
        "emoticons"
    }

    fn version(&self) -> u32 {
        1
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        if item_assets(cx).next().is_none() {
            return Ok(Box::new(NoAssets));
        }
        Ok(Box::new(OneAsset::new(NAME, move || build(cx))))
    }
}

/// The last segment of the package and the name of each `emoticon` asset of the version.
fn item_assets<'c>(cx: &'c dyn TableContext) -> impl Iterator<Item = (&'c str, &'c str)> + 'c {
    cx.bundles()
        .into_iter()
        .filter(|bundle| bundle.starts_with(PACKAGE_PREFIX))
        .flat_map(move |bundle| {
            let item = &bundle[PACKAGE_PREFIX.len()..];
            cx.assets(bundle)
                .iter()
                .filter(|entry| entry.form == FORM)
                .map(move |entry| (item, entry.name.as_str()))
        })
}

fn build(cx: &dyn TableContext) -> Result<Asset, ConvertError> {
    let mut items = Map::new();
    let mut unsupported = Vec::new();
    for (item, asset) in item_assets(cx) {
        let bytes = cx.member(asset, DOCUMENT)?;
        let document: Json =
            serde_json::from_slice(&bytes).map_err(|e| malformed(format!("{asset}: {e}")))?;
        if let Some(gaps) = document["unsupported"].as_array() {
            unsupported.extend(gaps.iter().cloned());
        }
        items.insert(item.to_owned(), document);
    }
    let summary = json!({ "items": items.len(), "unsupported": unsupported });
    document(&json!({ "items": items, "summary": summary }))
}

#[derive(Serialize)]
struct Item {
    view: View,
    #[serde(skip_serializing_if = "Option::is_none")]
    animator: Option<Animator>,
    textures: Vec<Texture>,
    sprites: BTreeMap<String, Sprite>,
    nodes: Vec<Node>,
    clips: BTreeMap<String, Clip>,
    particles: Vec<Emitter>,
    unsupported: Vec<Json>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct View {
    kind: &'static str,
    sound_input: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    anchor: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_position: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Animator {
    loop_end_flag: &'static str,
}

#[derive(Serialize, Clone, PartialEq)]
struct Texture {
    name: String,
    file: String,
    width: u64,
    height: u64,
}

#[derive(Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct Sprite {
    texture: String,
    rect: Vec<f64>,
    pivot: Vec<f64>,
    pixels_to_units: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Node {
    path: String,
    parent: Option<String>,
    active: bool,
    position: Vec<f64>,
    rotation: Vec<f64>,
    scale: Vec<f64>,
    animation_path: Option<String>,
    #[serde(flatten)]
    drawing: Option<Drawing>,
}

/// What a node with a sprite renderer draws.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Drawing {
    sprite: Option<String>,
    sorting_order: i64,
    color: Vec<f64>,
    flip_x: bool,
    flip_y: bool,
    renderer_enabled: bool,
    material: Option<Material>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Material {
    textures: BTreeMap<String, String>,
    texture_scale_offset: BTreeMap<String, [f64; 4]>,
    floats: BTreeMap<String, f64>,
}

#[derive(Serialize)]
struct Clip {
    rate: f32,
    duration: f32,
    channels: Vec<Channel>,
}

#[derive(Serialize)]
struct Channel {
    path: Option<String>,
    property: &'static str,
    values: Vec<Vec<f32>>,
}

#[derive(Serialize)]
struct Emitter {
    node: String,
    system: Json,
    renderer: Map<String, Json>,
}

/// A `ParticleSystem` and a `ParticleSystemRenderer` of a node, found during the walk.
struct EmitterParts {
    system: Option<Object>,
    renderer: Option<Object>,
}

/// What a walk of the prefab of an item collects.
struct Walk<'c> {
    cx: &'c dyn Context,
    nodes: Vec<Node>,
    node_of_game_object: HashMap<i64, String>,
    view: Option<View>,
    animator: Option<AnimatorPart>,
    emitters: BTreeMap<(String, usize), EmitterParts>,
    textures: BTreeMap<String, Texture>,
    sprites: BTreeMap<String, Sprite>,
    unsupported: Vec<Json>,
}

/// The node of the animator of an item and its controller.
struct AnimatorPart {
    node: String,
    controller: ObjectId,
}

impl<'c> Walk<'c> {
    fn new(cx: &'c dyn Context) -> Self {
        Self {
            cx,
            nodes: Vec::new(),
            node_of_game_object: HashMap::new(),
            view: None,
            animator: None,
            emitters: BTreeMap::new(),
            textures: BTreeMap::new(),
            sprites: BTreeMap::new(),
            unsupported: Vec::new(),
        }
    }

    fn item(mut self, root: ObjectId) -> Result<Item, ConvertError> {
        self.tree(root)?;
        let animation_paths = self.resolve_animation_paths();
        let clips = self.clips(&animation_paths)?;
        let particles = self.particles()?;
        let view = self
            .view
            .take()
            .ok_or_else(|| malformed("the item has no view"))?;
        Ok(Item {
            view,
            animator: self.animator.as_ref().map(|_| Animator {
                loop_end_flag: LOOP_END_FLAG,
            }),
            textures: self.textures.into_values().collect(),
            sprites: self.sprites,
            nodes: self.nodes,
            clips,
            particles,
            unsupported: self.unsupported,
        })
    }

    /// Visits the nodes of the prefab, parents first and siblings in path id order.
    fn tree(&mut self, root: ObjectId) -> Result<(), ConvertError> {
        let game_object = self.cx.object(root)?;
        let transform = self.transform_of(&game_object)?;
        let mut queue = VecDeque::from([(transform, None::<String>)]);
        while let Some((id, parent)) = queue.pop_front() {
            let transform = self.cx.object(id)?;
            let path = self.visit(&transform, parent)?;
            for child in self.children(&transform)? {
                queue.push_back((child, Some(path.clone())));
            }
        }
        Ok(())
    }

    fn transform_of(&self, game_object: &Object) -> Result<ObjectId, ConvertError> {
        for component in self.components(game_object)? {
            let class_id = self.cx.object(component)?.class_id;
            if class_id == TRANSFORM || class_id == RECT_TRANSFORM {
                return Ok(component);
            }
        }
        Err(malformed("the game object has no transform"))
    }

    fn components(&self, game_object: &Object) -> Result<Vec<ObjectId>, ConvertError> {
        let entries = field(&game_object.fields, "m_Component")?
            .as_array()
            .ok_or_else(|| malformed("m_Component is not an array"))?;
        entries
            .iter()
            .filter_map(|entry| {
                let component = reference(entry, "component").transpose()?;
                Some(component.and_then(|pointer| self.resolved(game_object.id, pointer)))
            })
            .collect()
    }

    fn resolved(&self, from: ObjectId, pointer: PPtr) -> Result<ObjectId, ConvertError> {
        self.cx
            .resolve(from, pointer)?
            .ok_or_else(|| ConvertError::new("missing_object", "a reference does not resolve"))
    }

    fn children(&self, transform: &Object) -> Result<Vec<ObjectId>, ConvertError> {
        let mut pointers: Vec<PPtr> = field(&transform.fields, "m_Children")?
            .as_array()
            .ok_or_else(|| malformed("m_Children is not an array"))?
            .iter()
            .filter_map(Value::as_pptr)
            .collect();
        pointers.sort_by_key(|pointer| pointer.path_id);
        pointers
            .into_iter()
            .map(|pointer| self.resolved(transform.id, pointer))
            .collect()
    }

    /// Records the node of `transform` and its components, and returns the path of the node.
    fn visit(
        &mut self,
        transform: &Object,
        parent: Option<String>,
    ) -> Result<String, ConvertError> {
        let owner = required_reference(&transform.fields, "m_GameObject")?;
        let game_object = self.cx.object(self.resolved(transform.id, owner)?)?;
        let name = string(&game_object.fields, "m_Name")?;
        let path = match parent.as_deref() {
            None => String::new(),
            Some("") => name.to_owned(),
            Some(parent) => format!("{parent}/{name}"),
        };
        self.node_of_game_object.insert(owner.path_id, path.clone());
        let index = self.nodes.len();
        let mut drawing = None;
        for component in self.components(&game_object)? {
            let component = self.cx.object(component)?;
            if component.class_id == SPRITE_RENDERER {
                drawing = Some(self.drawing(&component)?);
            } else {
                self.component(component, &path, index)?;
            }
        }
        self.nodes.push(Node {
            path: path.clone(),
            parent,
            active: is_set(&game_object.fields, "m_IsActive")?,
            position: numbers(&transform.fields, "m_LocalPosition", &["x", "y", "z"])?,
            rotation: numbers(&transform.fields, "m_LocalRotation", &["x", "y", "z", "w"])?,
            scale: numbers(&transform.fields, "m_LocalScale", &["x", "y", "z"])?,
            animation_path: None,
            drawing,
        });
        Ok(path)
    }

    /// Records a component other than a sprite renderer, by what it is.
    fn component(
        &mut self,
        component: Object,
        node: &str,
        index: usize,
    ) -> Result<(), ConvertError> {
        match component.class_id {
            TRANSFORM | RECT_TRANSFORM => {}
            MONO_BEHAVIOUR => self.view_component(&component)?,
            ANIMATOR => self.animator_component(&component, node)?,
            PARTICLE_SYSTEM => {
                self.emitter_parts(node, index).system = Some(component);
            }
            PARTICLE_RENDERER => {
                self.emitter_parts(node, index).renderer = Some(component);
            }
            _ => self.unsupported.push(json!({
                "type": component.class,
                "reason": OBJECT_NOT_MODELLED,
            })),
        }
        Ok(())
    }

    fn emitter_parts(&mut self, node: &str, index: usize) -> &mut EmitterParts {
        self.emitters
            .entry((node.to_owned(), index))
            .or_insert(EmitterParts {
                system: None,
                renderer: None,
            })
    }

    fn view_component(&mut self, behaviour: &Object) -> Result<(), ConvertError> {
        let kind = match behaviour.script.as_deref() {
            Some(SPRITE_VIEW) => "sprite",
            Some(PARTICLE_VIEW) => "particle",
            _ => return Ok(()),
        };
        let fields = &behaviour.fields;
        let anchor = fields
            .get("_avatarEmoticonRootType")
            .map(anchor_name)
            .transpose()?;
        self.view = Some(View {
            kind,
            sound_input: string(fields, "_input")?.to_owned(),
            anchor,
            keep_position: fields.get("_keepPosition").map(stored_flag).transpose()?,
        });
        Ok(())
    }

    fn animator_component(&mut self, animator: &Object, node: &str) -> Result<(), ConvertError> {
        let controller = required_reference(&animator.fields, "m_Controller")?;
        self.animator = Some(AnimatorPart {
            node: node.to_owned(),
            controller: self.resolved(animator.id, controller)?,
        });
        Ok(())
    }

    fn drawing(&mut self, renderer: &Object) -> Result<Drawing, ConvertError> {
        let fields = &renderer.fields;
        let sprite = reference(fields, "m_Sprite")?
            .map(|pointer| self.sprite(renderer.id, pointer))
            .transpose()?;
        Ok(Drawing {
            sprite,
            sorting_order: integer(fields, "m_SortingOrder")?,
            color: numbers(fields, "m_Color", &["r", "g", "b", "a"])?,
            flip_x: is_set(fields, "m_FlipX")?,
            flip_y: is_set(fields, "m_FlipY")?,
            renderer_enabled: is_set(fields, "m_Enabled")?,
            material: self.first_material(renderer)?,
        })
    }

    /// Records the sprite a reference names and returns its name.
    fn sprite(&mut self, from: ObjectId, pointer: PPtr) -> Result<String, ConvertError> {
        let sprite = self.cx.object(self.resolved(from, pointer)?)?;
        let fields = &sprite.fields;
        let name = string(fields, "m_Name")?.to_owned();
        let texture_pointer = required_reference(field(fields, "m_RD")?, "texture")?;
        let texture = self.texture(self.resolved(sprite.id, texture_pointer)?)?;
        let texture = texture.ok_or_else(|| malformed(format!("sprite {name} has no texture")))?;
        let entry = Sprite {
            texture: texture.name,
            rect: numbers(fields, "m_Rect", &["x", "y", "width", "height"])?,
            pivot: numbers(fields, "m_Pivot", &["x", "y"])?,
            pixels_to_units: number(fields, "m_PixelsToUnits")?,
        };
        if let Some(existing) = self.sprites.get(&name)
            && existing != &entry
        {
            return Err(malformed(format!("two sprites are named {name}")));
        }
        self.sprites.insert(name.clone(), entry);
        Ok(name)
    }

    /// Records the texture `id` and returns it, or `None` when it cannot be drawn from a file.
    fn texture(&mut self, id: ObjectId) -> Result<Option<Texture>, ConvertError> {
        let texture = self.cx.object(id)?;
        if texture.class_id != TEXTURE_2D {
            self.report_once(json!({
                "type": texture.class,
                "reason": OBJECT_NOT_MODELLED,
            }));
            return Ok(None);
        }
        let name = string(&texture.fields, "m_Name")?.to_owned();
        let Some(file) = self.texture_file(id)? else {
            self.report_once(json!({
                "texture": name,
                "reason": NO_TEXTURE_FILE,
            }));
            return Ok(None);
        };
        let entry = Texture {
            name: name.clone(),
            file,
            width: count(&texture.fields, "m_Width")?,
            height: count(&texture.fields, "m_Height")?,
        };
        if let Some(existing) = self.textures.get(&name)
            && existing != &entry
        {
            return Err(malformed(format!("two textures are named {name}")));
        }
        self.textures.insert(name, entry.clone());
        Ok(Some(entry))
    }

    /// Records `gap` unless it is recorded already, as a texture many slots share would be.
    fn report_once(&mut self, gap: Json) {
        if !self.unsupported.contains(&gap) {
            self.unsupported.push(gap);
        }
    }

    /// The file of the `texture` form of the texture `id`, relative to the emoticon directory.
    fn texture_file(&self, id: ObjectId) -> Result<Option<String>, ConvertError> {
        let entries = self.cx.container(id)?;
        Ok(entries
            .iter()
            .filter(|(_, object)| *object == id)
            .find_map(|(path, _)| {
                let lower = path.to_ascii_lowercase();
                let pos = lower.find(PACKAGE_PREFIX)?;
                let relative = &path[pos + PACKAGE_PREFIX.len()..];
                let (stem, _) = relative.rsplit_once('.')?;
                Some(format!("{stem}.{TEXTURE_EXTENSION}"))
            }))
    }

    /// The first material of `renderer`, which is the one it draws its sprites or particles with.
    fn first_material(&mut self, renderer: &Object) -> Result<Option<Material>, ConvertError> {
        let slots = field(&renderer.fields, "m_Materials")?
            .as_array()
            .ok_or_else(|| malformed("m_Materials is not an array"))?;
        let pointer = slots
            .first()
            .and_then(Value::as_pptr)
            .filter(|p| p.path_id != 0);
        let Some(pointer) = pointer else {
            return Ok(None);
        };
        let id = self.resolved(renderer.id, pointer)?;
        self.material(id).map(Some)
    }

    fn material(&mut self, id: ObjectId) -> Result<Material, ConvertError> {
        let material = self.cx.object(id)?;
        let saved = field(&material.fields, "m_SavedProperties")?;
        let mut textures = BTreeMap::new();
        let mut texture_scale_offset = BTreeMap::new();
        for (name, slot) in property_pairs(saved, "m_TexEnvs")? {
            let scale = numbers(slot, "m_Scale", &["x", "y"])?;
            let offset = numbers(slot, "m_Offset", &["x", "y"])?;
            texture_scale_offset
                .insert(name.to_owned(), [scale[0], scale[1], offset[0], offset[1]]);
            if let Some(pointer) = reference(slot, "m_Texture")? {
                let texture = self.resolved(material.id, pointer)?;
                if let Some(texture) = self.texture(texture)? {
                    textures.insert(name.to_owned(), texture.file);
                }
            }
        }
        let floats = property_pairs(saved, "m_Floats")?
            .into_iter()
            .filter(|(name, _)| MATERIAL_FLOATS.contains(name))
            .map(|(name, value)| Ok((name.to_owned(), float(value, name)?)))
            .collect::<Result<_, ConvertError>>()?;
        Ok(Material {
            textures,
            texture_scale_offset,
            floats,
        })
    }

    /// Sets the path of each node relative to the animator, and returns those paths by hash.
    fn resolve_animation_paths(&mut self) -> HashMap<u32, String> {
        let anchor = self
            .animator
            .as_ref()
            .map_or("", |animator| animator.node.as_str());
        let prefix = if anchor.is_empty() {
            String::new()
        } else {
            format!("{anchor}/")
        };
        let mut by_hash = HashMap::new();
        for node in &mut self.nodes {
            let relative = if node.path == anchor {
                Some("")
            } else {
                node.path.strip_prefix(&prefix)
            };
            if let Some(relative) = relative {
                by_hash.insert(crc32fast::hash(relative.as_bytes()), relative.to_owned());
            }
            node.animation_path = relative.map(str::to_owned);
        }
        by_hash
    }

    fn clips(
        &mut self,
        animation_paths: &HashMap<u32, String>,
    ) -> Result<BTreeMap<String, Clip>, ConvertError> {
        let Some(animator) = &self.animator else {
            return Ok(BTreeMap::new());
        };
        let controller = self.cx.object(animator.controller)?;
        let pointers = field(&controller.fields, "m_AnimationClips")?
            .as_array()
            .ok_or_else(|| malformed("m_AnimationClips is not an array"))?;
        let mut clips = BTreeMap::new();
        for pointer in pointers.iter().filter_map(Value::as_pptr) {
            let id = self.resolved(controller.id, pointer)?;
            let clip = clip_document(id, self.cx)?;
            clips.insert(
                phase(&clip.name).to_owned(),
                resampled(&clip, animation_paths, &mut self.unsupported)?,
            );
        }
        Ok(clips)
    }

    fn particles(&mut self) -> Result<Vec<Emitter>, ConvertError> {
        let parts = std::mem::take(&mut self.emitters);
        parts
            .into_iter()
            .map(|((node, _), parts)| self.emitter(node, parts))
            .collect()
    }

    fn emitter(&mut self, node: String, parts: EmitterParts) -> Result<Emitter, ConvertError> {
        let (Some(system), Some(renderer)) = (parts.system, parts.renderer) else {
            return Err(malformed(format!(
                "node {node} does not have both a particle system and a renderer"
            )));
        };
        let decoded = particle::system(&system.fields, &|pointer| {
            self.node_of_component(system.id, pointer)
        })?;
        self.unsupported.extend(
            decoded
                .unsupported
                .into_iter()
                .map(|gap| with_node(gap, &node)),
        );
        let mut draw = particle::renderer(&renderer.fields)?;
        let material = self.first_material(&renderer)?;
        draw.insert(
            "material".to_owned(),
            serde_json::to_value(material).map_err(malformed)?,
        );
        Ok(Emitter {
            node,
            system: decoded.document,
            renderer: draw,
        })
    }

    /// The node of the component that `pointer` names, if it is in the same file.
    fn node_of_component(
        &self,
        from: ObjectId,
        pointer: PPtr,
    ) -> Result<Option<String>, ConvertError> {
        if pointer.file_id != 0 || pointer.path_id == 0 {
            return Ok(None);
        }
        let component = self.cx.object(self.resolved(from, pointer)?)?;
        let owner = required_reference(&component.fields, "m_GameObject")?;
        Ok(self.node_of_game_object.get(&owner.path_id).cloned())
    }
}

fn with_node(mut gap: Json, node: &str) -> Json {
    gap["node"] = Json::from(node);
    gap
}

/// The phase a clip plays in, from the end of its name, or its whole name if it has none.
fn phase(clip: &str) -> &str {
    let tail = clip.rsplit('_').next().unwrap_or(clip);
    CLIP_PHASES
        .iter()
        .find(|phase| tail.eq_ignore_ascii_case(phase))
        .map_or(clip, |phase| *phase)
}

/// The `Transform` channels of `clip`, sampled at the rate of the clip.
fn resampled(
    clip: &ClipDocument,
    animation_paths: &HashMap<u32, String>,
    unsupported: &mut Vec<Json>,
) -> Result<Clip, ConvertError> {
    let frames = frame_count(clip)?;
    let mut grouped: BTreeMap<(u32, u32), Vec<&Curve>> = BTreeMap::new();
    let mut other_attributes = Vec::new();
    for curve in &clip.curves {
        if curve.class_id == TRANSFORM {
            grouped
                .entry((curve.path, curve.attribute))
                .or_default()
                .push(curve);
        } else {
            other_attributes.push(curve.attribute);
        }
    }
    other_attributes.sort_unstable();
    other_attributes.dedup();
    unsupported.extend(
        other_attributes
            .into_iter()
            .map(|attribute| json!({ "attribute": attribute, "reason": NON_TRANSFORM_BINDING })),
    );
    let channels = grouped
        .into_iter()
        .filter_map(|((hash, attribute), curves)| {
            let Some(&(_, property, width)) = TRANSFORM_PROPERTIES
                .iter()
                .find(|(known, ..)| *known == attribute)
            else {
                unsupported.push(json!({
                    "pathHash": hash,
                    "attribute": attribute,
                    "reason": UNKNOWN_ATTRIBUTE,
                }));
                return None;
            };
            let path = animation_paths.get(&hash).cloned();
            if path.is_none() {
                unsupported.push(json!({ "pathHash": hash, "reason": UNRESOLVED_PATH }));
            }
            let values = (0..frames)
                .map(|frame| sample(&curves, width, frame as f32 / clip.sample_rate))
                .collect();
            Some(Channel {
                path,
                property,
                values,
            })
        })
        .collect();
    Ok(Clip {
        rate: clip.sample_rate,
        duration: clip.stop_time,
        channels,
    })
}

/// The number of frames that cover a clip at its own rate, at least two.
fn frame_count(clip: &ClipDocument) -> Result<u32, ConvertError> {
    if !clip.sample_rate.is_finite() || clip.sample_rate <= 0.0 {
        return Err(malformed("the clip has a non-positive sample rate"));
    }
    if !clip.stop_time.is_finite() || clip.stop_time < 0.0 {
        return Err(malformed("the clip has a negative duration"));
    }
    let product = f64::from(clip.stop_time) * f64::from(clip.sample_rate);
    if !product.is_finite() || product.round() >= f64::from(MAX_FRAMES) {
        return Err(malformed("the clip has too many frames"));
    }
    let last_frame = product.round() as u32;
    Ok((last_frame + 1).max(2))
}

/// The `width` components of a property at `time`; a component without a curve is 0.
fn sample(curves: &[&Curve], width: u8, time: f32) -> Vec<f32> {
    (0..width)
        .map(|component| {
            curves
                .iter()
                .find(|curve| curve.component == component)
                .map_or(0.0, |curve| curve.data.evaluate(time))
        })
        .collect()
}

fn reference(value: &Value, name: &str) -> Result<Option<PPtr>, ConvertError> {
    let pointer = field(value, name)?
        .as_pptr()
        .ok_or_else(|| malformed(format!("{name} is not a reference")))?;
    Ok((pointer.path_id != 0).then_some(pointer))
}

fn required_reference(value: &Value, name: &str) -> Result<PPtr, ConvertError> {
    reference(value, name)?.ok_or_else(|| malformed(format!("{name} is null")))
}

fn string<'v>(value: &'v Value, name: &str) -> Result<&'v str, ConvertError> {
    field(value, name)?
        .as_str()
        .ok_or_else(|| malformed(format!("{name} is not a string")))
}

fn float(value: &Value, name: &str) -> Result<f64, ConvertError> {
    value
        .as_f64()
        .ok_or_else(|| malformed(format!("{name} is not a number")))
}

fn number(value: &Value, name: &str) -> Result<f64, ConvertError> {
    float(field(value, name)?, name)
}

fn numbers(value: &Value, name: &str, components: &[&str]) -> Result<Vec<f64>, ConvertError> {
    let section = field(value, name)?;
    components
        .iter()
        .map(|component| number(section, component))
        .collect()
}

fn integer(value: &Value, name: &str) -> Result<i64, ConvertError> {
    field(value, name)?
        .as_i64()
        .ok_or_else(|| malformed(format!("{name} is not an integer")))
}

fn count(value: &Value, name: &str) -> Result<u64, ConvertError> {
    field(value, name)?
        .as_u64()
        .ok_or_else(|| malformed(format!("{name} is not a count")))
}

fn stored_flag(value: &Value) -> Result<bool, ConvertError> {
    value
        .as_bool()
        .or_else(|| value.as_u64().map(|stored| stored != 0))
        .ok_or_else(|| malformed("a flag is neither a boolean nor an integer"))
}

fn is_set(value: &Value, name: &str) -> Result<bool, ConvertError> {
    stored_flag(field(value, name)?)
}

fn anchor_name(value: &Value) -> Result<&'static str, ConvertError> {
    value
        .as_u64()
        .and_then(|index| ANCHORS.get(usize::try_from(index).ok()?))
        .copied()
        .ok_or_else(|| ConvertError::new("unsupported", "the view has an unknown anchor"))
}

/// The entries of the property map `name`, by property name.
fn property_pairs<'v>(
    saved: &'v Value,
    name: &str,
) -> Result<Vec<(&'v str, &'v Value)>, ConvertError> {
    field(saved, name)?
        .as_map()
        .ok_or_else(|| malformed(format!("{name} is not a map")))?
        .iter()
        .map(|(key, value)| {
            let key = key
                .as_str()
                .ok_or_else(|| malformed(format!("a key of {name} is not a string")))?;
            Ok((key, value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekai_format::clip::CurveData;

    fn clip_with(curves: Vec<Curve>) -> ClipDocument {
        ClipDocument {
            format: sekai_format::clip::FORMAT.to_owned(),
            version: sekai_format::clip::VERSION,
            name: "clip_emote_loop".to_owned(),
            sample_rate: 2.0,
            start_time: 0.0,
            stop_time: 1.0,
            looping: false,
            curves,
            events: Vec::new(),
        }
    }

    fn constant_curve(
        class_id: i32,
        path: u32,
        attribute: u32,
        component: u8,
        value: f32,
    ) -> Curve {
        Curve {
            path,
            attribute,
            class_id,
            script: None,
            custom_type: 0,
            component,
            data: CurveData::Constant { value },
        }
    }

    #[test]
    fn a_clip_ending_in_a_phase_is_named_by_it() {
        assert_eq!(phase("clip_emote_001_Start"), "start");
        assert_eq!(phase("clip_emote_001_loop"), "loop");
        assert_eq!(phase("clip_emote_001_pose"), "clip_emote_001_pose");
    }

    #[test]
    fn a_channel_samples_every_frame_of_its_components() {
        let curves = vec![
            constant_curve(TRANSFORM, 7, 1, 0, 1.5),
            constant_curve(TRANSFORM, 7, 1, 2, 2.5),
        ];
        let paths = HashMap::from([(7, "mark".to_owned())]);
        let mut unsupported = Vec::new();
        let clip = resampled(&clip_with(curves), &paths, &mut unsupported).unwrap();
        assert_eq!(clip.channels.len(), 1);
        assert_eq!(clip.channels[0].path.as_deref(), Some("mark"));
        assert_eq!(clip.channels[0].property, "position");
        assert_eq!(clip.channels[0].values, vec![vec![1.5, 0.0, 2.5]; 3]);
        assert!(unsupported.is_empty());
    }

    #[test]
    fn what_a_channel_cannot_carry_is_reported() {
        let curves = vec![
            constant_curve(TRANSFORM, 9, 3, 0, 1.0),
            constant_curve(TRANSFORM, 7, 8, 0, 1.0),
            constant_curve(212, 7, 11, 0, 1.0),
            constant_curve(212, 7, 11, 1, 1.0),
        ];
        let paths = HashMap::from([(7, "mark".to_owned())]);
        let mut unsupported = Vec::new();
        let clip = resampled(&clip_with(curves), &paths, &mut unsupported).unwrap();
        assert_eq!(clip.channels.len(), 1);
        assert_eq!(clip.channels[0].path, None);
        assert_eq!(
            unsupported,
            vec![
                json!({ "attribute": 11, "reason": NON_TRANSFORM_BINDING }),
                json!({ "pathHash": 7, "attribute": 8, "reason": UNKNOWN_ATTRIBUTE }),
                json!({ "pathHash": 9, "reason": UNRESOLVED_PATH }),
            ]
        );
    }

    #[test]
    fn a_clip_of_no_length_still_has_two_frames() {
        let mut clip = clip_with(Vec::new());
        clip.stop_time = 0.0;
        assert_eq!(frame_count(&clip).unwrap(), 2);
    }

    #[test]
    fn invalid_clip_rates_and_extreme_lengths_are_malformed() {
        let mut zero_rate = clip_with(Vec::new());
        zero_rate.sample_rate = 0.0;
        assert_eq!(frame_count(&zero_rate).unwrap_err().code, "malformed");

        let mut huge = clip_with(Vec::new());
        huge.stop_time = 1.0e20;
        assert_eq!(frame_count(&huge).unwrap_err().code, "malformed");
    }
}
