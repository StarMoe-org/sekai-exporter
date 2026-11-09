//! The `episode` form on tables and assets built in memory.

#![cfg(feature = "episode")]

use std::collections::{BTreeMap, HashMap};

use sekai_convert::tables::{AssetEntry, TableAsset, TableContext, TableConverter};
use sekai_convert::{Content, ConvertError};
use sekai_format::AssetDocument;
use sekai_format::asset::MemberFile;
use sekai_format::audio::{CueEntry, CueTable, CueWaveform, FORMAT as CUE_FORMAT};
use sekai_format::episode::{EpisodeDocument, MotionRef, StoryType, WarningKind};
use sekai_format::story::{
    ACTION_SET_FORMAT, ActionSet, ActionSetDetail, SCENARIO_BACKGROUND_FORMAT, ScenarioBackground,
};
use sekai_moesekai::episode::Episode;
use serde_json::{Value, json};

/// A version built in memory. Every table not set is empty.
#[derive(Default)]
struct Version {
    tables: HashMap<String, Vec<u8>>,
    bundles: BTreeMap<String, Vec<AssetEntry>>,
    members: HashMap<(String, String), Vec<u8>>,
    roles: HashMap<String, Vec<String>>,
}

impl Version {
    fn table(mut self, name: &str, rows: Value) -> Self {
        self.tables
            .insert(name.to_owned(), serde_json::to_vec(&rows).unwrap());
        self
    }

    fn asset(&mut self, bundle: &str, file: &str, form: &'static str) -> String {
        let name = format!("{bundle}/{file}");
        let assets = self.bundles.entry(bundle.to_owned()).or_default();
        assets.push(AssetEntry {
            name: name.clone(),
            form,
        });
        assets.sort_by(|a, b| a.name.cmp(&b.name));
        name
    }

    fn scene(mut self, bundle: &str, id: &str, scene: Value) -> Self {
        let name = self.asset(bundle, id, "scenario");
        self.members.insert(
            (name, "scenario".to_owned()),
            serde_json::to_vec(&scene).unwrap(),
        );
        self
    }

    fn cues(mut self, bundle: &str, file: &str, cues: &[&str]) -> Self {
        let name = self.asset(bundle, file, "audio");
        let table = CueTable {
            format: CUE_FORMAT.to_owned(),
            version: 1,
            cues: cues
                .iter()
                .map(|cue| CueEntry {
                    name: (*cue).to_owned(),
                    waveforms: vec![waveform(cue)],
                    arrangement: None,
                })
                .collect(),
        };
        self.members
            .insert((name, "cues".to_owned()), table.to_bytes().unwrap());
        self
    }

    fn with_roles(mut self, bundle: &str, file: &str, form: &'static str, roles: &[&str]) -> Self {
        let name = self.asset(bundle, file, form);
        self.roles
            .insert(name, roles.iter().map(|r| (*r).to_owned()).collect());
        self
    }

    fn plain(mut self, bundle: &str, file: &str, form: &'static str) -> Self {
        self.asset(bundle, file, form);
        self
    }

    fn action_set(mut self, bundle: &str, file: &str, details: &[(i64, i64)]) -> Self {
        let name = self.asset(bundle, file, "action-set");
        let set = ActionSet {
            format: ACTION_SET_FORMAT.to_owned(),
            version: 1,
            action_set_id: 0,
            area_id: 0,
            details: details
                .iter()
                .map(|&(area_character_id, costume2d_group_id)| ActionSetDetail {
                    area_character_id,
                    costume2d_group_id,
                })
                .collect(),
        };
        self.members
            .insert((name, "action-set".to_owned()), set.to_bytes().unwrap());
        self
    }

    fn background_movies(mut self, bundle: &str, file: &str, movies: &[&str]) -> Self {
        let name = self.asset(bundle, file, "scenario-background");
        let background = ScenarioBackground {
            format: SCENARIO_BACKGROUND_FORMAT.to_owned(),
            version: 1,
            movies: movies.iter().map(|m| (*m).to_owned()).collect(),
        };
        self.members.insert(
            (name, "scenario-background".to_owned()),
            background.to_bytes().unwrap(),
        );
        self
    }
}

