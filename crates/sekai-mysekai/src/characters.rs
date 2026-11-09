//! The roster of the characters of MySekai.
//!
//! [`Characters`] produces the `characters` form, specified in `docs/spec/characters.md`.

use std::collections::{BTreeMap, HashMap};

use sekai_convert::tables::{TableContext, TableConverter, TablePlan};
use sekai_convert::{Asset, ConvertError};
use serde::{Deserialize, Serialize};

use crate::scripts::{OneAsset, document, malformed, rows};

const NAME: &str = "mysekai-runtime/characters";

const MOTIONS: &str = "mysekaiCharacterTalkMotions";
const UNITS: &str = "gameCharacterUnits";
const PEOPLE: &str = "gameCharacters";
const SOLO_ACTIONS: &str = "mysekaiCharacterTalkSoloActions";

/// Stored durations and speeds are this many times the seconds and metres per second they mean.
const STORED_PER_UNIT: f64 = 1000.0;

/// The roster of the characters of a version.
#[derive(Debug, Clone, Copy, Default)]
pub struct Characters;

impl TableConverter for Characters {
    fn name(&self) -> &'static str {
        "characters"
    }

    fn version(&self) -> u32 {
        1
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        Ok(Box::new(OneAsset::new(NAME, move || {
            build(&Tables::read(cx)?)
        })))
    }
}

