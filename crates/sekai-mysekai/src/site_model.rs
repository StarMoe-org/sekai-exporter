//! The packages of the MySekai places as model files.
//!
//! [`SiteModel`] produces the `site-model` form: every prefab of one package of the places, such as
//! a field, a level of the house or a door, as one model file with a scene for each prefab, the
//! source identity, renderer state and collision record of every node, and the collision meshes of
//! the package beside it. The form is specified in `docs/spec/site-model.md`.

use std::collections::BTreeMap;

use sekai_convert::Converter;
use sekai_convert::model::{ModelConfig, NodeKeys, Prefab, model_scenes};
use sekai_convert::roots::{is_site_prefab, site_model};
use sekai_convert::{
    Asset, Content, Context, ConvertError, Cost, CostClass, Member, ObjectId, ObjectIndex,
};
use serde_json::Value as Json;

use crate::model_keys::{ColliderMeshes, Collision, ShaderKeys, source_keys};
use crate::scripts::malformed;

/// The converting of a package, estimated from the largest fields: a few hundred meshes and a
/// hundred images of up to 2048 × 2048, with their working copies.
const PACKAGE_MEMORY: u64 = 512 << 20;

/// One package of the places: its prefabs as one model file, with their collision.
#[derive(Debug, Clone, Copy, Default)]
pub struct SiteModel;

impl Converter for SiteModel {
    fn name(&self) -> &'static str {
        "site-model"
    }

    fn version(&self) -> u32 {
        2
    }

    fn encoders(&self) -> sekai_convert::Encoders {
        sekai_convert::Encoders::IMAGE
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        site_model(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let package = Package::of(root, cx)?;
        if package.principal() != root {
            return Ok(None);
        }
        let prefabs = package
            .prefabs
            .iter()
            .map(|&(_, prefab)| Prefab::read(prefab, cx))
            .collect::<Result<Vec<_>, _>>()?;
        let mut collision = Collision::new(ColliderMeshes::AsAuthored);
        for prefab in &prefabs {
            collision.add(prefab, cx)?;
        }
        let node_keys = SiteNodes {
            prefabs: &prefabs,
            collision: &collision,
        };
        let config = ModelConfig {
            // Texture coordinates keep their `v`, which the runtime flips when it samples an image.
            flip_uv: false,
            node_keys: Some(&node_keys),
            material_keys: Some(&ShaderKeys),
        };
        let scenes: Vec<&Prefab> = prefabs.iter().collect();
        let model = model_scenes(&scenes, package.shown, &config, cx)?;
        let collision = serde_json::to_vec(&collision.document())
            .map_err(|e| malformed(format!("the collision document cannot be written: {e}")))?;
        let mut members = vec![file("model", "glb", model.glb)];
        members.extend(model.images);
        members.push(file("collision", "json", collision));
        Ok(Some(Asset { members }))
    }

    /// The other prefabs of the package, which the principal prefab writes as scenes of its file.
    fn inputs(&self, root: ObjectId, cx: &dyn Context) -> Vec<ObjectId> {
        match Package::of(root, cx) {
            Ok(package) if package.principal() == root => package
                .prefabs
                .iter()
                .map(|&(_, prefab)| prefab)
                .filter(|&prefab| prefab != root)
                .collect(),
            _ => Vec::new(),
        }
    }

    fn cost(&self, root: ObjectId, _object: &ObjectIndex, cx: &dyn Context) -> Cost {
        let principal = Package::of(root, cx).is_ok_and(|package| package.principal() == root);
        Cost {
            class: if principal {
                CostClass::Heavy
            } else {
                CostClass::Light
            },
            memory: if principal { PACKAGE_MEMORY } else { 0 },
        }
    }
}

/// The prefabs of a package, in the order of their paths, and which of them is shown first.
struct Package {
    /// Each prefab's container path, in lower case, and its root game object.
    prefabs: Vec<(String, ObjectId)>,
    /// The position of the principal prefab in `prefabs`.
    shown: usize,
}

impl Package {
    /// The package of the prefab `root`: the prefabs the container of its bundle lists.
    fn of(root: ObjectId, cx: &dyn Context) -> Result<Self, ConvertError> {
        let mut paths: BTreeMap<ObjectId, String> = BTreeMap::new();
        // The container lists a prefab under its root game object, so the path alone says which
        // entries are prefabs, and no other object needs to be read.
        for (path, object) in cx.container(root)? {
            if !is_site_prefab(&path) {
                continue;
            }
            let path = path.to_ascii_lowercase();
            // A prefab listed under two paths goes by the first of them.
            match paths.get_mut(&object) {
                Some(known) if *known <= path => {}
                Some(known) => *known = path,
                None => {
                    paths.insert(object, path);
                }
            }
        }
        let mut prefabs: Vec<(String, ObjectId)> = paths
            .into_iter()
            .map(|(object, path)| (path, object))
            .collect();
        prefabs.sort_by(|a, b| (&a.0, a.1.path_id).cmp(&(&b.0, b.1.path_id)));
        if !prefabs.iter().any(|&(_, prefab)| prefab == root) {
            return Err(malformed("the prefab is not listed in its bundle"));
        }
        let shown = principal(&prefabs);
        Ok(Self { prefabs, shown })
    }

    /// The root game object of the principal prefab.
    fn principal(&self) -> ObjectId {
        self.prefabs[self.shown].1
    }
}

/// The position of the principal prefab among `prefabs`, which are in path order: the prefab
/// `<directory>/<name>.prefab`, where `<directory>` is the deepest directory that holds every
/// prefab and `<name>` is its last part, or else the first prefab.
fn principal(prefabs: &[(String, ObjectId)]) -> usize {
    let directory = prefabs
        .iter()
        .map(|(path, _)| path.rsplit_once('/').map_or("", |(directory, _)| directory))
        .reduce(common_directory)
        .unwrap_or("");
    let name = directory.rsplit('/').next().unwrap_or(directory);
    let expected = format!("{directory}/{name}.prefab");
    prefabs
        .iter()
        .position(|(path, _)| *path == expected)
        .unwrap_or(0)
}

/// The deepest directory that holds both `a` and `b`, which are directories.
fn common_directory<'a>(a: &'a str, b: &str) -> &'a str {
    let mut end = 0;
    let mut a_parts = a.split('/');
    let mut b_parts = b.split('/');
    while let (Some(x), Some(y)) = (a_parts.next(), b_parts.next()) {
        if x != y {
            break;
        }
        end += x.len() + usize::from(end > 0);
    }
    &a[..end]
}

/// The keys of the nodes of a package: those of every MySekai model.
struct SiteNodes<'a> {
    prefabs: &'a [Prefab],
    collision: &'a Collision,
}

impl NodeKeys for SiteNodes<'_> {
    fn keys(
        &self,
        object: ObjectId,
        cx: &dyn Context,
    ) -> Result<BTreeMap<String, Json>, ConvertError> {
        for prefab in self.prefabs {
            if let Some(node) = prefab.node(object) {
                return source_keys(prefab, node, self.collision, cx);
            }
        }
        Err(malformed("a node is not a game object of the package"))
    }
}

/// A member of the asset that holds finished bytes.
fn file(role: &str, extension: &str, bytes: Vec<u8>) -> Member {
    Member {
        role: role.into(),
        content: Content::File {
            extension: extension.into(),
            bytes,
        },
    }
}
