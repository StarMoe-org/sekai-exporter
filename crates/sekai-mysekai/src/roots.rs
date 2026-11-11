//! The objects each MySekai form starts from, compiled into every build so that a build without
//! the forms can still name the assets they would have produced.

use sekai_convert::ObjectIndex;

const GAME_OBJECT: i32 = 1;
const MONO_BEHAVIOUR: i32 = 114;

/// The scripts of the facial tables of the MySekai characters.
const FACIAL_TABLES: &[&str] = &[
    "Assembly-CSharp/Sekai.Mysekai.NPCAvatarEyeDataList",
    "Assembly-CSharp/Sekai.Mysekai.NPCAvatarLipSyncDataList",
    "Assembly-CSharp/Sekai.Mysekai.NPCAvatarDefaultFacialDataList",
];

/// The container path ending of the prefab of the field camera.
const FIELD_CAMERA: &str = "/mysekai/camera/fieldcamera.prefab";

const FIXTURE_META: &str = "Assembly-CSharp/Sekai.Mysekai.FixtureBundleMeta";
const FIXTURE_REACTIONS: &str = "Assembly-CSharp/Sekai.Mysekai.FixtureReactionDataSet";

/// The directory of the furniture pieces, each in a directory of its own.
const FIXTURE_MODELS: &str = "/resources/ondemand/mysekai/fixture/";

/// The directory of the packages of the MySekai places.
const SITES: &str = "/resources/ondemand/mysekai/site/";
/// The directory, below [`SITES`], of the map of the places, which is a screen of the interface.
const SITE_MAP: &str = "sitemap/";

/// The package directory of the overhead items of the MySekai characters.
const EMOTICONS: &str = "/mysekai/effect/emoticon/";

/// The roots of the `facial-table` form.
pub fn facial_table(object: &ObjectIndex, script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == MONO_BEHAVIOUR && script.is_some_and(|name| FACIAL_TABLES.contains(&name))
}

/// The roots of the `field-camera` form.
pub fn field_camera(object: &ObjectIndex, _script: Option<&str>, paths: &[String]) -> bool {
    object.class_id == GAME_OBJECT
        && paths
            .iter()
            .any(|path| path.to_ascii_lowercase().ends_with(FIELD_CAMERA))
}

/// The roots of the `fixture-model` form: the prefabs of the furniture pieces, which the container
/// lists as `…/resources/ondemand/mysekai/fixture/<name>/<name>.prefab`.
pub fn fixture_model(object: &ObjectIndex, _script: Option<&str>, paths: &[String]) -> bool {
    object.class_id == GAME_OBJECT && paths.iter().any(|path| is_fixture_model(path))
}

/// Whether `path`, compared without regard to case, is `…/<FIXTURE_MODELS><name>/<name>.prefab`.
fn is_fixture_model(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    let Some((directory, name)) = path
        .strip_suffix(".prefab")
        .and_then(|stem| stem.rsplit_once('/'))
    else {
        return false;
    };
    !name.is_empty()
        && directory
            .strip_suffix(name)
            .is_some_and(|parent| parent.ends_with(FIXTURE_MODELS))
}

/// The roots of the `site-model` form: every prefab of a package of the MySekai places, the map
/// aside. The form writes one asset for each package, from its principal prefab, and none for the
/// other prefabs.
pub fn site_model(object: &ObjectIndex, _script: Option<&str>, paths: &[String]) -> bool {
    object.class_id == GAME_OBJECT && paths.iter().any(|path| is_site_prefab(path))
}

/// Whether `path`, compared without regard to case, is `…<SITES><path>.prefab` outside
/// [`SITE_MAP`].
pub fn is_site_prefab(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.ends_with(".prefab")
        && path
            .split_once(SITES)
            .is_some_and(|(_, rest)| !rest.starts_with(SITE_MAP))
}

/// The roots of the `fixture-meta` form.
pub fn fixture_meta(object: &ObjectIndex, script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == MONO_BEHAVIOUR && script == Some(FIXTURE_META)
}

/// The roots of the `fixture-reactions` form.
pub fn fixture_reactions(object: &ObjectIndex, script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == MONO_BEHAVIOUR && script == Some(FIXTURE_REACTIONS)
}

/// The roots of the `emoticon` form.
pub fn emoticon(object: &ObjectIndex, _script: Option<&str>, paths: &[String]) -> bool {
    object.class_id == GAME_OBJECT
        && paths
            .iter()
            .any(|path| path.to_ascii_lowercase().contains(EMOTICONS))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_object() -> ObjectIndex {
        ObjectIndex {
            path_id: 1,
            class_id: GAME_OBJECT,
            class: String::new(),
            name: None,
            script: None,
            script_name: None,
            references: Vec::new(),
            size: 0,
            sha256: [0; 32],
            error: None,
        }
    }

    #[test]
    fn prefab_root_paths_ignore_ascii_case() {
        let object = game_object();
        let paths = |path: &str| vec![path.to_owned()];
        assert!(field_camera(
            &object,
            None,
            &paths("Assets/MySekai/Camera/FieldCamera.Prefab")
        ));
        assert!(emoticon(
            &object,
            None,
            &paths("Assets/MySekai/Effect/Emoticon/001/001.Prefab")
        ));
    }
}
