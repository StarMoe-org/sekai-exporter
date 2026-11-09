//! The objects each asset form of this crate starts from, compiled into every build so that a
//! build without a converter can still name the assets it would have produced.

use sekai_convert::ObjectIndex;

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
