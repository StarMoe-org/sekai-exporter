//! The characters who react to each piece of furniture.
//!
//! [`FixtureReactions`] produces the `fixture-reactions` form, specified in
//! `docs/spec/fixture-reactions.md`.

use crate::roots::fixture_reactions;
use sekai_convert::{Asset, Context, ConvertError, Converter, ObjectId, ObjectIndex, Value};
use serde::Serialize;

use crate::scripts::{document, malformed};
use crate::stored::{array, field};

const ROWS: &str = "FixturerRactions";
const FIXTURE: &str = "FixtureId";
const GROUPS: &str = "ReactionCharacter";
const UNITS: &str = "CharacterUnitIds";

#[derive(Serialize)]
struct Document {
    fixtures: Vec<Fixture>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    fixture_id: i64,
    groups: Vec<Vec<i64>>,
}

/// One data set of reactions as a JSON document.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixtureReactions;

impl Converter for FixtureReactions {
    fn name(&self) -> &'static str {
        "fixture-reactions"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        fixture_reactions(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        build(&cx.object(root)?.fields).map(Some)
    }
}

fn build(data_set: &Value) -> Result<Asset, ConvertError> {
    let fixtures = array(field(data_set, ROWS)?, ROWS)?
        .iter()
        .map(fixture)
        .collect::<Result<_, _>>()?;
    document(&Document { fixtures })
}

fn fixture(row: &Value) -> Result<Fixture, ConvertError> {
    Ok(Fixture {
        fixture_id: integer(field(row, FIXTURE)?, FIXTURE)?,
        groups: array(field(row, GROUPS)?, GROUPS)?
            .iter()
            .map(group)
            .collect::<Result<_, _>>()?,
    })
}

fn group(entry: &Value) -> Result<Vec<i64>, ConvertError> {
    array(field(entry, UNITS)?, UNITS)?
        .iter()
        .map(|unit| integer(unit, UNITS))
        .collect()
}

fn integer(value: &Value, source: &str) -> Result<i64, ConvertError> {
    value
        .as_i64()
        .ok_or_else(|| malformed(format!("{source} is not an integer")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripts::testing::{json_of, structure};
    use serde_json::json;

    fn units(ids: &[i64]) -> Value {
        structure(&[(
            UNITS,
            Value::Array(ids.iter().copied().map(Value::Int).collect()),
        )])
    }

    fn row(fixture: i64, groups: Vec<Value>) -> Value {
        structure(&[
            (FIXTURE, Value::Int(fixture)),
            (GROUPS, Value::Array(groups)),
        ])
    }

    #[test]
    fn rows_and_groups_keep_their_stored_order() {
        let data_set = structure(&[(
            ROWS,
            Value::Array(vec![
                row(534, vec![units(&[20, 3]), units(&[1])]),
                row(5, vec![]),
            ]),
        )]);
        assert_eq!(
            json_of(&build(&data_set).unwrap()),
            json!({"fixtures": [
                {"fixtureId": 534, "groups": [[20, 3], [1]]},
                {"fixtureId": 5, "groups": []},
            ]})
        );
    }

    #[test]
    fn a_field_of_the_wrong_type_is_an_error() {
        let data_set = structure(&[(
            ROWS,
            Value::Array(vec![structure(&[
                (FIXTURE, Value::String("five".to_owned())),
                (GROUPS, Value::Array(Vec::new())),
            ])]),
        )]);
        assert_eq!(build(&data_set).unwrap_err().code, "malformed");
    }
}
