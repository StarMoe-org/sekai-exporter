//! The objects each asset form starts from.
//!
//! These rules are compiled into every build, so that a build without a converter can still name
//! the assets that converter would have produced: see [`Absent`].

use crate::{Asset, Context, ConvertError, Converter, FileConverter, ObjectId, ObjectIndex};

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
/// The extension of a cue sheet stored as a file.
const CUE_SHEET_FILE: &str = ".acb";
/// The container path ending of a USM file.
pub(crate) const USM: &str = ".usm.bytes";

/// Whether an object is the root of an asset of a form, with the arguments of
/// [`Converter::is_root`].
pub type Rule = fn(&ObjectIndex, Option<&str>, &[String]) -> bool;

/// Whether a file of a bundle is the root of an asset of a form, with the argument of
/// [`FileConverter::is_file_root`].
pub type FileRule = fn(&str) -> bool;

/// `path` without the ending `suffix`, compared without regard to ASCII case, or `None` when it
/// does not end with it.
fn strip_suffix_ignoring_case<'a>(path: &'a str, suffix: &str) -> Option<&'a str> {
    let start = path.len().checked_sub(suffix.len())?;
    let ending = path.get(start..)?;
    ending.eq_ignore_ascii_case(suffix).then(|| &path[..start])
}

/// The `<base>` and number of a container path of the form `<base>-<NNN>.usm.bytes`.
pub(crate) fn segment(path: &str) -> Option<(&str, u32)> {
    let (base, number) = strip_suffix_ignoring_case(path, USM)?.rsplit_once('-')?;
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

/// The file roots of the `audio` form: the cue sheets a bundle stores as files.
pub fn audio_file(path: &str) -> bool {
    strip_suffix_ignoring_case(path, CUE_SHEET_FILE).is_some()
}

/// Stands for a converter that is not compiled into this build. Each asset the converter would
/// have produced fails with the error `not_compiled`, which names the converter.
#[derive(Debug, Clone, Copy)]
pub struct Absent {
    name: &'static str,
    rule: Rule,
    file_rule: Option<FileRule>,
}

impl Absent {
    /// The converter called `name`, whose roots `rule` selects.
    pub fn boxed(name: &'static str, rule: Rule) -> Box<dyn Converter> {
        Box::new(Self {
            name,
            rule,
            file_rule: None,
        })
    }

    /// The converter called `name`, whose roots `rule` selects among objects and `file_rule`
    /// among files.
    pub fn with_files(name: &'static str, rule: Rule, file_rule: FileRule) -> Box<dyn Converter> {
        Box::new(Self {
            name,
            rule,
            file_rule: Some(file_rule),
        })
    }

    fn not_compiled(&self) -> ConvertError {
        let message = format!(
            "the converter {} is not compiled into this build",
            self.name
        );
        ConvertError::new("not_compiled", message)
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
        Err(self.not_compiled())
    }

    fn files(&self) -> Option<&dyn FileConverter> {
        let files: &dyn FileConverter = self;
        self.file_rule.is_some().then_some(files)
    }
}

impl FileConverter for Absent {
    fn is_file_root(&self, path: &str) -> bool {
        self.file_rule.is_some_and(|rule| rule(path))
    }

    fn convert_file(&self, _bytes: &[u8]) -> Result<Option<Asset>, ConvertError> {
        Err(self.not_compiled())
    }
}
