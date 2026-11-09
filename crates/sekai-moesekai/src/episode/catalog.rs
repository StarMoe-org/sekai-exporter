//! The episodes the game data tables list.

use std::collections::BTreeMap;

use sekai_convert::ConvertError;
use sekai_convert::tables::TableContext;
use sekai_format::episode::{Story, StoryType};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use super::rules;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UnitStory {
    #[serde(default)]
    chapters: Vec<Chapter>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Chapter {
    id: i64,
    assetbundle_name: String,
    #[serde(default)]
    episodes: Vec<EpisodeRow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EpisodeRow {
    id: i64,
    episode_no: i64,
    #[serde(default)]
    title: String,
    scenario_id: String,
    #[serde(default)]
    assetbundle_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventStory {
    event_id: i64,
    assetbundle_name: String,
    #[serde(default)]
    event_story_episodes: Vec<EpisodeRow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CardEpisode {
    id: i64,
    card_id: i64,
    #[serde(default)]
    title: String,
    scenario_id: String,
    #[serde(default)]
    card_episode_part_type: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Card {
    id: i64,
    assetbundle_name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpecialStory {
    id: i64,
    #[serde(default)]
    assetbundle_name: String,
    #[serde(default)]
    episodes: Vec<EpisodeRow>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Character2dRow {
    id: i64,
    #[serde(default)]
    asset_name: Option<String>,
    #[serde(default)]
    unit: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActionSetRow {
    id: i64,
    area_id: i64,
    #[serde(default)]
    scenario_id: Option<String>,
    #[serde(default)]
    script_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Costume2dRow {
    character2d_id: i64,
    costume2d_group_id: i64,
    #[serde(default)]
    from_mmddhh: String,
    #[serde(default)]
    to_mmddhh: String,
    #[serde(default)]
    live2d_assetbundle_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AreaPlaylist {
    area_id: i64,
    #[serde(default)]
    assetbundle_name: String,
    #[serde(default)]
    bgm_name: String,
}

/// What a row of `character2ds` says about a character.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Character2d {
    pub(super) asset_name: Option<String>,
    pub(super) unit: Option<String>,
}

/// A row of `costume2ds`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Costume2d {
    pub(super) group: i64,
    pub(super) from: String,
    pub(super) to: String,
    pub(super) model: Option<String>,
}

/// Where the data of an area conversation is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AreaSource {
    pub(super) action_set_id: i64,
    pub(super) area_id: i64,
    pub(super) script_id: String,
    pub(super) data_bundle: String,
    pub(super) bgm_bundle: String,
    pub(super) bgm_cue: String,
}

/// One episode of the tables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub(super) story: Story,
    /// The bundles that may hold the scene data, in order.
    pub(super) scenario_bundles: Vec<String>,
    /// The bundles of the sound effects of the episode.
    pub(super) se_bundles: Vec<String>,
    pub(super) area: Option<AreaSource>,
}

/// The tables as the episodes read them.
#[derive(Debug, Default)]
pub(super) struct Catalog {
    pub(super) rows: Vec<Row>,
    pub(super) character2ds: BTreeMap<i64, Character2d>,
    pub(super) costume2ds: BTreeMap<i64, Vec<Costume2d>>,
}

fn table<T: DeserializeOwned>(cx: &dyn TableContext, name: &str) -> Result<Vec<T>, ConvertError> {
    serde_json::from_slice(&cx.table(name)?)
        .map_err(|e| ConvertError::new("malformed", format!("table {name}: {e}")))
}

fn not_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.is_empty())
}

fn selector(story_type: StoryType, key: &str, episode: &str) -> String {
    format!("{}:{key}/{episode}", story_type.as_str())
}

/// The row of a numbered episode.
fn numbered(
    story_type: StoryType,
    (key, story_id): (&str, i64),
    row: EpisodeRow,
    scenario_bundles: Vec<String>,
    se_bundles: Vec<String>,
) -> Row {
    Row {
        story: Story {
            story_type,
            selector: selector(story_type, key, &row.episode_no.to_string()),
            story_id,
            episode_no: row.episode_no,
            episode_id: row.id,
            title: row.title,
            scenario_id: row.scenario_id,
        },
        scenario_bundles,
        se_bundles,
        area: None,
    }
}

fn units(stories: Vec<UnitStory>, rows: &mut Vec<Row>) {
    for chapter in stories.into_iter().flat_map(|story| story.chapters) {
        let key = chapter.assetbundle_name.as_str();
        rows.extend(chapter.episodes.into_iter().map(|row| {
            let scenario_bundles = vec![rules::unit_scenario(key)];
            numbered(
                StoryType::Unit,
                (key, chapter.id),
                row,
                scenario_bundles,
                Vec::new(),
            )
        }));
    }
}

fn events(stories: Vec<EventStory>, rows: &mut Vec<Row>) {
    for event in stories {
        let key = event.event_id.to_string();
        let bundle = event.assetbundle_name.as_str();
        rows.extend(event.event_story_episodes.into_iter().map(|row| {
            let scenario_bundles = vec![rules::event_scenario(bundle)];
            let se_bundles = vec![rules::event_se(bundle)];
            numbered(
                StoryType::Event,
                (&key, event.event_id),
                row,
                scenario_bundles,
                se_bundles,
            )
        }));
    }
}

fn specials(stories: Vec<SpecialStory>, rows: &mut Vec<Row>) {
    for special in stories {
        let key = special.id.to_string();
        let story = special.assetbundle_name.as_str();
        rows.extend(special.episodes.into_iter().map(|row| {
            let episode = row.assetbundle_name.as_deref();
            let scenario_bundles = rules::special_scenarios(&row.scenario_id, story, episode);
            numbered(
                StoryType::Special,
                (&key, special.id),
                row,
                scenario_bundles,
                Vec::new(),
            )
        }));
    }
}

fn card_episodes(episodes: Vec<CardEpisode>, cards: Vec<Card>, rows: &mut Vec<Row>) {
    let cards: BTreeMap<i64, String> = cards
        .into_iter()
        .map(|card| (card.id, card.assetbundle_name))
        .collect();
    let mut ordinals: BTreeMap<i64, i64> = BTreeMap::new();
    for row in episodes {
        let Some(card) = cards.get(&row.card_id) else {
            continue;
        };
        let ordinal = ordinals.entry(row.card_id).or_default();
        *ordinal += 1;
        let (episode_no, part) = match row.card_episode_part_type.as_deref() {
            Some("first_part") => (1, "first".to_owned()),
            Some("second_part") => (2, "second".to_owned()),
            _ => (*ordinal, row.id.to_string()),
        };
        rows.push(Row {
            story: Story {
                story_type: StoryType::Card,
                selector: selector(StoryType::Card, &row.card_id.to_string(), &part),
                story_id: row.card_id,
                episode_no,
                episode_id: row.id,
                title: row.title,
                scenario_id: row.scenario_id,
            },
            scenario_bundles: rules::card_scenarios(card),
            se_bundles: Vec::new(),
            area: None,
        });
    }
}

fn action_sets(sets: Vec<ActionSetRow>, playlists: &[AreaPlaylist], rows: &mut Vec<Row>) {
    for set in sets {
        let Some(scenario_id) = not_empty(set.scenario_id) else {
            continue;
        };
        let playlist = playlists
            .iter()
            .find(|row| row.area_id == set.area_id)
            .map(|row| (row.assetbundle_name.as_str(), row.bgm_name.as_str()));
        let (bgm_bundle, bgm_cue) = rules::area_bgm(playlist);
        rows.push(Row {
            story: Story {
                story_type: StoryType::Area,
                selector: selector(
                    StoryType::Area,
                    &set.area_id.to_string(),
                    &set.id.to_string(),
                ),
                story_id: set.area_id,
                episode_no: set.id,
                episode_id: set.id,
                title: String::new(),
                scenario_id,
            },
            scenario_bundles: vec![rules::action_set_scenario(set.id)],
            se_bundles: Vec::new(),
            area: Some(AreaSource {
                action_set_id: set.id,
                area_id: set.area_id,
                script_id: set.script_id,
                data_bundle: rules::action_set_data(set.id),
                bgm_bundle,
                bgm_cue,
            }),
        });
    }
}

/// Reads the tables.
pub(super) fn load(cx: &dyn TableContext) -> Result<Catalog, ConvertError> {
    let mut catalog = Catalog::default();
    let rows = &mut catalog.rows;
    units(table(cx, "unitStories")?, rows);
    events(table(cx, "eventStories")?, rows);
    card_episodes(table(cx, "cardEpisodes")?, table(cx, "cards")?, rows);
    specials(table(cx, "specialStories")?, rows);
    let playlists: Vec<AreaPlaylist> = table(cx, "areaPlaylists")?;
    action_sets(table(cx, "actionSets")?, &playlists, rows);
    for row in table::<Character2dRow>(cx, "character2ds")? {
        catalog.character2ds.insert(
            row.id,
            Character2d {
                asset_name: not_empty(row.asset_name),
                unit: not_empty(row.unit),
            },
        );
    }
    for row in table::<Costume2dRow>(cx, "costume2ds")? {
        catalog
            .costume2ds
            .entry(row.character2d_id)
            .or_default()
            .push(Costume2d {
                group: row.costume2d_group_id,
                from: row.from_mmddhh,
                to: row.to_mmddhh,
                model: not_empty(row.live2d_assetbundle_name),
            });
    }
    Ok(catalog)
}
