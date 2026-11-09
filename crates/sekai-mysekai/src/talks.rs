//! The talks of the characters: with the player, with each other, at parties, in the tutorial and
//! at furniture.
//!
//! [`Talks`] produces the `talks` form, specified in `docs/spec/talks.md`, and [`FixtureTalks`] the
//! `fixture-talks` form, specified in `docs/spec/fixture-talks.md`.

use std::collections::{BTreeMap, BTreeSet};

use sekai_convert::tables::{TableContext, TableConverter, TablePlan};
use sekai_convert::{Asset, ConvertError};
use serde::{Deserialize, Serialize};

use crate::scripts::{NoAssets, OneAsset, document, malformed, rows};
use crate::talk_script::{TalkReader, TalkScript};

const NAME: &str = "mysekai-runtime/talks";
const FIXTURE_TALKS_NAME: &str = "mysekai-runtime/fixture-talks";

const TALKS: &str = "mysekaiCharacterTalks";
const UNIT_GROUPS: &str = "mysekaiGameCharacterUnitGroups";
const CONDITIONS: &str = "mysekaiCharacterTalkConditions";
const CONDITION_GROUPS: &str = "mysekaiCharacterTalkConditionGroups";
const PRE_ACTIONS: &str = "mysekaiCharacterTalkPreActions";
const TWEETS: &str = "mysekaiCharacterTalkTweets";
const SOME_CHARACTER_TALKS: &str = "mysekaiCharacterTalkSomeCharacterTalks";
const BIRTHDAY_SCENARIOS: &str = "birthdayPartyScenarios";
const TUTORIAL_TALKS: &str = "mysekaiTutorialTalks";
const NO_TALK_FIXTURE_ACTIONS: &str = "mysekaiCharacterTalkNoTalkMysekaiFixtureActions";

/// The conditions that leave a talk of one character to the talks of furniture.
const FURNITURE_CONDITIONS: [&str; 3] = [
    "mysekai_fixture_id",
    "mysekai_fixture_tag_id",
    "after_set_fixture",
];
/// The condition that anchors a talk to a piece of furniture.
const FIXTURE_ANCHOR: [&str; 1] = ["mysekai_fixture_id"];