fn waveform(cue: &str) -> CueWaveform {
    CueWaveform {
        role: format!("cue/{cue}"),
        frames: 48,
        sample_rate: 48_000,
        channels: 2,
        loop_frames: None,
        volume: 1.0,
    }
}

impl TableContext for Version {
    fn table(&self, name: &str) -> Result<Vec<u8>, ConvertError> {
        Ok(self.tables.get(name).cloned().unwrap_or(b"[]".to_vec()))
    }

    fn bundles(&self) -> Vec<&str> {
        self.bundles.keys().map(String::as_str).collect()
    }

    fn assets(&self, bundle: &str) -> &[AssetEntry] {
        self.bundles.get(bundle).map_or(&[], Vec::as_slice)
    }

    fn document(&self, name: &str) -> Result<AssetDocument, ConvertError> {
        Ok(AssetDocument {
            format: String::new(),
            version: 1,
            name: name.to_owned(),
            form: String::new(),
            form_version: 1,
            fingerprint: String::new(),
            sources: Vec::new(),
            resources: Vec::new(),
            members: self.roles[name]
                .iter()
                .map(|role| MemberFile {
                    role: role.clone(),
                    sha256: String::new(),
                    size: 0,
                    extension: String::new(),
                    media_type: String::new(),
                })
                .collect(),
            references: Vec::new(),
            error: None,
        })
    }

    fn member(&self, name: &str, role: &str) -> Result<Vec<u8>, ConvertError> {
        self.members
            .get(&(name.to_owned(), role.to_owned()))
            .cloned()
            .ok_or_else(|| ConvertError::new("store_failed", format!("{name} {role}")))
    }
}

fn unit_table(scenarios: &[&str]) -> Value {
    let episodes: Vec<Value> = scenarios
        .iter()
        .enumerate()
        .map(|(i, id)| json!({"id": 100 + i, "episodeNo": i + 1, "title": "T", "scenarioId": id}))
        .collect();
    json!([{"chapters": [{"id": 1, "assetbundleName": "c", "episodes": episodes}]}])
}

const UNIT: &str = "scenario/unitstory/c";

fn characters() -> Value {
    json!([
        {"id": 1, "assetName": "first"},
        {"id": 17, "assetName": "miku", "unit": "light"},
        {"id": 18, "assetName": "v2_rin", "unit": "street"}
    ])
}

fn converted(version: &Version) -> Vec<(String, TableAsset)> {
    let plan = Episode.plan(version).unwrap();
    let names = plan.names().to_vec();
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name, plan.convert(index).unwrap()))
        .collect()
}

fn bytes_of(asset: &TableAsset) -> &[u8] {
    match &asset.asset.members[0].content {
        Content::File { bytes, .. } => bytes,
        _ => panic!("not a file"),
    }
}

fn document(asset: &TableAsset) -> EpisodeDocument {
    EpisodeDocument::from_bytes(bytes_of(asset)).unwrap()
}

/// The only episode of a version.
fn only(version: &Version) -> EpisodeDocument {
    let episodes = converted(version);
    assert_eq!(
        episodes.len(),
        1,
        "{:?}",
        episodes.iter().map(|e| &e.0).collect::<Vec<_>>()
    );
    document(&episodes[0].1)
}

fn kinds(document: &EpisodeDocument) -> Vec<WarningKind> {
    document.warnings.iter().map(|w| w.kind).collect()
}

fn motion(asset: &str, role: &str) -> MotionRef {
    MotionRef {
        asset: asset.to_owned(),
        role: role.to_owned(),
    }
}

