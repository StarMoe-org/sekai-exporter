//! MySekai sites, furniture and phenomena in the full form a runtime that reproduces the game needs.
//!
//! The forms are compiled with the `forms` feature. The rules in [`roots`] that name the objects
//! each form starts from are always compiled, so a build without the forms still names their
//! assets.

#[cfg(feature = "forms")]
pub mod alone;
#[cfg(feature = "forms")]
pub mod characters;
#[cfg(feature = "forms")]
pub mod client_config;
#[cfg(feature = "forms")]
pub mod communication;
#[cfg(feature = "forms")]
mod constants;
#[cfg(feature = "forms")]
pub mod emoticon;
#[cfg(feature = "forms")]
pub mod facial;
#[cfg(feature = "forms")]
pub mod field_camera;
#[cfg(feature = "forms")]
pub mod fixture_areas;
#[cfg(feature = "forms")]
pub mod fixture_attach_points;
#[cfg(feature = "forms")]
pub mod fixture_model;
#[cfg(feature = "forms")]
pub mod fixture_reactions;
#[cfg(feature = "forms")]
pub mod fixtures;
#[cfg(feature = "forms")]
mod lua;
#[cfg(feature = "forms")]
mod model_keys;
#[cfg(feature = "forms")]
mod particle;
#[cfg(feature = "forms")]
mod performance;
pub mod roots;
#[cfg(feature = "forms")]
mod scripts;
#[cfg(feature = "forms")]
pub mod site_model;
#[cfg(feature = "forms")]
mod stored;
#[cfg(feature = "forms")]
mod talk_script;
#[cfg(feature = "forms")]
pub mod talks;