#[derive(Deserialize)]
struct TalkRow {
    id: i64,
    lua: String,
    #[serde(rename = "mysekaiSiteGroupId")]
    site_group_id: i64,
    #[serde(rename = "mysekaiCharacterTalkTermId")]
    term_id: i64,
    #[serde(rename = "mysekaiCharacterTalkConditionGroupId")]
    condition_group_id: i64,
    #[serde(rename = "mysekaiGameCharacterUnitGroupId")]
    unit_group_id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnitGroupRow {
    id: i64,
    game_character_unit_id1: Option<i64>,
    game_character_unit_id2: Option<i64>,
    game_character_unit_id3: Option<i64>,
    game_character_unit_id4: Option<i64>,
    game_character_unit_id5: Option<i64>,
}

impl UnitGroupRow {
    /// The characters in the slots of the group that hold one, in slot order.
    fn members(&self) -> Vec<i64> {
        [
            self.game_character_unit_id1,
            self.game_character_unit_id2,
            self.game_character_unit_id3,
            self.game_character_unit_id4,
            self.game_character_unit_id5,
        ]
        .into_iter()
        .flatten()
        .filter(|unit| *unit != 0)
        .collect()
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct Condition {
    #[serde(skip_serializing)]
    id: i64,
    #[serde(rename(
        deserialize = "mysekaiCharacterTalkConditionType",
        serialize = "conditionType"
    ))]
    kind: String,
    #[serde(rename(
        deserialize = "mysekaiCharacterTalkConditionTypeValue",
        serialize = "conditionTypeValue"
    ))]
    value: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConditionGroupRow {
    group_id: i64,
    mysekai_character_talk_condition_id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreActionRow {
    mysekai_character_talk_id: i64,
    mysekai_character_talk_tweet_id: i64,
}

#[derive(Debug, Deserialize, Serialize)]
struct Tweet {
    id: i64,
    text: String,
    #[serde(rename(deserialize = "motionName", serialize = "motion"))]
    motion: Option<String>,
    #[serde(rename(deserialize = "expressionEyeName", serialize = "eye"))]
    eye: Option<String>,
    #[serde(rename(deserialize = "expressionMouthName", serialize = "mouth"))]
    mouth: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SomeCharacterLink {
    id: i64,
    mysekai_character_talk_id: i64,
    main_game_character_unit_id: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct NoTalkRow {
    mysekai_fixture_id: i64,
    game_character_unit_id: i64,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct BirthdayScenario {
    id: i64,
    birthday_party_id: i64,
    seq: i64,
    lua: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct TutorialRow {
    id: i64,
    assetbundle_name: String,
    lua: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    mysekai_character_talk_tweet_id: Option<i64>,
}

#[derive(Serialize)]
struct FixtureTalksDocument<'t> {
    version: u32,
    talks: Vec<FixtureTalk<'t>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FixtureTalk<'t> {
    talk_id: i64,
    lua: &'t str,
    form: u8,
    site_group_id: i64,
    term_id: i64,
    condition_group_id: i64,
    fixture_ids: Vec<i64>,
    unit_ids: &'t [i64],
    pairs: Vec<(i64, i64)>,
    tweet: Option<&'t Tweet>,
    #[serde(flatten)]
    script: TalkScript,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Document<'t> {
    version: u32,
    units: BTreeMap<String, UnitTalks<'t>>,
    birthday_party: BirthdayParty<'t>,
    common_some_character: CommonTalks<'t>,
    tutorial: Tutorial<'t>,
}

#[derive(Default, Serialize)]
struct UnitTalks<'t> {
    talks: Vec<Talk<'t>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Talk<'t> {
    talk_id: i64,
    lua: &'t str,
    site_group_id: i64,
    term_id: i64,
    conditions: Vec<&'t str>,
    condition_values: Vec<&'t Condition>,
    tweet: Option<&'t Tweet>,
    #[serde(flatten)]
    script: TalkScript,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SharedTalk<'t> {
    #[serde(flatten)]
    talk: Talk<'t>,
    condition_group_id: i64,
    unit_ids: &'t [i64],
    some_character_talks: &'t [SomeCharacterLink],
}

#[derive(Serialize)]
struct CommonTalks<'t> {
    talks: Vec<SharedTalk<'t>>,
}

#[derive(Serialize)]
struct BirthdayParty<'t> {
    status: &'static str,
    scenarios: &'t [BirthdayScenario],
    talks: BTreeMap<&'t str, TalkScript>,
}

#[derive(Serialize)]
struct Tutorial<'t> {
    status: &'static str,
    rows: &'t [TutorialRow],
    talks: BTreeMap<&'t str, TalkScript>,
}

/// The talks of the characters.
#[derive(Debug, Clone, Copy, Default)]
pub struct Talks;

impl TableConverter for Talks {
    fn name(&self) -> &'static str {
        "talks"
    }

    fn version(&self) -> u32 {
        1
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        let Some(reader) = TalkReader::open(cx)? else {
            return Ok(Box::new(NoAssets));
        };
        let tables = Tables::read(cx)?;
        Ok(Box::new(OneAsset::new(NAME, move || {
            build(&reader, &tables)
        })))
    }
}

/// The talks that characters have at a piece of furniture.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixtureTalks;

impl TableConverter for FixtureTalks {
    fn name(&self) -> &'static str {
        "fixture-talks"
    }

    fn version(&self) -> u32 {
        1
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        let Some(reader) = TalkReader::open(cx)? else {
            return Ok(Box::new(NoAssets));
        };
        let tables = Tables::read(cx)?;
        let no_talk: Vec<NoTalkRow> = rows(cx, NO_TALK_FIXTURE_ACTIONS)?;
        let no_talk: BTreeSet<(i64, i64)> = no_talk
            .iter()
            .map(|row| (row.mysekai_fixture_id, row.game_character_unit_id))
            .collect();
        Ok(Box::new(OneAsset::new(FIXTURE_TALKS_NAME, move || {
            build_fixture_talks(&reader, &tables, &no_talk)
        })))
    }
}