#[test]
fn an_episode_names_everything_it_uses() {
    let scene = json!({
        "AppearCharacters": [{"Character2dId": 17, "CostumeType": "skin"}],
        "FirstLayout": [{"Character2dId": 17, "MotionName": "wave", "FacialName": "smile"}],
        "TalkData": [{
            "Motions": [{"Character2dId": 17, "MotionName": "BOW", "FacialName": ""}],
            "Voices": [{"VoiceId": "v1 ", "Character2dId": 17}]
        }],
        "FirstBackground": "bg",
        "FirstBgm": "bgm1",
        "SoundData": [{"Bgm": "", "Se": "se00001", "SeBundleName": ""}],
        "NeedBundleNames": ["scenario/movie/m"],
        "SpecialEffectData": [
            {"EffectType": 15, "StringVal": "spark", "StringValSub": "scenario/effect/spark"},
            {"EffectType": 22, "StringVal": "hologram"},
            {"EffectType": 37, "IntVal": 42}
        ]
    });
    let version = Version::default()
        .table("unitStories", unit_table(&["s", "not_in_the_version"]))
        .table("character2ds", characters())
        .scene(UNIT, "s", scene)
        .with_roles(
            "live2d/model/skin",
            "skin",
            "live2d-model",
            &["motion/wave"],
        )
        .with_roles(
            "live2d/motion/miku_motion_base",
            "base",
            "live2d-motions",
            &["motion/bow", "facial/smile", "facial/wave"],
        )
        .plain("scenario/background/bg", "bg", "texture")
        .plain("scenario/movie/m", "b", "video")
        .plain("scenario/movie/m", "a", "video")
        .cues("sound/scenario/bgm/bgm1", "bgm1", &["bgm1"])
        .cues("sound/scenario/voice/s", "s", &["v1 "])
        .cues("sound/scenario/se/se_pack00001", "pack", &["se00001"]);

    let episodes = converted(&version);
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].0, "episode/unit/c/1");
    let doc = document(&episodes[0].1);

    assert_eq!(doc.format, "episode");
    assert_eq!(doc.story.story_type, StoryType::Unit);
    assert_eq!(doc.story.selector, "unit:c/1");
    assert_eq!(
        (
            doc.story.story_id,
            doc.story.episode_no,
            doc.story.episode_id
        ),
        (1, 1, 100)
    );
    assert_eq!(doc.scenario, format!("{UNIT}/s"));

    let character = &doc.characters[&17];
    assert_eq!(character.asset_name.as_deref(), Some("miku"));
    assert_eq!(
        character.motion_set.as_deref(),
        Some("live2d/motion/miku_motion_base/base")
    );
    let costume = &character.costumes[0];
    assert_eq!(costume.costume_type, "skin");
    assert_eq!(costume.model.as_deref(), Some("live2d/model/skin/skin"));
    let base = "live2d/motion/miku_motion_base/base";
    assert_eq!(
        costume.motions["wave"],
        motion("live2d/model/skin/skin", "motion/wave"),
        "the model wins over the motion set"
    );
    assert_eq!(costume.motions["BOW"], motion(base, "motion/bow"));
    assert_eq!(costume.motions["smile"], motion(base, "facial/smile"));

    assert_eq!(doc.backgrounds["bg"], "scenario/background/bg/bg");
    assert_eq!(
        doc.movies["m"],
        ["scenario/movie/m/a", "scenario/movie/m/b"]
    );
    assert_eq!(doc.bgm["bgm1"].asset, "sound/scenario/bgm/bgm1/bgm1");
    assert_eq!(doc.bgm["bgm1"].waveforms, [waveform("bgm1")]);
    assert_eq!(
        doc.se["se00001"].asset,
        "sound/scenario/se/se_pack00001/pack"
    );
    assert_eq!(doc.voices["v1 "].cue, "v1 ");
    assert_eq!(doc.effects["spark"], "scenario/effect/spark");
    assert_eq!(doc.effects["hologram"], "scenario/effect/hologram");
    assert_eq!(doc.music_videos, ["42"]);
    assert!(doc.area_talk.is_none());
    assert_eq!(doc.warnings, []);

    let references = &episodes[0].1.references;
    for name in [
        "scenario/unitstory/c/s",
        "live2d/model/skin/skin",
        "live2d/motion/miku_motion_base/base",
        "scenario/background/bg/bg",
        "scenario/movie/m/a",
        "sound/scenario/bgm/bgm1/bgm1",
    ] {
        assert!(references.iter().any(|known| known == name), "{name}");
    }
}

