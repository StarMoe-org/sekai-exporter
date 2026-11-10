//! The names of assets: a pure function of the root objects of a version.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};

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

/// A root object that needs a name.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    /// The logical path of the root.
    pub logical: &'a str,
    /// The asset form.
    pub form: &'a str,
    /// The class ID of the root.
    pub class_id: i32,
    /// The class name of the root.
    pub class: &'a str,
    /// The bundle name of the root.
    pub bundle: &'a str,
    /// The path ID of the root.
    pub path_id: i64,
}

impl Candidate<'_> {
    fn rank(&self) -> (u8, &str) {
        match self.class_id {
            TEXTURE_2D => (0, ""),
            SPRITE => (1, ""),
            _ => (2, self.class),
        }
    }
}

/// The name of each candidate, in the order of `candidates`. `None` marks a candidate that
/// cannot be told apart from another one: same logical path, class, path ID and form.
pub fn assign(candidates: &[Candidate<'_>]) -> Vec<Option<String>> {
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, candidate) in candidates.iter().enumerate() {
        groups.entry(candidate.logical).or_default().push(i);
    }
    let mut names = vec![None; candidates.len()];
    for (logical, mut members) in groups {
        members.sort_by_key(|&i| {
            let c = &candidates[i];
            (c.rank(), c.bundle, c.path_id, c.form)
        });
        let classes = members.chunk_by(|&a, &b| candidates[a].rank() == candidates[b].rank());
        for (position, class) in classes.enumerate() {
            let base = if position == 0 {
                logical.to_owned()
            } else {
                format!("{logical}@{}", candidates[class[0]].class)
            };
            let mut uses: HashMap<(i64, &str), u32> = HashMap::new();
            for &i in class {
                *uses
                    .entry((candidates[i].path_id, candidates[i].form))
                    .or_default() += 1;
            }
            let objects = class.chunk_by(|&a, &b| {
                let (ca, cb) = (&candidates[a], &candidates[b]);
                (ca.bundle, ca.path_id) == (cb.bundle, cb.path_id)
            });
            for (order, object) in objects.enumerate() {
                let path_id = candidates[object[0]].path_id;
                let with_id = if order == 0 {
                    base.clone()
                } else {
                    format!("{base}@{path_id}")
                };
                for (form_order, &i) in object.iter().enumerate() {
                    let form = candidates[i].form;
                    if uses[&(path_id, form)] > 1 {
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
            class_id,
            class,
            bundle: "b",
            path_id,
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
    fn distinct_forms_on_one_root_take_the_form_name() {
        let mut beta = candidate("a", "GameObject", 5);
        beta.form = "beta";
        let mut alpha = candidate("a", "GameObject", 5);
        alpha.form = "alpha";
        assert_eq!(names(&[beta, alpha]), [Some("a@beta"), Some("a")]);
    }
}