/// The game data tables of the talks, joined by their keys.
struct Tables {
    talks: Vec<TalkRow>,
    members: BTreeMap<i64, Vec<i64>>,
    conditions: BTreeMap<i64, Condition>,
    condition_groups: BTreeMap<i64, Vec<i64>>,
    tweets: BTreeMap<i64, Tweet>,
    tweet_of_talk: BTreeMap<i64, i64>,
    links: BTreeMap<i64, Vec<SomeCharacterLink>>,
    birthday: Vec<BirthdayScenario>,
    tutorial: Vec<TutorialRow>,
}

impl Tables {
    fn read(cx: &dyn TableContext) -> Result<Self, ConvertError> {
        let unit_groups: Vec<UnitGroupRow> = rows(cx, UNIT_GROUPS)?;
        let conditions: Vec<Condition> = rows(cx, CONDITIONS)?;
        let condition_groups: Vec<ConditionGroupRow> = rows(cx, CONDITION_GROUPS)?;
        let pre_actions: Vec<PreActionRow> = rows(cx, PRE_ACTIONS)?;
        let tweets: Vec<Tweet> = rows(cx, TWEETS)?;
        let links: Vec<SomeCharacterLink> = rows(cx, SOME_CHARACTER_TALKS)?;
        Ok(Self {
            talks: rows(cx, TALKS)?,
            members: unit_groups
                .iter()
                .map(|group| (group.id, group.members()))
                .collect(),
            conditions: conditions.into_iter().map(|row| (row.id, row)).collect(),
            condition_groups: group_by(
                condition_groups,
                |row| row.group_id,
                |row| row.mysekai_character_talk_condition_id,
            ),
            tweets: tweets.into_iter().map(|row| (row.id, row)).collect(),
            tweet_of_talk: pre_actions
                .iter()
                .map(|row| {
                    (
                        row.mysekai_character_talk_id,
                        row.mysekai_character_talk_tweet_id,
                    )
                })
                .collect(),
            links: group_by(links, |row| row.mysekai_character_talk_id, |row| row),
            birthday: rows(cx, BIRTHDAY_SCENARIOS)?,
            tutorial: rows(cx, TUTORIAL_TALKS)?,
        })
    }

    fn members(&self, group: i64) -> Result<&[i64], ConvertError> {
        self.members
            .get(&group)
            .map(Vec::as_slice)
            .ok_or_else(|| malformed(format!("unit group {group} is not in its table")))
    }

    /// The conditions of a group, in the order of the rows of the group.
    fn conditions_of(&self, group: i64) -> Result<Vec<&Condition>, ConvertError> {
        self.condition_groups
            .get(&group)
            .ok_or_else(|| malformed(format!("condition group {group} is not in its table")))?
            .iter()
            .map(|id| {
                self.conditions
                    .get(id)
                    .ok_or_else(|| malformed(format!("condition {id} is not in its table")))
            })
            .collect()
    }

    /// The tweet of a talk, if its pre-action names one.
    fn tweet_of(&self, talk: i64) -> Result<Option<&Tweet>, ConvertError> {
        let Some(id) = self.tweet_of_talk.get(&talk).filter(|id| **id != 0) else {
            return Ok(None);
        };
        self.tweets
            .get(id)
            .map(Some)
            .ok_or_else(|| malformed(format!("tweet {id} is not in its table")))
    }
}