fn part_voice_scene(character: i64) -> Value {
    json!({
        "AppearCharacters": [{"Character2dId": character, "CostumeType": "skin"}],
        "TalkData": [{"Voices": [{"VoiceId": "partvoice_a", "Character2dId": character}]}]
    })
}

#[test]
fn a_part_voice_searches_its_character_then_every_holder() {
    let version = |character_holds: bool| {
        let v = Version::default()
            .table("unitStories", unit_table(&["s"]))
            .table("character2ds", characters())
            .scene(UNIT, "s", part_voice_scene(17))
            .cues("sound/scenario/part_voice/zzz", "zzz", &["partvoice_a"])
            .cues(
                "sound/scenario/voice/part_voice_aaa",
                "aaa",
                &["partvoice_a"],
            );
        if character_holds {
            v.cues(
                "sound/scenario/part_voice/miku_light",
                "own",
                &["partvoice_a"],
            )
        } else {
            v
        }
    };
    assert_eq!(
        only(&version(true)).voices["partvoice_a"].asset,
        "sound/scenario/part_voice/miku_light/own"
    );
    // Bundle names sort `sound/scenario/part_voice/` before `sound/scenario/voice/`.
    assert_eq!(
        only(&version(false)).voices["partvoice_a"].asset,
        "sound/scenario/part_voice/zzz/zzz"
    );
}

#[test]
fn a_bundle_tries_the_asset_of_the_scene_first() {
    let scene = json!({"TalkData": [{"Voices": [{"VoiceId": "v", "Character2dId": 0}]}]});
    let version = Version::default()
        .table("unitStories", unit_table(&["s"]))
        .scene(UNIT, "s", scene)
        .cues("sound/scenario/voice/s", "a_other", &["v"])
        .cues("sound/scenario/voice/s", "s", &["v"]);
    assert_eq!(only(&version).voices["v"].asset, "sound/scenario/voice/s/s");
}

#[test]
fn a_card_episode_is_found_in_either_bundle() {
    let scene = json!({"TalkData": [{"Voices": [{"VoiceId": "v", "Character2dId": 0}]}]});
    let version = Version::default()
        .table(
            "cardEpisodes",
            json!([
                {"id": 7, "cardId": 5, "title": "A", "scenarioId": "s1", "cardEpisodePartType": "first_part"},
                {"id": 8, "cardId": 5, "title": "B", "scenarioId": "s2", "cardEpisodePartType": "second_part"}
            ]),
        )
        .table("cards", json!([{"id": 5, "assetbundleName": "res"}]))
        .scene("character/member/res", "s1", scene.clone())
        .scene("character/member_scenario/res", "s2", scene)
        .cues("sound/card_scenario/voice/s1", "s1", &["v"])
        .cues("sound/card_scenario/voice/s2", "s2", &["v"]);
    let episodes = converted(&version);
    let names: Vec<&str> = episodes.iter().map(|e| e.0.as_str()).collect();
    assert_eq!(names, ["episode/card/5/first", "episode/card/5/second"]);
    let second = document(&episodes[1].1);
    assert_eq!(second.story.episode_no, 2);
    assert_eq!(second.scenario, "character/member_scenario/res/s2");
    assert_eq!(second.voices["v"].asset, "sound/card_scenario/voice/s2/s2");
}

