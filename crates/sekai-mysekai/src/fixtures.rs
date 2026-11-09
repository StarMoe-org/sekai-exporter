//! The furniture of MySekai: what each piece is, where it is put and what it does.
//!
//! [`Fixtures`] produces the `mysekai-fixtures` form, specified in
//! `docs/spec/mysekai-fixtures.md`.

use std::collections::BTreeSet;

use sekai_convert::tables::{TableContext, TableConverter, TablePlan};
use sekai_convert::{Asset, ConvertError};
use serde::{Deserialize, Serialize};

use crate::scripts::{OneAsset, document, malformed, rows};

const NAME: &str = "mysekai-runtime/mysekai-fixtures";
const TABLE: &str = "mysekaiFixtures";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    id: i64,
    name: String,
    flavor_text: String,
    grid_size: GridSize,
    #[serde(rename = "mysekaiFixtureType")]
    fixture_type: FixtureType,
    #[serde(rename = "mysekaiFixturePlayerActionType")]
    player_action_type: PlayerActionType,
    #[serde(rename = "mysekaiFixturePutType")]
    put_type: String,
    #[serde(rename = "mysekaiFixtureHandleType")]
    handle_type: String,
    #[serde(rename = "mysekaiFixtureMainGenreId")]
    main_genre_id: i64,
    #[serde(rename = "mysekaiFixtureSubGenreId")]
    sub_genre_id: Option<i64>,
    #[serde(rename = "mysekaiSettableLayoutType")]
    layout_type: String,
    #[serde(rename = "mysekaiFixtureAnotherColors")]
    another_colors: Vec<AnotherColor>,
    #[serde(rename = "mysekaiFixturePutSoundId")]
    put_sound_id: i64,
    assetbundle_name: String,
    first_put_cost: Option<i64>,
    second_put_cost: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GridSize {
    width: i64,
    depth: i64,
    height: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AnotherColor {
    texture_id: i64,
}

/// What a piece is.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum FixtureType {
    System,
    Custom,
    Plant,
    HousePlant,
    SurfaceAppearance,
    Gate,
    Normal,
    Canvas,
}

impl FixtureType {
    fn number(&self) -> i64 {
        match self {
            Self::System => 0,
            Self::Custom => 1,
            Self::Plant => 2,
            Self::HousePlant => 3,
            Self::SurfaceAppearance => 4,
            Self::Gate => 5,
            Self::Normal => 6,
            Self::Canvas => 7,
        }
    }
}

/// What a placed piece offers.
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum PlayerActionType {
    NoAction,
    Loop,
    Timeline,
    OneShot,
}

impl PlayerActionType {
    fn number(&self) -> i64 {
        match self {
            Self::NoAction => 0,
            Self::Loop => 1,
            Self::Timeline => 2,
            Self::OneShot => 3,
        }
    }
}

#[derive(Serialize)]
struct Document<'f> {
    version: u32,
    fixtures: &'f [Fixture],
}

/// One piece of furniture.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Fixture {
    assetbundle_name: String,
    color_ids: Vec<i64>,
    description: String,
    first_put_cost: Option<i64>,
    fixture_type: FixtureType,
    fixture_type_value: i64,
    grid_depth: i64,
    grid_height: i64,
    grid_width: i64,
    handle_type: String,
    id: i64,
    layout_type: String,
    main_genre_id: i64,
    name: String,
    player_action_type: PlayerActionType,
    player_action_type_value: i64,
    put_sound_id: i64,
    put_type: String,
    second_put_cost: Option<i64>,
    sub_genre_id: Option<i64>,
}