/// The values `value` reads from `rows`, by the key `key` reads.
fn group_by<R, V>(
    rows: Vec<R>,
    key: impl Fn(&R) -> i64,
    value: impl Fn(R) -> V,
) -> BTreeMap<i64, Vec<V>> {
    let mut groups: BTreeMap<i64, Vec<V>> = BTreeMap::new();
    for row in rows {
        groups.entry(key(&row)).or_default().push(value(row));
    }
    groups
}

fn names_any(conditions: &[&Condition], kinds: &[&str]) -> bool {
    conditions
        .iter()
        .any(|condition| kinds.contains(&condition.kind.as_str()))
}

fn build(reader: &TalkReader<'_>, tables: &Tables) -> Result<Asset, ConvertError> {
    let mut units: BTreeMap<String, UnitTalks<'_>> = BTreeMap::new();
    let mut shared = Vec::new();
    for row in &tables.talks {
        let members = tables.members(row.unit_group_id)?;
        let conditions = tables.conditions_of(row.condition_group_id)?;
        match (members, tables.links.get(&row.id)) {
            ([unit], _) if !names_any(&conditions, &FURNITURE_CONDITIONS) => {
                let units_talks = units.entry(unit.to_string()).or_default();
                units_talks
                    .talks
                    .push(talk(reader, tables, row, conditions)?);
            }
            ([_, _, ..], Some(links)) if !names_any(&conditions, &FIXTURE_ANCHOR) => {
                shared.push(SharedTalk {
                    talk: talk(reader, tables, row, conditions)?,
                    condition_group_id: row.condition_group_id,
                    unit_ids: members,
                    some_character_talks: links,
                });
            }
            _ => {}
        }
    }
    document(&Document {
        version: 1,
        units,
        birthday_party: BirthdayParty {
            status: "present",
            scenarios: &tables.birthday,
            talks: scripts_of(reader, tables.birthday.iter().map(|row| row.lua.as_str()))?,
        },
        common_some_character: CommonTalks { talks: shared },
        tutorial: Tutorial {
            status: "complete",
            rows: &tables.tutorial,
            talks: scripts_of(reader, tables.tutorial.iter().map(|row| row.lua.as_str()))?,
        },
    })
}

fn build_fixture_talks(
    reader: &TalkReader<'_>,
    tables: &Tables,
    no_talk: &BTreeSet<(i64, i64)>,
) -> Result<Asset, ConvertError> {
    let mut talks = Vec::new();
    for row in &tables.talks {
        let conditions = tables.conditions_of(row.condition_group_id)?;
        let unit_ids = tables.members(row.unit_group_id)?;
        let fixture_ids = fixture_ids(&conditions);
        let pairs: Vec<(i64, i64)> = fixture_ids
            .iter()
            .flat_map(|fixture| unit_ids.iter().map(|unit| (*fixture, *unit)))
            .collect();
        if pairs.is_empty() {
            continue;
        }
        talks.push(FixtureTalk {
            talk_id: row.id,
            lua: &row.lua,
            form: if pairs.iter().any(|pair| no_talk.contains(pair)) {
                1
            } else {
                2
            },
            site_group_id: row.site_group_id,
            term_id: row.term_id,
            condition_group_id: row.condition_group_id,
            fixture_ids,
            unit_ids,
            pairs,
            tweet: tables.tweet_of(row.id)?,
            script: reader.read(&row.lua)?,
        });
    }
    document(&FixtureTalksDocument { version: 1, talks })
}

/// The pieces of furniture that `conditions` anchor a talk to.
fn fixture_ids(conditions: &[&Condition]) -> Vec<i64> {
    conditions
        .iter()
        .filter(|condition| FIXTURE_ANCHOR.contains(&condition.kind.as_str()))
        .filter_map(|condition| condition.value)
        .collect()
}

