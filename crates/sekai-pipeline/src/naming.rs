//! The names of assets: a pure function of the root objects of a version.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

/// The resource directories of the game's downloaded bundles. Below each lies a directory of the
/// download category, and below that the path an asset is loaded by.
const BUNDLE_RESOURCES: [&str; 2] = [
    "assets/sekai/assetbundle/resources/",
    "assets/sekai/builtinassets/assetbundle/resources/",
];
/// The resource directory of the assets built into the client. As in a Unity `Resources` folder,
/// the path below it is the path an asset is loaded by.
const BUILT_IN_RESOURCES: &str = "assets/sekai/builtinassets/resources/";

const SPRITE: i32 = 213;
const TEXTURE_2D: i32 = 28;

/// The logical path of a container path: the path its asset is loaded by below its resource
/// directory, without the extension.
pub fn logical_path(container_path: &str) -> &str {
    let path = load_path(container_path);
    match path.rfind('.') {
        Some(dot) if !path[dot..].contains('/') => &path[..dot],
        _ => path,
    }
}

/// The path below the resource directory of `container_path`, and below the category of a
/// downloaded bundle's resources; the whole path when it lies in no resource directory.
fn load_path(container_path: &str) -> &str {
    let in_bundle = BUNDLE_RESOURCES
        .iter()
        .find_map(|directory| container_path.strip_prefix(directory))
        .and_then(|rest| rest.split_once('/'))
        .map(|(_, path)| path);
    in_bundle
        .or_else(|| container_path.strip_prefix(BUILT_IN_RESOURCES))
        .unwrap_or(container_path)
}

/// The path a root of `bundle` with the logical path `logical` is named by: the logical path when
/// it lies in the directory of the bundle, and otherwise the logical path under that directory.
pub fn path_in_bundle<'a>(bundle: &str, logical: &'a str) -> Cow<'a, str> {
    match logical.strip_prefix(bundle) {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => Cow::Borrowed(logical),
        _ => Cow::Owned(format!("{bundle}/{logical}")),
    }
}

/// The logical path of an object without a container path, of the class `class` and named
/// `name`: `loose/<class>/<name>` when its class is one of `classes`, and none for an object
/// without a name.
pub fn loose_path(class: &str, name: Option<&str>, classes: &[&str]) -> Option<String> {
    let name = name.filter(|name| !name.is_empty())?;
    classes
        .contains(&class)
        .then(|| format!("loose/{class}/{name}"))
}

/// A root of a bundle that needs a name.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    /// The logical path of the root.
    pub logical: &'a str,
    /// The asset form.
    pub form: &'a str,
    /// What the root is in its bundle.
    pub origin: Origin<'a>,
}

/// What a root is in its bundle.
#[derive(Debug, Clone, Copy)]
pub enum Origin<'a> {
    /// An object.
    Object {
        /// The class ID.
        class_id: i32,
        /// The class name.
        class: &'a str,
        /// The serialized file that holds the object, when the bundle holds several.
        file: Option<&'a str>,
        /// The path ID of the object in its file.
        path_id: i64,
    },
    /// A file that is not a serialized file, which its logical path names.
    File,
}

impl Origin<'_> {
    /// Objects come first, ordered by class: textures, then sprites, then the others by name.
    fn rank(&self) -> (u8, &str) {
        match *self {
            Self::Object {
                class_id: TEXTURE_2D,
                ..
            } => (0, ""),
            Self::Object {
                class_id: SPRITE, ..
            } => (1, ""),
            Self::Object { class, .. } => (2, class),
            Self::File => (3, ""),
        }
    }

    /// The class name of an object, or `file`.
    fn class(&self) -> &str {
        match *self {
            Self::Object { class, .. } => class,
            Self::File => "file",
        }
    }

    /// An object by its path ID and its file. A file has none: in a bundle, its logical path tells
    /// it apart.
    fn object(&self) -> Option<(i64, Option<&str>)> {
        match *self {
            Self::Object { path_id, file, .. } => Some((path_id, file)),
            Self::File => None,
        }
    }
}