impl From<Row> for Fixture {
    fn from(row: Row) -> Self {
        Self {
            fixture_type_value: row.fixture_type.number(),
            player_action_type_value: row.player_action_type.number(),
            color_ids: color_ids(row.another_colors),
            assetbundle_name: row.assetbundle_name,
            description: row.flavor_text,
            first_put_cost: row.first_put_cost,
            fixture_type: row.fixture_type,
            grid_depth: row.grid_size.depth,
            grid_height: row.grid_size.height,
            grid_width: row.grid_size.width,
            handle_type: row.handle_type,
            id: row.id,
            layout_type: row.layout_type,
            main_genre_id: row.main_genre_id,
            name: row.name,
            player_action_type: row.player_action_type,
            put_sound_id: row.put_sound_id,
            put_type: row.put_type,
            second_put_cost: row.second_put_cost,
            sub_genre_id: row.sub_genre_id,
        }
    }
}

/// The furniture of the version.
#[derive(Debug, Clone, Copy, Default)]
pub struct Fixtures;

impl TableConverter for Fixtures {
    fn name(&self) -> &'static str {
        "mysekai-fixtures"
    }

    fn version(&self) -> u32 {
        1
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        Ok(Box::new(OneAsset::new(NAME, move || {
            build(&sorted(rows(cx, TABLE)?))
        })))
    }
}

/// The entries of `table` in id order.
fn sorted(table: Vec<Row>) -> Vec<Fixture> {
    let mut fixtures: Vec<Fixture> = table.into_iter().map(Fixture::from).collect();
    fixtures.sort_by_key(|fixture| fixture.id);
    fixtures
}

/// The document of `fixtures`, which are in id order; two rows of one id are an error.
fn build(fixtures: &[Fixture]) -> Result<Asset, ConvertError> {
    if let Some(pair) = fixtures.windows(2).find(|pair| pair[0].id == pair[1].id) {
        return Err(malformed(format!(
            "fixtures {} and {} have the same id",
            pair[0].name, pair[1].name
        )));
    }
    document(&Document {
        version: 1,
        fixtures,
    })
}