fn talk<'t>(
    reader: &TalkReader<'_>,
    tables: &'t Tables,
    row: &'t TalkRow,
    conditions: Vec<&'t Condition>,
) -> Result<Talk<'t>, ConvertError> {
    Ok(Talk {
        talk_id: row.id,
        lua: &row.lua,
        site_group_id: row.site_group_id,
        term_id: row.term_id,
        conditions: conditions.iter().map(|c| c.kind.as_str()).collect(),
        condition_values: conditions,
        tweet: tables.tweet_of(row.id)?,
        script: reader.read(&row.lua)?,
    })
}

/// Each distinct script of `names`.
fn scripts_of<'t>(
    reader: &TalkReader<'_>,
    names: impl Iterator<Item = &'t str>,
) -> Result<BTreeMap<&'t str, TalkScript>, ConvertError> {
    let mut scripts = BTreeMap::new();
    for name in names {
        if !scripts.contains_key(name) {
            scripts.insert(name, reader.read(name)?);
        }
    }
    Ok(scripts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekai_convert::Content;
    use sekai_convert::tables::AssetEntry;
    use sekai_format::AssetDocument;
    use serde_json::{Value, json};
    use std::collections::HashMap;

    const TALK_BUNDLE: &str = "mysekai/talk/scenario/talk";
    const LIB_BUNDLE: &str = "mysekai/talk/scenario/lib";

    /// A version with tables and the scripts of the two packages.
    struct Version {
        tables: HashMap<&'static str, Value>,
        scripts: HashMap<String, &'static str>,
        talk_assets: Vec<AssetEntry>,
        lib_assets: Vec<AssetEntry>,
    }

    fn entry(name: String) -> AssetEntry {
        AssetEntry { name, form: "text" }
    }

    impl Version {
        fn new(scripts: &[(&str, &'static str)]) -> Self {
            let lib = format!("{LIB_BUNDLE}/defines.lua");
            let files: HashMap<String, &'static str> = scripts
                .iter()
                .map(|(name, text)| (format!("{TALK_BUNDLE}/{name}.lua"), *text))
                .chain([(lib.clone(), "EyePresets = { normal = \"open\" }")])
                .collect();
            let mut talk_assets: Vec<AssetEntry> = files
                .keys()
                .filter(|name| name.starts_with(TALK_BUNDLE))
                .cloned()
                .map(entry)
                .collect();
            talk_assets.sort_by(|a, b| a.name.cmp(&b.name));
            Self {
                tables: HashMap::from([
                    (TALKS, json!([])),
                    (UNIT_GROUPS, json!([])),
                    (CONDITIONS, json!([])),
                    (CONDITION_GROUPS, json!([])),
                    (PRE_ACTIONS, json!([])),
                    (TWEETS, json!([])),
                    (SOME_CHARACTER_TALKS, json!([])),
                    (BIRTHDAY_SCENARIOS, json!([])),
                    (TUTORIAL_TALKS, json!([])),
                    (NO_TALK_FIXTURE_ACTIONS, json!([])),
                ]),
                scripts: files,
                talk_assets,
                lib_assets: vec![entry(lib)],
            }
        }

        fn with(mut self, table: &'static str, rows: Value) -> Self {
            self.tables.insert(table, rows);
            self
        }
    }

    impl TableContext for Version {
        fn table(&self, name: &str) -> Result<Vec<u8>, ConvertError> {
            Ok(serde_json::to_vec(&self.tables[name]).unwrap())
        }

        fn bundles(&self) -> Vec<&str> {
            vec![TALK_BUNDLE, LIB_BUNDLE]
        }

        fn assets(&self, bundle: &str) -> &[AssetEntry] {
            match bundle {
                TALK_BUNDLE => &self.talk_assets,
                LIB_BUNDLE => &self.lib_assets,
                _ => &[],
            }
        }

        fn document(&self, name: &str) -> Result<AssetDocument, ConvertError> {
            Err(ConvertError::new("missing_asset", name))
        }

        fn member(&self, name: &str, _role: &str) -> Result<Vec<u8>, ConvertError> {
            self.scripts
                .get(name)
                .map(|text| text.as_bytes().to_vec())
                .ok_or_else(|| ConvertError::new("missing_asset", name))
        }
    }

    fn converted(version: &Version) -> Result<Value, ConvertError> {
        converted_by(&Talks, version)
    }

    fn converted_by(
        converter: &dyn TableConverter,
        version: &Version,
    ) -> Result<Value, ConvertError> {
        let plan = converter.plan(version)?;
        let asset = plan.convert(0)?.asset;
        let [member] = asset.members.as_slice() else {
            panic!("{asset:?}");
        };
        let Content::File { bytes, .. } = &member.content else {
            panic!("the document is not a file");
        };
        Ok(serde_json::from_slice(bytes).unwrap())
    }

    fn talk_row(id: i64, lua: &str, unit_group: i64, condition_group: i64) -> Value {
        json!({
            "id": id, "lua": lua, "mysekaiSiteGroupId": 2, "mysekaiCharacterTalkTermId": 3,
            "mysekaiCharacterTalkConditionGroupId": condition_group,
            "mysekaiGameCharacterUnitGroupId": unit_group,
        })
    }

    /// Unit groups 1 (character 7) and 2 (characters 7 and 8), and condition group 1 of a visit
    /// count, 2 of a fixture and 3 of a fixture tag.
    fn tables(version: Version) -> Version {
        version
            .with(
                UNIT_GROUPS,
                json!([
                    {"id": 1, "gameCharacterUnitId1": 7, "gameCharacterUnitId2": 0},
                    {"id": 2, "gameCharacterUnitId1": 7, "gameCharacterUnitId2": 8},
                ]),
            )
            .with(
                CONDITIONS,
                json!([
                    {"id": 1, "mysekaiCharacterTalkConditionType": "visit_count",
                     "mysekaiCharacterTalkConditionTypeValue": 4},
                    {"id": 2, "mysekaiCharacterTalkConditionType": "mysekai_fixture_id",
                     "mysekaiCharacterTalkConditionTypeValue": 30},
                    {"id": 3, "mysekaiCharacterTalkConditionType": "mysekai_fixture_tag_id"},
                ]),
            )
            .with(
                CONDITION_GROUPS,
                json!([
                    {"groupId": 1, "mysekaiCharacterTalkConditionId": 1},
                    {"groupId": 2, "mysekaiCharacterTalkConditionId": 2},
                    {"groupId": 3, "mysekaiCharacterTalkConditionId": 3},
                ]),
            )
    }

    const SCRIPT: &str = "label(\"name\")\nvoice(\"talk\", \"cue_1\", 1)\nwait_click()\n";

    #[test]
    fn a_talk_of_one_character_joins_its_tables() {
        let version = tables(Version::new(&[("a", SCRIPT)]))
            .with(TALKS, json!([talk_row(5, "a", 1, 1)]))
            .with(
                PRE_ACTIONS,
                json!([{"mysekaiCharacterTalkId": 5, "mysekaiCharacterTalkTweetId": 9}]),
            )
            .with(
                TWEETS,
                json!([{"id": 9, "text": "hi", "motionName": "m", "expressionEyeName": "e",
                        "expressionMouthName": "o"}]),
            );
        let value = converted(&version).unwrap();
        let talk = &value["units"]["7"]["talks"][0];
        assert_eq!(talk["talkId"], 5);
        assert_eq!(talk["lua"], "a");
        assert_eq!(talk["siteGroupId"], 2);
        assert_eq!(talk["termId"], 3);
        assert_eq!(talk["conditions"], json!(["visit_count"]));
        assert_eq!(
            talk["conditionValues"],
            json!([{"conditionType": "visit_count", "conditionTypeValue": 4}])
        );
        assert_eq!(
            talk["tweet"],
            json!({"id": 9, "text": "hi", "motion": "m", "eye": "e", "mouth": "o"})
        );
        assert_eq!(talk["voices"], json!(["cue_1"]));
        assert_eq!(talk["steps"][2]["op"], "wait_click");
    }

    #[test]
    fn a_talk_without_a_tweet_has_null() {
        let version = tables(Version::new(&[("a", SCRIPT)]))
            .with(TALKS, json!([talk_row(5, "a", 1, 1)]))
            .with(
                PRE_ACTIONS,
                json!([{"mysekaiCharacterTalkId": 5, "mysekaiCharacterTalkTweetId": 0}]),
            );
        assert!(converted(&version).unwrap()["units"]["7"]["talks"][0]["tweet"].is_null());
    }

    #[test]
    fn a_talk_about_furniture_is_nobodys_own() {
        let version = tables(Version::new(&[("a", SCRIPT)])).with(
            TALKS,
            json!([talk_row(5, "a", 1, 2), talk_row(6, "a", 1, 3)]),
        );
        assert_eq!(converted(&version).unwrap()["units"], json!({}));
    }

    #[test]
    fn a_shared_talk_needs_a_link_and_no_fixture_anchor() {
        let version = tables(Version::new(&[("a", SCRIPT)]))
            .with(
                TALKS,
                json!([
                    talk_row(5, "a", 2, 1),
                    talk_row(6, "a", 2, 1),
                    talk_row(7, "a", 2, 2),
                    talk_row(8, "a", 2, 3),
                ]),
            )
            .with(
                SOME_CHARACTER_TALKS,
                json!([
                    {"id": 1, "mysekaiCharacterTalkId": 5, "mainGameCharacterUnitId": 7},
                    {"id": 2, "mysekaiCharacterTalkId": 7, "mainGameCharacterUnitId": 7},
                    {"id": 3, "mysekaiCharacterTalkId": 8, "mainGameCharacterUnitId": 8},
                ]),
            );
        let value = converted(&version).unwrap();
        let talks = value["commonSomeCharacter"]["talks"].as_array().unwrap();
        let ids: Vec<&Value> = talks.iter().map(|talk| &talk["talkId"]).collect();
        assert_eq!(ids, [&json!(5), &json!(8)]);
        assert_eq!(talks[0]["unitIds"], json!([7, 8]));
        assert_eq!(talks[0]["conditionGroupId"], 1);
        assert_eq!(
            talks[0]["someCharacterTalks"],
            json!([{"id": 1, "mysekaiCharacterTalkId": 5, "mainGameCharacterUnitId": 7}])
        );
        assert_eq!(talks[0]["voices"], json!(["cue_1"]));
        assert!(talks[0]["tweet"].is_null());
    }

    #[test]
    fn parties_and_the_tutorial_hold_each_script_once() {
        let version = Version::new(&[("p", SCRIPT), ("t", SCRIPT)])
            .with(
                BIRTHDAY_SCENARIOS,
                json!([
                    {"id": 1, "birthdayPartyId": 1, "seq": 1, "lua": "p"},
                    {"id": 2, "birthdayPartyId": 2, "seq": 1, "lua": "p"},
                ]),
            )
            .with(
                TUTORIAL_TALKS,
                json!([
                    {"id": 3, "assetbundleName": "bundle", "lua": "t"},
                    {"id": 4, "assetbundleName": "bundle", "lua": "t",
                     "mysekaiCharacterTalkTweetId": 6},
                ]),
            );
        let value = converted(&version).unwrap();
        let party = &value["birthdayParty"];
        assert_eq!(party["status"], "present");
        assert_eq!(party["scenarios"].as_array().unwrap().len(), 2);
        assert_eq!(party["talks"].as_object().unwrap().len(), 1);
        assert_eq!(party["talks"]["p"]["voices"], json!(["cue_1"]));
        let tutorial = &value["tutorial"];
        assert_eq!(tutorial["status"], "complete");
        assert_eq!(tutorial["rows"][0]["assetbundleName"], "bundle");
        assert!(
            tutorial["rows"][0]
                .get("mysekaiCharacterTalkTweetId")
                .is_none()
        );
        assert_eq!(tutorial["rows"][1]["mysekaiCharacterTalkTweetId"], 6);
        assert_eq!(tutorial["talks"]["t"]["steps"][0]["name"], "name");
    }

    #[test]
    fn a_talk_at_furniture_pairs_each_piece_with_each_character() {
        let version = tables(Version::new(&[("a", SCRIPT)]))
            .with(
                TALKS,
                json!([
                    talk_row(5, "a", 2, 2),
                    talk_row(6, "a", 1, 2),
                    talk_row(7, "a", 2, 1)
                ]),
            )
            .with(
                NO_TALK_FIXTURE_ACTIONS,
                json!([{"mysekaiFixtureId": 30, "gameCharacterUnitId": 8}]),
            );
        let value = converted_by(&FixtureTalks, &version).unwrap();
        assert_eq!(value["version"], 1);
        let talks = value["talks"].as_array().unwrap();
        assert_eq!(talks.len(), 2);
        assert_eq!(talks[0]["talkId"], 5);
        assert_eq!(talks[0]["fixtureIds"], json!([30]));
        assert_eq!(talks[0]["unitIds"], json!([7, 8]));
        assert_eq!(talks[0]["pairs"], json!([[30, 7], [30, 8]]));
        assert_eq!(talks[0]["form"], 1);
        assert_eq!(talks[0]["conditionGroupId"], 2);
        assert_eq!(talks[0]["voices"], json!(["cue_1"]));
        assert_eq!(talks[1]["form"], 2);
        assert!(talks[1]["tweet"].is_null());
    }

    #[test]
    fn a_version_without_the_packages_has_no_asset() {
        struct Empty;
        impl TableContext for Empty {
            fn table(&self, name: &str) -> Result<Vec<u8>, ConvertError> {
                Err(ConvertError::new("missing_asset", name))
            }
            fn bundles(&self) -> Vec<&str> {
                Vec::new()
            }
            fn assets(&self, _bundle: &str) -> &[AssetEntry] {
                &[]
            }
            fn document(&self, name: &str) -> Result<AssetDocument, ConvertError> {
                Err(ConvertError::new("missing_asset", name))
            }
            fn member(&self, name: &str, _role: &str) -> Result<Vec<u8>, ConvertError> {
                Err(ConvertError::new("missing_asset", name))
            }
        }
        assert!(Talks.plan(&Empty).unwrap().names().is_empty());
    }

    #[test]
    fn what_the_tables_or_scripts_do_not_hold_is_an_error() {
        let one = || json!([talk_row(5, "a", 1, 1)]);
        let unknown_unit_group =
            tables(Version::new(&[("a", SCRIPT)])).with(TALKS, json!([talk_row(5, "a", 9, 1)]));
        let unknown_condition_group =
            tables(Version::new(&[("a", SCRIPT)])).with(TALKS, json!([talk_row(5, "a", 1, 9)]));
        let unknown_tweet = tables(Version::new(&[("a", SCRIPT)]))
            .with(TALKS, one())
            .with(
                PRE_ACTIONS,
                json!([{"mysekaiCharacterTalkId": 5, "mysekaiCharacterTalkTweetId": 9}]),
            );
        let unknown_script = tables(Version::new(&[("other", SCRIPT)])).with(TALKS, one());
        let unknown_operation =
            tables(Version::new(&[("a", "something_else(1)")])).with(TALKS, one());
        let control_flow =
            tables(Version::new(&[("a", "if x then wait_click() end")])).with(TALKS, one());
        for version in [
            unknown_unit_group,
            unknown_condition_group,
            unknown_tweet,
            unknown_script,
            unknown_operation,
            control_flow,
        ] {
            assert!(converted(&version).is_err());
        }
    }
}