#[test]
fn an_area_conversation_lists_every_candidate() {
    let scene = json!({
        "AppearCharacters": [{"Character2dId": 17, "CostumeType": "skin"}],
        "FirstLayout": [{"Character2dId": 17, "MotionName": "wave"}]
    });
    let version = Version::default()
        .table(
            "actionSets",
            json!([
                {"id": 305, "areaId": 8, "scenarioId": "as", "scriptId": "script"},
                {"id": 306, "areaId": 8, "scriptId": "no_scene"}
            ]),
        )
        .table("character2ds", characters())
        .table(
            "costume2ds",
            json!([
                {"costume2dGroupId": 3, "character2dId": 17, "fromMmddhh": "120100", "toMmddhh": "010523", "live2dAssetbundleName": "winter"},
                {"costume2dGroupId": 1, "character2dId": 17, "fromMmddhh": "010100", "toMmddhh": "123123", "live2dAssetbundleName": "other"},
                {"costume2dGroupId": 3, "character2dId": 17, "fromMmddhh": "010100", "toMmddhh": "123123", "live2dAssetbundleName": ""}
            ]),
        )
        .scene("scenario/actionset/group3", "as", scene)
        .action_set("actionset/group3", "script", &[(17, 3), (17, 1)])
        .plain("area/common_bg", "bg_area_8", "texture")
        .plain("area/common_bg", "bg_area_9", "texture")
        .cues("sound/area_bgm/normal_area", "normal_area", &["BGM_AREA_2"])
        .with_roles("live2d/model/skin", "skin", "live2d-model", &[])
        .with_roles("live2d/model/winter", "winter", "live2d-model", &["motion/wave"])
        .with_roles("live2d/motion/miku_motion_base", "base", "live2d-motions", &[]);

    let episodes = converted(&version);
    assert_eq!(episodes.len(), 1);
    assert_eq!(episodes[0].0, "episode/area/8/305");
    let doc = document(&episodes[0].1);
    assert_eq!(doc.story.selector, "area:8/305");
    let talk = doc.area_talk.as_ref().unwrap();
    assert_eq!((talk.action_set_id, talk.area_id), (305, 8));
    assert_eq!(talk.background.as_deref(), Some("bg_area_8"));
    assert_eq!(talk.bgm.as_deref(), Some("BGM_AREA_2"));
    assert_eq!(doc.backgrounds["bg_area_8"], "area/common_bg/bg_area_8");
    assert_eq!(
        doc.bgm["BGM_AREA_2"].asset,
        "sound/area_bgm/normal_area/normal_area"
    );

    let costumes = &doc.characters[&17].area_costumes;
    assert_eq!(costumes.len(), 2, "only the group of the first detail");
    assert_eq!(
        (costumes[0].from.as_str(), costumes[0].to.as_str()),
        ("120100", "010523")
    );
    assert_eq!(
        costumes[0].model.as_deref(),
        Some("live2d/model/winter/winter")
    );
    assert_eq!(
        costumes[0].motions["wave"],
        motion("live2d/model/winter/winter", "motion/wave")
    );
    assert_eq!(costumes[1].model, None);
    assert!(costumes[1].motions.is_empty());
    assert_eq!(doc.warnings, []);
}

#[test]
fn an_area_playlist_chooses_the_music() {
    let version = Version::default()
        .table(
            "actionSets",
            json!([{"id": 1, "areaId": 2, "scenarioId": "as", "scriptId": "as"}]),
        )
        .table(
            "areaPlaylists",
            json!([
                {"areaId": 3, "assetbundleName": "wrong", "bgmName": "x"},
                {"areaId": 2, "assetbundleName": "park", "bgmName": "park_a"},
                {"areaId": 2, "assetbundleName": "later", "bgmName": "y"}
            ]),
        )
        .scene("scenario/actionset/group0", "as", json!({}))
        .action_set("actionset/group0", "as", &[])
        .plain("area/common_bg", "bg_area_2", "texture")
        .cues("sound/area_bgm/park", "park", &["park_a"]);
    let doc = only(&version);
    assert_eq!(doc.area_talk.unwrap().bgm.as_deref(), Some("park_a"));
    assert_eq!(doc.bgm["park_a"].asset, "sound/area_bgm/park/park");
}

