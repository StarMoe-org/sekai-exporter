//! The documents of the `action-set` and `scenario-background` forms, specified in
//! `docs/spec/action-set.md` and `docs/spec/scenario-background.md`.

use serde::{Deserialize, Serialize};

/// The value of the `format` key of an action set.
pub const ACTION_SET_FORMAT: &str = "action-set";
/// The value of the `format` key of a scenario background.
pub const SCENARIO_BACKGROUND_FORMAT: &str = "scenario-background";
/// The version of both document formats.
pub const VERSION: u32 = 1;

/// The area actors of an action set and the costumes they wear.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionSet {
    /// Always [`ACTION_SET_FORMAT`].
    pub format: String,
    /// The version of the document format.
    pub version: u32,
    /// The area of the action set.
    pub action_set_id: i64,
    /// The area the action set is shown in.
    pub area_id: i64,
    /// The area actors, in the order the action set lists them.
    pub details: Vec<ActionSetDetail>,
}

impl ActionSet {
    /// The bytes of the document.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// Reads a document.
    pub fn from_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

/// One area actor of an action set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionSetDetail {
    /// The area actor.
    pub area_character_id: i64,
    /// The group of 2D costumes the area actor wears.
    pub costume2d_group_id: i64,
}

/// The movies a scenario background plays, in the order the background reaches them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScenarioBackground {
    /// Always [`SCENARIO_BACKGROUND_FORMAT`].
    pub format: String,
    /// The version of the document format.
    pub version: u32,
    /// The names of the movies the background plays.
    pub movies: Vec<String>,
}

impl ScenarioBackground {
    /// The bytes of the document.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// Reads a document.
    pub fn from_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action_set() -> ActionSet {
        ActionSet {
            format: ACTION_SET_FORMAT.to_owned(),
            version: VERSION,
            action_set_id: 7001,
            area_id: 12,
            details: vec![
                ActionSetDetail {
                    area_character_id: 3,
                    costume2d_group_id: 31,
                },
                ActionSetDetail {
                    area_character_id: 1,
                    costume2d_group_id: -2,
                },
            ],
        }
    }

    fn background() -> ScenarioBackground {
        ScenarioBackground {
            format: SCENARIO_BACKGROUND_FORMAT.to_owned(),
            version: VERSION,
            movies: vec!["backdrop_loop".to_owned(), "backdrop_b".to_owned()],
        }
    }

    #[test]
    fn the_action_set_bytes_are_fixed() {
        assert_eq!(
            String::from_utf8(action_set().to_bytes().unwrap()).unwrap(),
            r#"{"format":"action-set","version":1,"action_set_id":7001,"area_id":12,"details":[{"area_character_id":3,"costume2d_group_id":31},{"area_character_id":1,"costume2d_group_id":-2}]}"#
        );
    }

    #[test]
    fn an_action_set_reads_back() {
        assert_eq!(
            ActionSet::from_bytes(&action_set().to_bytes().unwrap()).unwrap(),
            action_set()
        );
    }

    #[test]
    fn the_scenario_background_bytes_are_fixed() {
        assert_eq!(
            String::from_utf8(background().to_bytes().unwrap()).unwrap(),
            r#"{"format":"scenario-background","version":1,"movies":["backdrop_loop","backdrop_b"]}"#
        );
    }

    #[test]
    fn a_scenario_background_reads_back() {
        assert_eq!(
            ScenarioBackground::from_bytes(&background().to_bytes().unwrap()).unwrap(),
            background()
        );
    }
}