/// The colour cells: the texture of each other colour above 0, then 1.
fn color_ids(colors: Vec<AnotherColor>) -> Vec<i64> {
    let textures = colors
        .into_iter()
        .filter(|color| color.texture_id > 0)
        .map(|color| color.texture_id);
    BTreeSet::from_iter(textures.chain([1]))
        .into_iter()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A synthetic row with every column the form reads.
    fn row(id: i64, name: &str) -> serde_json::Value {
        json!({
            "id": id,
            "name": name,
            "pronunciation": "a reading",
            "flavorText": "a description",
            "seq": 7,
            "gridSize": { "width": 2, "depth": 3, "height": 4 },
            "mysekaiFixtureType": "normal",
            "mysekaiFixturePlayerActionType": "no_action",
            "mysekaiFixturePutType": "put_base",
            "mysekaiFixtureHandleType": "block",
            "mysekaiFixtureMainGenreId": 10,
            "mysekaiFixtureSubGenreId": null,
            "mysekaiSettableLayoutType": "floor",
            "mysekaiFixtureAnotherColors": [
                { "textureId": 3, "colorCode": "#000000" },
                { "textureId": 2, "colorCode": "#ffffff" },
                { "textureId": 0, "colorCode": "#000000" }
            ],
            "mysekaiFixturePutSoundId": 5,
            "assetbundleName": "a_bundle",
            "firstPutCost": 12,
            "secondPutCost": 0
        })
    }

    /// The row `id` with the word `word` in the column `key`.
    fn row_with(id: i64, word: &str, key: &str) -> serde_json::Value {
        let mut row = row(id, "a name");
        row[key] = json!(word);
        row
    }

    fn table(rows: serde_json::Value) -> Vec<Row> {
        serde_json::from_value(rows).unwrap()
    }

    fn document(rows: Vec<Row>) -> serde_json::Value {
        let asset = build(&sorted(rows)).unwrap();
        let [member] = asset.members.as_slice() else {
            panic!("{asset:?}");
        };
        let sekai_convert::Content::File { bytes, .. } = &member.content else {
            panic!("the document is not a file");
        };
        serde_json::from_slice(bytes).unwrap()
    }

    #[test]
    fn a_row_becomes_the_entry_its_readers_take() {
        let document = document(table(json!([row(3, "a name")])));
        assert_eq!(document["version"], 1);
        assert_eq!(
            document["fixtures"][0],
            json!({
                "assetbundleName": "a_bundle",
                "colorIds": [1, 2, 3],
                "description": "a description",
                "firstPutCost": 12,
                "fixtureType": "normal",
                "fixtureTypeValue": 6,
                "gridDepth": 3,
                "gridHeight": 4,
                "gridWidth": 2,
                "handleType": "block",
                "id": 3,
                "layoutType": "floor",
                "mainGenreId": 10,
                "name": "a name",
                "playerActionType": "no_action",
                "playerActionTypeValue": 0,
                "putSoundId": 5,
                "putType": "put_base",
                "secondPutCost": 0,
                "subGenreId": null
            })
        );
    }

    #[test]
    fn a_version_without_placement_costs_writes_null() {
        let mut row = row(1, "a name");
        row.as_object_mut().unwrap().remove("firstPutCost");
        row.as_object_mut().unwrap().remove("secondPutCost");
        let document = document(table(json!([row])));
        assert_eq!(document["fixtures"][0]["firstPutCost"], json!(null));
        assert_eq!(document["fixtures"][0]["secondPutCost"], json!(null));
    }

    #[test]
    fn each_kind_and_action_takes_its_number() {
        let kinds = [
            ("system", 0),
            ("custom", 1),
            ("plant", 2),
            ("house_plant", 3),
            ("surface_appearance", 4),
            ("gate", 5),
            ("normal", 6),
            ("canvas", 7),
        ];
        let actions = [
            ("no_action", 0),
            ("loop", 1),
            ("timeline", 2),
            ("one_shot", 3),
        ];
        let mut entries = Vec::new();
        for (id, (word, _)) in (1..).zip(kinds) {
            entries.push(row_with(id, word, "mysekaiFixtureType"));
        }
        for (id, (word, _)) in (101..).zip(actions) {
            entries.push(row_with(id, word, "mysekaiFixturePlayerActionType"));
        }
        let document = document(table(json!(entries)));
        let entries = document["fixtures"].as_array().unwrap();
        for (entry, (word, number)) in entries.iter().zip(kinds) {
            assert_eq!(entry["fixtureType"], word);
            assert_eq!(entry["fixtureTypeValue"], number);
        }
        for (entry, (word, number)) in entries.iter().skip(kinds.len()).zip(actions) {
            assert_eq!(entry["playerActionType"], word);
            assert_eq!(entry["playerActionTypeValue"], number);
        }
    }

    #[test]
    fn the_fixtures_are_in_id_order() {
        let document = document(table(json!([row(5, "late"), row(2, "early")])));
        assert_eq!(document["fixtures"][0]["id"], 2);
        assert_eq!(document["fixtures"][1]["id"], 5);
    }

    #[test]
    fn a_word_the_form_does_not_know_is_an_error() {
        let unknown_kind = row_with(1, "a_thing", "mysekaiFixtureType");
        assert!(serde_json::from_value::<Row>(unknown_kind).is_err());

        let unknown_action = row_with(1, "sometimes", "mysekaiFixturePlayerActionType");
        assert!(serde_json::from_value::<Row>(unknown_action).is_err());
    }

    #[test]
    fn a_row_without_a_column_it_must_give_is_an_error() {
        for column in [
            "id",
            "name",
            "gridSize",
            "mysekaiFixtureType",
            "mysekaiSettableLayoutType",
            "mysekaiFixtureAnotherColors",
            "assetbundleName",
        ] {
            let mut incomplete = row(1, "a name");
            incomplete.as_object_mut().unwrap().remove(column);
            assert!(
                serde_json::from_value::<Row>(incomplete).is_err(),
                "{column}"
            );
        }
    }

    #[test]
    fn two_rows_of_one_id_are_an_error() {
        let rows = table(json!([row(1, "one"), row(1, "other")]));
        assert!(build(&sorted(rows)).is_err());
    }
}
