//! What the keys of nodes recognize in the components of a game object.

use sekai_convert::Object;

/// The class ID of `MonoBehaviour`.
const MONO_BEHAVIOUR: i32 = 114;

/// Whether `component` is a `MonoBehaviour` of the script whose class name `class` is, as
/// `.FixtureView` names the `FixtureView` class of any namespace.
pub(crate) fn is_script(component: &Object, class: &str) -> bool {
    component.class_id == MONO_BEHAVIOUR
        && component
            .script
            .as_deref()
            .is_some_and(|script| script.ends_with(class) && script.len() > class.len())
}