#[test]
fn an_area_conversation_without_its_action_set_keeps_the_scene_costumes() {
    let scene = json!({"AppearCharacters": [{"Character2dId": 17, "CostumeType": "skin"}]});
    let version = Version::default()
        .table(
            "actionSets",
            json!([{"id": 1, "areaId": 2, "scenarioId": "as", "scriptId": "gone"}]),
        )
        .table("character2ds", characters())
        .scene("scenario/actionset/group0", "as", scene)
        .plain("area/common_bg", "bg_area_2", "texture")
        .cues("sound/area_bgm/normal_area", "a", &["BGM_AREA_2"])
        .with_roles("live2d/model/skin", "skin", "live2d-model", &[])
        .with_roles(
            "live2d/motion/miku_motion_base",
            "base",
            "live2d-motions",
            &[],
        );
    let doc = only(&version);
    assert_eq!(kinds(&doc), [WarningKind::AssetMissing]);
    assert!(doc.warnings[0].detail.contains("gone"));
    let character = &doc.characters[&17];
    assert_eq!(
        character.costumes[0].model.as_deref(),
        Some("live2d/model/skin/skin")
    );
    assert!(character.area_costumes.is_empty());
}

#[test]
fn backgrounds_choose_their_file_and_add_their_movies() {
    let scene = json!({
        "FirstBackground": "one",
        "NeedBundleNames": ["scenario/movie/own"],
        "SpecialEffectData": [
            {"EffectType": 7, "StringVal": "two", "StringValSub": "b"},
            {"EffectType": 19, "StringVal": "own"}
        ]
    });
    let version = Version::default()
        .table("unitStories", unit_table(&["s"]))
        .scene(UNIT, "s", scene)
        .plain("scenario/background/one", "whatever", "texture")
        .plain("scenario/background/two", "a", "texture")
        .plain("scenario/background/two", "b", "texture")
        .background_movies(
            "scenario/background/two",
            "prefab",
            &["scenario/movie/own", "scenario/movie/extra"],
        )
        .plain("scenario/movie/own", "v", "video")
        .plain("scenario/movie/extra", "v", "video");
    let doc = only(&version);
    assert_eq!(doc.backgrounds["one"], "scenario/background/one/whatever");
    assert_eq!(doc.backgrounds["two/b"], "scenario/background/two/b");
    assert_eq!(
        doc.movies.keys().map(String::as_str).collect::<Vec<_>>(),
        ["extra", "own"]
    );
    assert_eq!(doc.warnings, []);
}

#[test]
fn sound_effects_search_the_named_bundle_then_the_rules() {
    let scene = json!({"SoundData": [
        {"Se": "custom", "SeBundleName": "sound/x"},
        {"Se": "se_event_a_1_001"},
        {"Se": "packed"}
    ]});
    let version = Version::default()
        .table(
            "eventStories",
            json!([{"eventId": 9, "assetbundleName": "ev", "eventStoryEpisodes": [
                {"id": 1, "episodeNo": 1, "title": "T", "scenarioId": "s"}
            ]}]),
        )
        .scene("event_story/ev/scenario", "s", scene)
        .cues("sound/x", "x", &["custom"])
        .cues("event_story/a_1/scenario_se", "e", &["se_event_a_1_001"])
        .cues("event_story/ev/scenario_se", "own", &["other"])
        .cues("sound/scenario/se/se_pack00001", "p", &["packed"]);
    let doc = only(&version);
    assert_eq!(doc.se["custom"].asset, "sound/x/x");
    assert_eq!(
        doc.se["se_event_a_1_001"].asset,
        "event_story/a_1/scenario_se/e"
    );
    assert_eq!(doc.se["packed"].asset, "sound/scenario/se/se_pack00001/p");
    assert_eq!(doc.warnings, []);
}

