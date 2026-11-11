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

/// The container path ending of a cue sheet.
const CUE_SHEET: &str = ".acb.bytes";
/// The container path ending of a USM file.
pub(crate) const USM: &str = ".usm.bytes";

/// Whether an object is the root of an asset of a form, with the arguments of
/// [`Converter::is_root`].
pub type Rule = fn(&ObjectIndex, Option<&str>, &[String]) -> bool;

/// The `<base>` and number of a container path of the form `<base>-<NNN>.usm.bytes`.
pub(crate) fn segment(path: &str) -> Option<(&str, u32)> {
    if path.len() < USM.len() || !path[path.len() - USM.len()..].eq_ignore_ascii_case(USM) {
        return None;
    }
    let (base, number) = path[..path.len() - USM.len()].rsplit_once('-')?;
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
        && paths.iter().any(|path| {
            let lower = path.to_ascii_lowercase();
            lower.ends_with(USM) && segment(&lower).is_none_or(|(_, number)| number == 1)
        })
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
