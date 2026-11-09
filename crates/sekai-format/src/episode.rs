//! Episode documents, specified in `docs/spec/episode.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::audio::CueWaveform;

/// The value of the `format` key.
pub const FORMAT: &str = "episode";
/// The version of the document format.
pub const VERSION: u32 = 1;

/// Everything one story episode needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeDocument {
    /// Always [`FORMAT`].
    pub format: String,
    /// The version of the document format.
    pub version: u32,
    /// Which episode this is.
    pub story: Story,
    /// The name of the scenario asset.
    pub scenario: String,
    /// The characters, by `Character2dId`.
    pub characters: BTreeMap<i64, Character>,
    /// Background key to the name of the texture asset.
    pub backgrounds: BTreeMap<String, String>,
    /// The music, by cue name.
    pub bgm: BTreeMap<String, Sound>,
    /// The sound effects, by cue name.
    pub se: BTreeMap<String, Sound>,
    /// The voices, by `VoiceId`.
    pub voices: BTreeMap<String, Sound>,
    /// Effect name to the name of the bundle that holds it.
    pub effects: BTreeMap<String, String>,
    /// Movie name to the names of its video assets.
    pub movies: BTreeMap<String, Vec<String>>,
    /// The music video IDs, in order of first use.
    pub music_videos: Vec<String>,
    /// Present for area conversations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub area_talk: Option<AreaTalk>,
    /// What could not be resolved.
    pub warnings: Vec<Warning>,
}

impl EpisodeDocument {
    /// The bytes of the document.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// Reads a document.
    pub fn from_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

/// The table that lists an episode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryType {
    /// `unitStories`.
    Unit,
    /// `eventStories`.
    Event,
    /// `cardEpisodes`.
    Card,
    /// `specialStories`.
    Special,
    /// `actionSets`.
    Area,
}

impl StoryType {
    /// The name of the type in selectors and asset names.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Event => "event",
            Self::Card => "card",
            Self::Special => "special",
            Self::Area => "area",
        }
    }
}

/// Which episode a document is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Story {
    /// The table that lists the episode.
    #[serde(rename = "type")]
    pub story_type: StoryType,
    /// `<type>:<key>/<episode>`.
    pub selector: String,
    /// The ID of the story.
    pub story_id: i64,
    /// The number of the episode within the story.
    pub episode_no: i64,
    /// The ID of the table row.
    pub episode_id: i64,
    /// The title.
    pub title: String,
    /// The scene data the table row names.
    pub scenario_id: String,
}

/// One character of an episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Character {
    /// `assetName` of the `character2ds` row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_name: Option<String>,
    /// The name of the motion set asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion_set: Option<String>,
    /// The costumes in order of first appearance.
    pub costumes: Vec<Costume>,
    /// The candidates of an area conversation, in table order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub area_costumes: Vec<AreaCostume>,
}

/// A costume of a character.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Costume {
    /// `CostumeType` as the scene data names it.
    pub costume_type: String,
    /// The name of the model asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The motions and facials the episode uses, by the name in the scene data.
    pub motions: BTreeMap<String, MotionRef>,
}

/// A model an area conversation may load, by the time of year.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AreaCostume {
    /// The start of the window, `MMDDHH`.
    pub from: String,
    /// The last hour of the window, `MMDDHH`.
    pub to: String,
    /// The name of the model asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The motions and facials the episode uses with this model.
    pub motions: BTreeMap<String, MotionRef>,
}

/// Where a motion is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MotionRef {
    /// The name of the asset.
    pub asset: String,
    /// The role of the member.
    pub role: String,
}

/// One cue an episode plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sound {
    /// The name of the audio asset.
    pub asset: String,
    /// The name of the cue.
    pub cue: String,
    /// The entries of the cue in the cue table.
    pub waveforms: Vec<CueWaveform>,
}

/// How an area conversation plays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AreaTalk {
    /// `actionSets.id`.
    pub action_set_id: i64,
    /// `actionSets.areaId`.
    pub area_id: i64,
    /// A key of `backgrounds`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// A key of `bgm`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bgm: Option<String>,
}

/// Something an episode refers to that could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    /// What kind of problem it is.
    pub kind: WarningKind,
    /// A description for people.
    pub detail: String,
}

impl Warning {
    /// A warning of `kind`.
    pub fn new(kind: WarningKind, detail: impl Into<String>) -> Self {
        Self {
            kind,
            detail: detail.into(),
        }
    }
}

/// The kinds of [`Warning`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    /// The version holds no asset that a reference needs.
    AssetMissing,
    /// No searched cue sheet has the cue.
    CueNotFound,
    /// The version holds no model for a costume.
    ModelMissing,
    /// The motion set of a character is unknown or not in the version.
    MotionSetUnresolved,
    /// A name is in none of the models of a character nor its motion set.
    MotionNotFound,
    /// The scene data uses a `Character2dId` that `character2ds` lacks.
    TableMismatch,
    /// A motion is given for a character outside `AppearCharacters`.
    UnknownCharacter,
    /// An `EffectType` that the game skips.
    UnknownEffectType,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> EpisodeDocument {
        EpisodeDocument {
            format: FORMAT.to_owned(),
            version: VERSION,
            story: Story {
                story_type: StoryType::Area,
                selector: "area:8/1".to_owned(),
                story_id: 8,
                episode_no: 1,
                episode_id: 1,
                title: String::new(),
                scenario_id: "s".to_owned(),
            },
            scenario: "b/s".to_owned(),
            characters: BTreeMap::from([(
                17,
                Character {
                    asset_name: None,
                    motion_set: Some("m".to_owned()),
                    costumes: vec![Costume {
                        costume_type: "c".to_owned(),
                        model: None,
                        motions: BTreeMap::new(),
                    }],
                    area_costumes: Vec::new(),
                },
            )]),
            backgrounds: BTreeMap::new(),
            bgm: BTreeMap::new(),
            se: BTreeMap::new(),
            voices: BTreeMap::new(),
            effects: BTreeMap::new(),
            movies: BTreeMap::new(),
            music_videos: Vec::new(),
            area_talk: None,
            warnings: vec![Warning::new(WarningKind::AssetMissing, "x")],
        }
    }

    #[test]
    fn the_bytes_are_fixed() {
        assert_eq!(
            String::from_utf8(document().to_bytes().unwrap()).unwrap(),
            r#"{"format":"episode","version":1,"story":{"type":"area","selector":"area:8/1","story_id":8,"episode_no":1,"episode_id":1,"title":"","scenario_id":"s"},"scenario":"b/s","characters":{"17":{"motion_set":"m","costumes":[{"costume_type":"c","motions":{}}]}},"backgrounds":{},"bgm":{},"se":{},"voices":{},"effects":{},"movies":{},"music_videos":[],"warnings":[{"kind":"asset_missing","detail":"x"}]}"#
        );
    }

    #[test]
    fn a_document_reads_back() {
        let document = document();
        let back = EpisodeDocument::from_bytes(&document.to_bytes().unwrap()).unwrap();
        assert_eq!(back, document);
    }
}
