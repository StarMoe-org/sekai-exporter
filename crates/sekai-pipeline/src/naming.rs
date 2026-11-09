//! The names of assets: a pure function of the root objects of a version.

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

/// A root object that needs a name.
#[derive(Debug, Clone, Copy)]
pub struct Candidate<'a> {
    /// The logical path of the root.
    pub logical: &'a str,
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
            SPRITE => (0, ""),
            TEXTURE_2D => (1, ""),
            _ => (2, self.class),
        }
    }
}

/// The name of each candidate, in the order of `candidates`. `None` marks a candidate that
/// cannot be told apart from another one: same logical path, class and path ID.
pub fn assign(candidates: &[Candidate<'_>]) -> Vec<Option<String>> {
    let mut groups: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, candidate) in candidates.iter().enumerate() {
        groups.entry(candidate.logical).or_default().push(i);
    }
    let mut names = vec![None; candidates.len()];
    for (logical, mut members) in groups {
        members.sort_by_key(|&i| {
            let c = &candidates[i];
            (c.rank(), c.bundle, c.path_id)
        });
        let classes = members.chunk_by(|&a, &b| candidates[a].rank() == candidates[b].rank());
        for (position, class) in classes.enumerate() {
            let base = if position == 0 {
                logical.to_owned()
            } else {
                format!("{logical}@{}", candidates[class[0]].class)
            };
            let mut uses: HashMap<i64, u32> = HashMap::new();
            for &i in class {
                *uses.entry(candidates[i].path_id).or_default() += 1;
            }
            for (order, &i) in class.iter().enumerate() {
                let path_id = candidates[i].path_id;
                if uses[&path_id] > 1 {
                    continue;
                }
                names[i] = Some(if order == 0 {
                    base.clone()
                } else {
                    format!("{base}@{path_id}")
                });
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
        assert_eq!(names(&c), [Some("a@Mesh"), Some("a@Texture2D"), Some("a")]);
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
}