/// The name of each root of one bundle, in the order of `candidates`. Roots that share a logical
/// path are told apart by class, then by object, then by form. `None` marks a candidate whose name
/// another candidate would receive as well.
pub fn assign(candidates: &[Candidate<'_>]) -> Vec<Option<String>> {
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, candidate) in candidates.iter().enumerate() {
        groups.entry(candidate.logical).or_default().push(i);
    }
    let mut names = vec![String::new(); candidates.len()];
    for (logical, mut members) in groups {
        members.sort_by_key(|&i| {
            let c = &candidates[i];
            (c.origin.rank(), c.origin.object(), c.form)
        });
        let rank = |i: usize| candidates[i].origin.rank();
        for (position, class) in members.chunk_by(|&a, &b| rank(a) == rank(b)).enumerate() {
            let base = match position {
                0 => logical.to_owned(),
                _ => format!("{logical}@{}", candidates[class[0]].origin.class()),
            };
            name_class(candidates, class, &base, &mut names);
        }
    }
    unique(names)
}

/// Names the roots `class` of `candidates`, which share a logical path and a class, after `base`:
/// each object but the first takes `@<path ID>`, or `@<file>:<path ID>` when another object of the
/// class has the same path ID, and each form of an object but the first in name order takes
/// `@<form>`.
fn name_class(candidates: &[Candidate<'_>], class: &[usize], base: &str, names: &mut [String]) {
    let object = |i: usize| candidates[i].origin.object();
    let objects: Vec<&[usize]> = class.chunk_by(|&a, &b| object(a) == object(b)).collect();
    let mut objects_of: HashMap<i64, usize> = HashMap::new();
    for (path_id, _) in objects.iter().filter_map(|roots| object(roots[0])) {
        *objects_of.entry(path_id).or_default() += 1;
    }
    for (order, roots) in objects.iter().enumerate() {
        let name = match object(roots[0]) {
            Some((path_id, Some(file))) if order > 0 && objects_of[&path_id] > 1 => {
                format!("{base}@{file}:{path_id}")
            }
            Some((path_id, _)) if order > 0 => format!("{base}@{path_id}"),
            _ => base.to_owned(),
        };
        let first_form = candidates[roots[0]].form;
        for &i in *roots {
            names[i] = match candidates[i].form {
                form if form == first_form => name.clone(),
                form => format!("{name}@{form}"),
            };
        }
    }
}

/// Each of `names` that only one candidate receives: a name that several would receive tells none
/// of them apart.
fn unique(names: Vec<String>) -> Vec<Option<String>> {
    let mut uses: HashMap<&str, usize> = HashMap::new();
    for name in &names {
        *uses.entry(name).or_default() += 1;
    }
    let once: Vec<bool> = names.iter().map(|name| uses[name.as_str()] == 1).collect();
    names
        .into_iter()
        .zip(once)
        .map(|(name, once)| once.then_some(name))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(logical: &'static str, class: &'static str, path_id: i64) -> Candidate<'static> {
        let class_id = match class {
            "Sprite" => SPRITE,
            "Texture2D" => TEXTURE_2D,
            _ => 1,
        };
        Candidate {
            logical,
            form: "form",
            origin: Origin::Object {
                class_id,
                class,
                file: None,
                path_id,
            },
        }
    }

    fn file(logical: &'static str) -> Candidate<'static> {
        Candidate {
            logical,
            form: "form",
            origin: Origin::File,
        }
    }

    fn names(candidates: &[Candidate<'_>]) -> Vec<Option<&'static str>> {
        assign(candidates)
            .into_iter()
            .map(|n| n.map(|n| &*n.leak()))
            .collect()
    }

    #[test]
    fn a_logical_path_has_no_prefix_or_extension() {
        assert_eq!(
            logical_path("assets/sekai/assetbundle/resources/startapp/a/b.png"),
            "a/b"
        );
        assert_eq!(logical_path("other/a.b/c"), "other/a.b/c");
        assert_eq!(logical_path("a/b.acb.bytes"), "a/b.acb");
    }

    #[test]
    fn the_resources_built_into_the_client_are_named_by_their_load_path() {
        let dir = "assets/sekai/builtinassets/";
        assert_eq!(
            logical_path(&format!("{dir}resources/font/a.otf")),
            "font/a"
        );
        let shader = format!("{dir}assetbundle/resources/ondemand/shader/live/b.shader");
        assert_eq!(logical_path(&shader), "shader/live/b");
        let uncategorised = "assets/sekai/assetbundle/resources/c.png";
        assert_eq!(
            logical_path(uncategorised),
            "assets/sekai/assetbundle/resources/c"
        );
    }

    #[test]
    fn a_name_stays_in_the_directory_of_its_bundle() {
        assert_eq!(path_in_bundle("a/b", "a/b/c"), "a/b/c");
        assert_eq!(path_in_bundle("a/b", "a/b"), "a/b");
        assert_eq!(path_in_bundle("a/b", "a/bc/d"), "a/b/a/bc/d");
        assert_eq!(path_in_bundle("a/b", "x/y"), "a/b/x/y");
    }

    #[test]
    fn a_loose_object_of_a_named_class_is_named_by_class_and_name() {
        let classes = ["Sprite"];
        assert_eq!(
            loose_path("Sprite", Some("frame"), &classes).as_deref(),
            Some("loose/Sprite/frame")
        );
        assert_eq!(loose_path("Texture2D", Some("frame"), &classes), None);
        assert_eq!(loose_path("Sprite", Some(""), &classes), None);
        assert_eq!(loose_path("Sprite", None, &classes), None);
    }

    #[test]
    fn a_lone_asset_keeps_its_logical_path() {
        assert_eq!(names(&[candidate("a", "Mesh", 7)]), [Some("a")]);
    }

    #[test]
    fn classes_are_ordered_by_priority() {
        let c = [
            candidate("a", "Mesh", 1),
            candidate("a", "Texture2D", 2),
            candidate("a", "Sprite", 3),
        ];
        assert_eq!(names(&c), [Some("a@Mesh"), Some("a"), Some("a@Sprite")]);
    }

    #[test]
    fn the_same_class_takes_the_path_id() {
        let c = [
            candidate("a", "Sprite", 9),
            candidate("a", "Sprite", -4),
            candidate("a", "Mesh", 5),
            candidate("a", "Mesh", 6),
        ];
        assert_eq!(
            names(&c),
            [Some("a@9"), Some("a"), Some("a@Mesh"), Some("a@Mesh@6")]
        );
    }

    /// `candidate` in the serialized file `name` of a bundle that holds several.
    fn in_file(mut candidate: Candidate<'static>, name: &'static str) -> Candidate<'static> {
        if let Origin::Object { file, .. } = &mut candidate.origin {
            *file = Some(name);
        }
        candidate
    }

    #[test]
    fn objects_of_two_files_with_one_path_id_take_their_file() {
        let c = [
            in_file(candidate("a", "Sprite", 3), "g"),
            in_file(candidate("a", "Sprite", 8), "g"),
            in_file(candidate("a", "Sprite", 3), "f"),
        ];
        assert_eq!(names(&c), [Some("a@g:3"), Some("a@8"), Some("a")]);
    }

    #[test]
    fn the_order_of_the_candidates_does_not_matter() {
        let a = candidate("x", "Sprite", 1);
        let b = candidate("x", "Sprite", 2);
        assert_eq!(names(&[a, b]), [Some("x"), Some("x@2")]);
        assert_eq!(names(&[b, a]), [Some("x@2"), Some("x")]);
    }

    #[test]
    fn a_file_keeps_its_logical_path_and_comes_after_the_objects() {
        let c = [file("a"), candidate("a", "Mesh", 1), file("x.acb")];
        assert_eq!(names(&c), [Some("a@file"), Some("a"), Some("x.acb")]);
    }

    #[test]
    fn a_name_that_two_roots_would_receive_is_given_to_neither() {
        let c = [file("a"), file("a"), candidate("a", "Mesh", 1)];
        assert_eq!(names(&c), [None, None, Some("a")]);
    }

    #[test]
    fn distinct_forms_on_one_root_take_the_form_name() {
        let mut beta = candidate("a", "GameObject", 5);
        beta.form = "beta";
        let mut alpha = candidate("a", "GameObject", 5);
        alpha.form = "alpha";
        assert_eq!(names(&[beta, alpha]), [Some("a@beta"), Some("a")]);
    }
}
