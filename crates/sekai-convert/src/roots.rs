//! The objects each asset form starts from.
//!
//! These rules are compiled into every build, so that a build without a converter can still name
//! the assets that converter would have produced: see [`Absent`].

use crate::{Asset, Context, ConvertError, Converter, ObjectId, ObjectIndex};

const TEXT_ASSET: i32 = 49;
const TEXTURE_2D: i32 = 28;
const CUBEMAP: i32 = 89;
pub(crate) const TEXTURE_3D: i32 = 117;
pub(crate) const TEXTURE_2D_ARRAY: i32 = 187;
const SPRITE: i32 = 213;
const FONT: i32 = 128;
const MONO_BEHAVIOUR: i32 = 114;

/// The script of the build data of a Live2D model.
const LIVE2D_MODEL_DATA: &str = "Assembly-CSharp/Sekai.Live2D.Live2DBuildModelData";
/// The script of the build data of a set of Live2D motions.
const LIVE2D_MOTION_DATA: &str = "Assembly-CSharp/Sekai.Live2D.Live2DBuildMotionData";

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
const GAME_OBJECT: i32 = 1;

/// The container path ending of a cue sheet.
const CUE_SHEET: &str = ".acb.bytes";
/// The container path ending of a USM file.
pub(crate) const USM: &str = ".usm.bytes";

/// Whether an object is the root of an asset of a form, with the arguments of
/// [`Converter::is_root`].
pub type Rule = fn(&ObjectIndex, Option<&str>, &[String]) -> bool;

/// The `<base>` and number of a container path of the form `<base>-<NNN>.usm.bytes`.
pub(crate) fn segment(path: &str) -> Option<(&str, u32)> {
    let (base, number) = path.strip_suffix(USM)?.rsplit_once('-')?;
    if number.len() != 3 || !number.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((base, number.parse().ok()?))
}

/// The roots of the `texture` form.
pub fn texture(object: &ObjectIndex, _script: Option<&str>, _paths: &[String]) -> bool {
    matches!(
        object.class_id,
        TEXTURE_2D | CUBEMAP | TEXTURE_3D | TEXTURE_2D_ARRAY
    )
}

/// The roots of the `sprite` form.
pub fn sprite(object: &ObjectIndex, _script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == SPRITE
}

/// The roots of the `audio` form.
pub fn audio(object: &ObjectIndex, _script: Option<&str>, paths: &[String]) -> bool {
    object.class_id == TEXT_ASSET
        && paths
            .iter()
            .any(|path| path.to_ascii_lowercase().ends_with(CUE_SHEET))
}

/// The roots of the `video` form. A video split into segments starts from its first one.
pub fn video(object: &ObjectIndex, _script: Option<&str>, paths: &[String]) -> bool {
    object.class_id == TEXT_ASSET
        && paths
            .iter()
            .any(|path| path.ends_with(USM) && segment(path).is_none_or(|(_, number)| number == 1))
}

/// The roots of the `text` form: text assets that do not hold CRI audio or video.
pub fn text(object: &ObjectIndex, _script: Option<&str>, paths: &[String]) -> bool {
    object.class_id == TEXT_ASSET
        && !paths.iter().any(|path| {
            let path = path.to_ascii_lowercase();
            path.ends_with(CUE_SHEET) || path.ends_with(USM)
        })
}

/// The roots of the `font` form.
pub fn font(object: &ObjectIndex, _script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == FONT
}

/// The roots of the `facial-table` form.
pub fn facial_table(object: &ObjectIndex, script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == MONO_BEHAVIOUR && script.is_some_and(|name| FACIAL_TABLES.contains(&name))
}

/// The roots of the `field-camera` form.
pub fn field_camera(object: &ObjectIndex, _script: Option<&str>, paths: &[String]) -> bool {
    object.class_id == GAME_OBJECT && paths.iter().any(|path| path.ends_with(FIELD_CAMERA))
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
    object.class_id == GAME_OBJECT && paths.iter().any(|path| path.contains(EMOTICONS))
}

/// The roots of the `live2d-model` form.
pub fn live2d_model(object: &ObjectIndex, script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == MONO_BEHAVIOUR && script == Some(LIVE2D_MODEL_DATA)
}

/// The roots of the `live2d-motions` form.
pub fn live2d_motions(object: &ObjectIndex, script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == MONO_BEHAVIOUR && script == Some(LIVE2D_MOTION_DATA)
}

/// Stands for a converter that is not compiled into this build. Each asset the converter would
/// have produced fails with the error `not_compiled`, which names the converter.
#[derive(Debug, Clone, Copy)]
pub struct Absent {
    name: &'static str,
    rule: Rule,
}

impl Absent {
    /// The converter called `name`, whose roots `rule` selects.
    pub fn boxed(name: &'static str, rule: Rule) -> Box<dyn Converter> {
        Box::new(Self { name, rule })
    }
}

impl Converter for Absent {
    fn name(&self) -> &'static str {
        self.name
    }

    /// Lower than the version of any converter, so no result of a real converter is reused.
    fn version(&self) -> u32 {
        0
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        (self.rule)(object, script, container_paths)
    }

    fn convert(&self, _root: ObjectId, _cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        Err(ConvertError::new(
            "not_compiled",
            format!(
                "the converter {} is not compiled into this build",
                self.name
            ),
        ))
    }
}
