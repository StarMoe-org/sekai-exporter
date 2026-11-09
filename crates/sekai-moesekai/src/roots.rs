//! The objects each asset form of this crate starts from, compiled into every build so that a
//! build without a converter can still name the assets it would have produced.

use sekai_convert::ObjectIndex;

const MONO_BEHAVIOUR: i32 = 114;
/// The script of the scene data of a story.
const SCENE_DATA: &str = "Assembly-CSharp/Sekai.ScenarioSceneData";

/// The roots of the `scenario` form.
pub fn scenario(object: &ObjectIndex, script: Option<&str>, _paths: &[String]) -> bool {
    object.class_id == MONO_BEHAVIOUR && script == Some(SCENE_DATA)
}