fn build(tables: &Tables) -> Result<Asset, ConvertError> {
    document(&Document {
        version: 1,
        characters: tables.roster()?,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MotionRow {
    game_character_unit_id: u32,
    idle_motion: String,
    walk_motion: String,
    pause_milli_seconds: f64,
    walk_speed: f64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnitRow {
    id: u32,
    game_character_id: u32,
    unit: String,
    color_code: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersonRow {
    id: u32,
    #[serde(default)]
    first_name: String,
    #[serde(default)]
    given_name: String,
    gender: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SoloActionRow {
    game_character_unit_id: u32,
    lua: String,
}

/// The game data tables of the roster, keyed by what they are joined on.
struct Tables {
    motions: Vec<MotionRow>,
    units: HashMap<u32, UnitRow>,
    people: HashMap<u32, PersonRow>,
    solo_actions: HashMap<u32, String>,
}

impl Tables {
    fn read(cx: &dyn TableContext) -> Result<Self, ConvertError> {
        Ok(Self::new(
            rows(cx, MOTIONS)?,
            rows(cx, UNITS)?,
            rows(cx, PEOPLE)?,
            rows(cx, SOLO_ACTIONS)?,
        ))
    }

    fn new(
        motions: Vec<MotionRow>,
        units: Vec<UnitRow>,
        people: Vec<PersonRow>,
        solo_actions: Vec<SoloActionRow>,
    ) -> Self {
        Self {
            motions,
            units: units.into_iter().map(|row| (row.id, row)).collect(),
            people: people.into_iter().map(|row| (row.id, row)).collect(),
            solo_actions: solo_actions
                .into_iter()
                .map(|row| (row.game_character_unit_id, row.lua))
                .collect(),
        }
    }

    /// One character per row of the motion table, by unit.
    fn roster(&self) -> Result<BTreeMap<u32, Character<'_>>, ConvertError> {
        self.motions
            .iter()
            .map(|motion| {
                let character = self.character(motion)?;
                Ok((character.unit_id, character))
            })
            .collect()
    }

    fn character<'t>(&'t self, motion: &'t MotionRow) -> Result<Character<'t>, ConvertError> {
        let unit_id = motion.game_character_unit_id;
        let unit = joined(&self.units, unit_id, UNITS)?;
        let person = joined(&self.people, unit.game_character_id, PEOPLE)?;
        Ok(Character {
            unit_id,
            name: format!("{}{}", person.first_name, person.given_name),
            game_character: GameCharacter {
                id: person.id,
                gender: &person.gender,
            },
            identity: Identity {
                game_character_id: unit.game_character_id,
                unit: &unit.unit,
                color_code: &unit.color_code,
            },
            locomotion: Locomotion {
                idle_motion: &motion.idle_motion,
                walk_motion: &motion.walk_motion,
                pause_seconds: motion.pause_milli_seconds / STORED_PER_UNIT,
                walk_speed_meters_per_second: motion.walk_speed / STORED_PER_UNIT,
            },
            solo_action: self.solo_actions.get(&unit_id).map(String::as_str),
        })
    }
}

/// The row `id` of `table`, which a reference to it requires.
fn joined<'t, T>(by_id: &'t HashMap<u32, T>, id: u32, table: &str) -> Result<&'t T, ConvertError> {
    by_id
        .get(&id)
        .ok_or_else(|| malformed(format!("{table} has no row {id}")))
}

#[derive(Serialize)]
struct Document<'t> {
    version: u32,
    characters: BTreeMap<u32, Character<'t>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Character<'t> {
    unit_id: u32,
    name: String,
    game_character: GameCharacter<'t>,
    identity: Identity<'t>,
    locomotion: Locomotion<'t>,
    solo_action: Option<&'t str>,
}

#[derive(Serialize)]
struct GameCharacter<'t> {
    id: u32,
    gender: &'t str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Identity<'t> {
    game_character_id: u32,
    unit: &'t str,
    color_code: &'t str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Locomotion<'t> {
    idle_motion: &'t str,
    walk_motion: &'t str,
    pause_seconds: f64,
    walk_speed_meters_per_second: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn tables(motions: Value, units: Value, people: Value, solo_actions: Value) -> Tables {
        Tables::new(
            serde_json::from_value(motions).unwrap(),
            serde_json::from_value(units).unwrap(),
            serde_json::from_value(people).unwrap(),
            serde_json::from_value(solo_actions).unwrap(),
        )
    }

    fn motions() -> Value {
        json!([{
            "gameCharacterUnitId": 21,
            "idleMotion": "an_idle",
            "walkMotion": "a_walk",
            "pauseMilliSeconds": 15000,
            "walkSpeed": 400.0
        }])
    }

    fn units() -> Value {
        json!([{ "id": 21, "gameCharacterId": 3, "unit": "piapro", "colorCode": "#33ccbb" }])
    }

    fn people() -> Value {
        json!([{ "id": 3, "givenName": "A name", "gender": "female" }])
    }

    /// The code of the error building the roster of `tables` gives.
    fn error_code(tables: &Tables) -> &'static str {
        tables.roster().err().expect("the roster is an error").code
    }

    #[test]
    fn joins_a_character_and_converts_its_stored_units() {
        let tables = tables(motions(), units(), people(), json!([]));
        let written = serde_json::to_value(tables.roster().unwrap()).unwrap();
        assert_eq!(
            written,
            json!({ "21": {
                "unitId": 21,
                "name": "A name",
                "gameCharacter": { "id": 3, "gender": "female" },
                "identity": { "gameCharacterId": 3, "unit": "piapro", "colorCode": "#33ccbb" },
                "locomotion": {
                    "idleMotion": "an_idle",
                    "walkMotion": "a_walk",
                    "pauseSeconds": 15.0,
                    "walkSpeedMetersPerSecond": 0.4
                },
                "soloAction": null
            } })
        );
    }

    #[test]
    fn a_character_takes_its_solo_action_by_unit() {
        let solo = json!([{ "gameCharacterUnitId": 21, "lua": "an_action" }]);
        let tables = tables(motions(), units(), people(), solo);
        assert_eq!(tables.roster().unwrap()[&21].solo_action, Some("an_action"));
    }

    #[test]
    fn names_join_the_family_and_given_name() {
        let people = json!([{
            "id": 3, "firstName": "Family", "givenName": "Given", "gender": "male"
        }]);
        let tables = tables(motions(), units(), people, json!([]));
        assert_eq!(tables.roster().unwrap()[&21].name, "FamilyGiven");
    }

    #[test]
    fn a_unit_without_its_row_is_malformed() {
        let tables = tables(motions(), json!([]), people(), json!([]));
        assert_eq!(error_code(&tables), "malformed");
    }

    #[test]
    fn a_character_without_its_row_is_malformed() {
        let tables = tables(motions(), units(), json!([]), json!([]));
        assert_eq!(error_code(&tables), "malformed");
    }
}
