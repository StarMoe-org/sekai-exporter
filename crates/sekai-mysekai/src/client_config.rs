//! The configuration a client reads by numeric ID.
//!
//! [`ClientConfig`] produces the `client-config` form, specified in `docs/spec/client-config.md`.

use std::collections::BTreeMap;
use std::fmt::Display;
use std::str::FromStr;

use sekai_convert::tables::{TableContext, TableConverter, TablePlan};
use sekai_convert::{Asset, ConvertError};
use serde::{Deserialize, Serialize};

use crate::scripts::{OneAsset, document, malformed, rows};

const NAME: &str = "mysekai-runtime/client-config";
const TABLE: &str = "clientConfigs";

#[derive(Deserialize)]
struct Row {
    id: i32,
    #[serde(rename = "type")]
    kind: String,
    value: String,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "PascalCase")]
struct Configs {
    float_configs: BTreeMap<i32, f64>,
    int_configs: BTreeMap<i32, i64>,
    string_configs: BTreeMap<i32, String>,
    bool_configs: BTreeMap<i32, bool>,
}

/// The configuration table of a version, typed.
#[derive(Debug, Clone, Copy, Default)]
pub struct ClientConfig;

impl TableConverter for ClientConfig {
    fn name(&self) -> &'static str {
        "client-config"
    }

    fn version(&self) -> u32 {
        1
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        Ok(Box::new(OneAsset::new(NAME, move || {
            build(rows(cx, TABLE)?)
        })))
    }
}

fn build(table: Vec<Row>) -> Result<Asset, ConvertError> {
    let mut configs = Configs::default();
    for row in table {
        match row.kind.as_str() {
            "Float" => insert(&mut configs.float_configs, row.id, parsed(&row)?)?,
            "Int" => insert(&mut configs.int_configs, row.id, parsed(&row)?)?,
            "Bool" => insert(&mut configs.bool_configs, row.id, boolean(&row)?)?,
            "String" => insert(&mut configs.string_configs, row.id, row.value)?,
            other => return Err(malformed(format!("config {}: type {other}", row.id))),
        }
    }
    document(&configs)
}

fn insert<V>(configs: &mut BTreeMap<i32, V>, id: i32, value: V) -> Result<(), ConvertError> {
    match configs.insert(id, value) {
        None => Ok(()),
        Some(_) => Err(malformed(format!("config {id} is repeated"))),
    }
}

/// The value of a row of the type `Bool`, which the tables of some regions write in capitals.
fn boolean(row: &Row) -> Result<bool, ConvertError> {
    match row.value.to_ascii_lowercase().as_str() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(malformed(format!("config {}: {other}", row.id))),
    }
}

/// The value of `row`, read as its type.
fn parsed<T>(row: &Row) -> Result<T, ConvertError>
where
    T: FromStr,
    T::Err: Display,
{
    row.value
        .parse()
        .map_err(|e| malformed(format!("config {}: {e}", row.id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripts::testing::{FakeVersion, json_of};
    use serde_json::json;

    fn converted(table: serde_json::Value) -> Result<serde_json::Value, ConvertError> {
        let version = FakeVersion::default().with_table(TABLE, table);
        let plan = ClientConfig.plan(&version)?;
        Ok(json_of(&plan.convert(0)?.asset))
    }

    fn row(id: i64, kind: &str, value: &str) -> serde_json::Value {
        json!({"id": id, "type": kind, "value": value})
    }

    #[test]
    fn each_row_lands_in_the_object_of_its_type() {
        let value = converted(json!([
            row(10, "Int", "16"),
            row(2, "Int", "-1"),
            row(3, "Float", "0.003"),
            row(4, "String", "999_garden"),
            row(5, "Bool", "true"),
        ]))
        .unwrap();
        assert_eq!(
            value,
            json!({
                "FloatConfigs": {"3": 0.003},
                "IntConfigs": {"2": -1, "10": 16},
                "StringConfigs": {"4": "999_garden"},
                "BoolConfigs": {"5": true},
            })
        );
    }

    #[test]
    fn a_boolean_may_be_in_capitals() {
        let value = converted(json!([row(1, "Bool", "TRUE"), row(2, "Bool", "FALSE")])).unwrap();
        assert_eq!(value["BoolConfigs"], json!({"1": true, "2": false}));
    }

    #[test]
    fn one_id_may_serve_two_types() {
        let value = converted(json!([row(1, "Int", "1"), row(1, "Bool", "false")])).unwrap();
        assert_eq!(value["IntConfigs"]["1"], 1);
        assert_eq!(value["BoolConfigs"]["1"], false);
    }

    #[test]
    fn a_row_that_does_not_read_is_an_error() {
        for table in [
            json!([row(1, "Vector", "1")]),
            json!([row(1, "Int", "1.5")]),
            json!([row(1, "Float", "fast")]),
            json!([row(1, "Bool", "1")]),
            json!([row(1, "Int", "1"), row(1, "Int", "2")]),
        ] {
            assert_eq!(converted(table).unwrap_err().code, "malformed");
        }
    }
}