#[test]
fn an_empty_costume_takes_the_casual_model() {
    let scene = json!({"AppearCharacters": [{"Character2dId": 0, "CostumeType": ""}]});
    let version = Version::default()
        .table("unitStories", unit_table(&["s"]))
        .table("character2ds", characters())
        .scene(UNIT, "s", scene)
        .with_roles("live2d/model/001_casual", "m", "live2d-model", &[])
        .with_roles(
            "live2d/motion/first_motion_base",
            "b",
            "live2d-motions",
            &[],
        );
    let doc = only(&version);
    assert_eq!(doc.characters[&0].costumes[0].costume_type, "001_casual");
    assert_eq!(doc.characters[&0].asset_name.as_deref(), Some("first"));
    assert_eq!(doc.warnings, []);
}

#[test]
fn what_cannot_be_resolved_is_warned() {
    let scene = json!({
        "AppearCharacters": [
            {"Character2dId": 17, "CostumeType": "gone"},
            {"Character2dId": 99, "CostumeType": "gone2"}
        ],
        "FirstLayout": [
            {"Character2dId": 17, "MotionName": "nope"},
            {"Character2dId": 50, "MotionName": "stranger"}
        ],
        "FirstBackground": "nothing",
        "FirstBgm": "silent",
        "SpecialEffectData": [
            {"EffectType": 45},
            {"EffectType": 15, "StringVal": "lost"}
        ]
    });
    let version = Version::default()
        .table("unitStories", unit_table(&["s"]))
        .table("character2ds", characters())
        .scene(UNIT, "s", scene)
        .with_roles("live2d/motion/miku_motion_base", "b", "live2d-motions", &[])
        .cues("sound/scenario/bgm/silent", "silent", &["another"]);
    let doc = only(&version);
    let found = kinds(&doc);
    for kind in [
        WarningKind::UnknownCharacter,
        WarningKind::UnknownEffectType,
        WarningKind::AssetMissing,
        WarningKind::ModelMissing,
        WarningKind::TableMismatch,
        WarningKind::MotionNotFound,
        WarningKind::CueNotFound,
    ] {
        assert!(found.contains(&kind), "{kind:?} in {found:?}");
    }
    assert_eq!(doc.characters[&17].costumes[0].model, None);
    assert!(doc.bgm.is_empty());
}

#[test]
fn a_motion_set_that_is_not_in_the_version_is_warned() {
    let scene = json!({"AppearCharacters": [{"Character2dId": 17, "CostumeType": "skin"}]});
    let version = Version::default()
        .table("unitStories", unit_table(&["s"]))
        .table("character2ds", characters())
        .scene(UNIT, "s", scene)
        .with_roles("live2d/model/skin", "m", "live2d-model", &[]);
    assert_eq!(kinds(&only(&version)), [WarningKind::MotionSetUnresolved]);
}

#[test]
fn a_scene_that_is_not_an_object_is_an_error() {
    let version = Version::default()
        .table("unitStories", unit_table(&["s"]))
        .scene(UNIT, "s", json!([1]));
    let plan = Episode.plan(&version).unwrap();
    assert_eq!(plan.convert(0).unwrap_err().code, "malformed");
}

#[test]
fn a_table_that_cannot_be_read_fails_the_plan() {
    let version = Version::default().table("cards", json!({"not": "rows"}));
    assert!(Episode.plan(&version).is_err());
}

#[test]
fn the_same_inputs_give_the_same_bytes() {
    let scene = json!({
        "AppearCharacters": [
            {"Character2dId": 18, "CostumeType": "b"},
            {"Character2dId": 17, "CostumeType": "a"}
        ],
        "FirstBackground": "bg"
    });
    let version = Version::default()
        .table("unitStories", unit_table(&["s"]))
        .table("character2ds", characters())
        .scene(UNIT, "s", scene);
    let first = converted(&version);
    let second = converted(&version);
    assert_eq!(bytes_of(&first[0].1), bytes_of(&second[0].1));
}
