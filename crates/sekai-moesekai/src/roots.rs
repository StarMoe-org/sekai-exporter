//! The objects each asset form of this crate starts from, compiled into every build so that a
//! build without a converter can still name the assets it would have produced.

use sekai_convert::ObjectIndex;

const GAME_OBJECT: i32 = 1;
const ANIMATION_CLIP: i32 = 74;
const MONO_BEHAVIOUR: i32 = 114;
/// The script of the scene data of a story.
const SCENE_DATA: &str = "Assembly-CSharp/Sekai.ScenarioSceneData";
/// The script of the master of ceremonies of a virtual live.
const MASTER_OF_CEREMONY_DATA: &str = "Assembly-CSharp/Sekai.Core.VirtualLive.MasterOfCeremonyData";
/// The script of the data of an action set.
const ACTION_SET_DATA: &str = "Assembly-CSharp/Sekai.ActionSetData";
/// The script of the build data of a scenario background.
const SCENARIO_BACKGROUND_DATA: &str = "Assembly-CSharp/Sekai.ScenarioBackgroundBuildData";

/// The roots of the `scenario` form.
pub fn scenario(object: &ObjectIndex, script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == MONO_BEHAVIOUR && script == Some(SCENE_DATA)
}

/// The roots of the `virtual-live-script` form.
pub fn virtual_live_script(object: &ObjectIndex, script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == MONO_BEHAVIOUR && script == Some(MASTER_OF_CEREMONY_DATA)
}

/// The roots of the `action-set` form.
pub fn action_set(object: &ObjectIndex, script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == MONO_BEHAVIOUR && script == Some(ACTION_SET_DATA)
}

/// The roots of the `scenario-background` form.
pub fn scenario_background(object: &ObjectIndex, script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == MONO_BEHAVIOUR && script == Some(SCENARIO_BACKGROUND_DATA)
}

/// The directories of the character models of the 3D screens.
const CHARACTER_MODELS: &[&str] = &["/live_pv/model/character/", "/live_pv/model/characterv2/"];
/// Each part of a costume: its directory below a character model directory, and its file name.
const COSTUME_PARTS: &[(&str, &str)] = &[
    ("body/", "/body.prefab"),
    ("face/", "/face.prefab"),
    ("head_optional/", "/optional.prefab"),
];
/// The directory of the motions of the costume screen.
const COSTUME_MOTIONS: &str = "/character/motion/costume_setting/";

/// The roots of the `costume-model` form: the prefabs of the parts of a costume.
pub fn costume_model(object: &ObjectIndex, _script: Option<&str>, paths: &[String]) -> bool {
    object.class_id == GAME_OBJECT && paths.iter().any(|path| is_costume_part(path))
}

fn is_costume_part(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    CHARACTER_MODELS.iter().any(|models| {
        path.split_once(models).is_some_and(|(_, part)| {
            COSTUME_PARTS
                .iter()
                .any(|(directory, file)| part.starts_with(directory) && part.ends_with(file))
        })
    })
}

/// The roots of the `costume-motion` form: the motions of the costume screen.
pub fn costume_motion(object: &ObjectIndex, _script: Option<&str>, paths: &[String]) -> bool {
    object.class_id == ANIMATION_CLIP
        && paths.iter().any(|path| {
            let path = path.to_ascii_lowercase();
            path.contains(COSTUME_MOTIONS) && path.ends_with(".anim")
        })
}
