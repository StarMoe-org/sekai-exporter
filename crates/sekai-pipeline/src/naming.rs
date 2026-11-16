//! The names of assets: a pure function of the root objects of a version.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

const RESOURCES: &str = "assets/sekai/assetbundle/resources/";

const SPRITE: i32 = 213;
const TEXTURE_2D: i32 = 28;

/// The logical path of a container path: without the resources directory and its category, and
/// without the extension.
pub fn logical_path(container_path: &str) -> &str {
    let path = container_path
        .strip_prefix(RESOURCES)
        .and_then(|rest| rest.split_once('/'))
        .map_or(container_path, |(_, rest)| rest);
    match path.rfind('.') {
        Some(dot) if !path[dot..].contains('/') => &path[..dot],
        _ => path,
    }
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

/// A root that needs a name.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    /// The logical path of the root.
    pub logical: &'a str,
    /// The asset form.
    pub form: &'a str,
    /// What the root is in its bundle.
    pub origin: Origin<'a>,
    /// The bundle name of the root.
    pub bundle: &'a str,
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
        /// The path ID.
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

    /// The path ID of an object. A file has none: in a bundle, its logical path tells it apart.
    fn path_id(&self) -> Option<i64> {
        match *self {
            Self::Object { path_id, .. } => Some(path_id),
            Self::File => None,
        }
    }
}

/// The path ID of an object, or `file`.
impl fmt::Display for Origin<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.path_id() {
            Some(path_id) => write!(f, "{path_id}"),
            None => f.write_str(self.class()),
        }
    }
}

/// The name of each candidate, in the order of `candidates`. `None` marks a candidate that
/// cannot be told apart from another one: same logical path, class, path ID and form, or the
/// same file and form.
pub fn assign(candidates: &[Candidate<'_>]) -> Vec<Option<String>> {
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, candidate) in candidates.iter().enumerate() {
        groups.entry(candidate.logical).or_default().push(i);
    }
    let mut names = vec![None; candidates.len()];
    for (logical, mut members) in groups {
        members.sort_by_key(|&i| {
            let c = &candidates[i];
            (c.origin.rank(), c.bundle, c.origin.path_id(), c.form)
        });
        let rank = |i: usize| candidates[i].origin.rank();
        let classes = members.chunk_by(|&a, &b| rank(a) == rank(b));
        for (position, class) in classes.enumerate() {
            let base = if position == 0 {
                logical.to_owned()
            } else {
                format!("{logical}@{}", candidates[class[0]].origin.class())
            };
            let mut uses: HashMap<(Option<i64>, &str), u32> = HashMap::new();
            for &i in class {
                let key = (candidates[i].origin.path_id(), candidates[i].form);
                *uses.entry(key).or_default() += 1;
            }
            let objects = class.chunk_by(|&a, &b| {
                let (ca, cb) = (&candidates[a], &candidates[b]);
                (ca.bundle, ca.origin.path_id()) == (cb.bundle, cb.origin.path_id())
            });
            for (order, object) in objects.enumerate() {
                let origin = candidates[object[0]].origin;
                let with_id = if order == 0 {
                    base.clone()
                } else {
                    format!("{base}@{origin}")
                };
                for (form_order, &i) in object.iter().enumerate() {
                    let form = candidates[i].form;
                    if uses[&(origin.path_id(), form)] > 1 {
                        continue;
                    }
                    names[i] = Some(if form_order == 0 {
                        with_id.clone()
                    } else {
                        format!("{with_id}@{form}")
                    });
                }
            }
        }
    }
    names
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
                path_id,
            },
            bundle: "b",
        }
    }

    fn file(logical: &'static str, bundle: &'static str) -> Candidate<'static> {
        Candidate {
            logical,
            form: "form",
            origin: Origin::File,
            bundle,
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

    #[test]
    fn a_shared_path_id_has_no_name() {
        let mut other = candidate("a", "Sprite", 3);
        other.bundle = "c";
        let c = [
            candidate("a", "Sprite", 3),
            other,
            candidate("a", "Sprite", 8),
        ];
        assert_eq!(names(&c), [None, None, Some("a@8")]);
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
        let c = [
            file("a", "b"),
            candidate("a", "Mesh", 1),
            file("x.acb", "b"),
        ];
        assert_eq!(names(&c), [Some("a@file"), Some("a"), Some("x.acb")]);
    }

    #[test]
    fn files_of_two_bundles_under_one_logical_path_have_no_name() {
        let c = [file("a", "c"), file("a", "b")];
        assert_eq!(names(&c), [None, None]);
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
