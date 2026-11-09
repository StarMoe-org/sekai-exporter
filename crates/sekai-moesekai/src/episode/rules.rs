//! How the client names the bundles of a story.

/// The prefix of background bundles.
pub(super) const BACKGROUND_PREFIX: &str = "scenario/background/";
/// The prefix of movie bundles.
pub(super) const MOVIE_PREFIX: &str = "scenario/movie/";
const ACTION_SET_SCENARIO_PREFIX: &str = "scenario/actionset/";
const TUTORIAL_SCENARIO: &str = "scenario/tutorial_story";
/// The bundle of the backgrounds of area conversations.
pub(super) const AREA_BACKGROUND: &str = "area/common_bg";
/// The bundle name and cue an area without a playlist plays.
const DEFAULT_AREA_BGM: (&str, &str) = ("normal_area", "BGM_AREA_2");
const SE_PACK: &str = "sound/scenario/se/se_pack00001";
const SE_PACK_B: &str = "sound/scenario/se/se_pack00001_b";
/// The highest number of a sound effect in [`SE_PACK`].
const SE_PACK_LAST: u32 = 528;
/// The packs searched for a cue that no rule places.
pub(super) const FALLBACK_SE: [&str; 2] = [SE_PACK, SE_PACK_B];
/// The prefix of a voice shared by the episodes of a character.
const PART_VOICE_PREFIX: &str = "partvoice_";

pub(super) fn unit_scenario(chapter: &str) -> String {
    format!("scenario/unitstory/{chapter}")
}

pub(super) fn event_scenario(event: &str) -> String {
    format!("event_story/{event}/scenario")
}

pub(super) fn event_se(event: &str) -> String {
    format!("event_story/{event}/scenario_se")
}

pub(super) fn card_scenarios(card: &str) -> Vec<String> {
    vec![
        format!("character/member/{card}"),
        format!("character/member_scenario/{card}"),
    ]
}

pub(super) fn special_scenarios(
    scenario_id: &str,
    story: &str,
    episode: Option<&str>,
) -> Vec<String> {
    let mut candidates = Vec::new();
    match scenario_id {
        "op_02" => candidates.push(TUTORIAL_SCENARIO.to_owned()),
        "op_02area" => candidates.push(format!("{ACTION_SET_SCENARIO_PREFIX}group0")),
        _ => {}
    }
    for name in [episode, Some(story)].into_iter().flatten() {
        let bundle = format!("scenario/special/{name}");
        if !name.is_empty() && !candidates.contains(&bundle) {
            candidates.push(bundle);
        }
    }
    candidates
}

pub(super) fn action_set_scenario(id: i64) -> String {
    format!("{ACTION_SET_SCENARIO_PREFIX}group{}", id / 100)
}

pub(super) fn action_set_data(id: i64) -> String {
    format!("actionset/group{}", id / 100)
}

pub(super) fn area_background(area: i64) -> String {
    format!("bg_area_{area}")
}

/// The bundle and cue of the music of an area, given its first playlist row.
pub(super) fn area_bgm(row: Option<(&str, &str)>) -> (String, String) {
    let (name, cue) = row
        .filter(|(name, cue)| !name.is_empty() && !cue.is_empty())
        .unwrap_or(DEFAULT_AREA_BGM);
    (format!("sound/area_bgm/{name}"), cue.to_owned())
}

pub(super) fn scenario_bgm(name: &str) -> String {
    format!("sound/scenario/bgm/{name}")
}

/// The voice bundle of a scene data, which depends on the bundle that holds the scene data.
pub(super) fn voice_bundle(scenario_bundle: &str, scenario_id: &str) -> String {
    let family = if scenario_bundle.starts_with("character/member/")
        || scenario_bundle.starts_with("character/member_scenario/")
    {
        "card_scenario"
    } else if scenario_bundle.starts_with(ACTION_SET_SCENARIO_PREFIX) {
        "actionset"
    } else if scenario_bundle == TUTORIAL_SCENARIO {
        "tutorial_scenario"
    } else {
        "scenario"
    };
    format!("sound/{family}/voice/{scenario_id}")
}

pub(super) fn is_part_voice(voice: &str) -> bool {
    voice.starts_with(PART_VOICE_PREFIX)
}

pub(super) fn is_part_voice_bundle(bundle: &str) -> bool {
    bundle.starts_with("sound/scenario/part_voice/")
        || bundle.starts_with("sound/scenario/voice/part_voice_")
}

pub(super) fn part_voice_bundle(asset_name: &str, unit: &str) -> String {
    let key = format!("{asset_name}_{unit}");
    if key.starts_with("v2_") || key.starts_with("clb") {
        format!("sound/scenario/voice/part_voice_{key}")
    } else {
        format!("sound/scenario/part_voice/{key}")
    }
}

/// The bundles that may hold the sound effect `cue`, in order. Empty when no rule places it.
pub(super) fn se_bundles(cue: &str) -> Vec<String> {
    let digits = |s: &str| s.len() == 5 && s.bytes().all(|b| b.is_ascii_digit());
    if let Some(number) = cue.strip_prefix("se").filter(|n| digits(n)) {
        let pack = if number.parse().unwrap_or(u32::MAX) <= SE_PACK_LAST {
            SE_PACK
        } else {
            SE_PACK_B
        };
        return vec![pack.to_owned()];
    }
    let suffixed = cue
        .strip_prefix("se")
        .and_then(|n| n.strip_suffix("_b"))
        .is_some_and(digits);
    if suffixed || cue.starts_with("se_walk_") || cue.starts_with("se_run_") {
        return vec![SE_PACK_B.to_owned()];
    }
    if let Some(rest) = cue.strip_prefix("se_event_")
        && let Some((name, number)) = rest.rsplit_once('_')
        && !name.is_empty()
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
    {
        return vec![event_se(&format!("event_{name}")), event_se(name)];
    }
    Vec::new()
}

/// The model key of a costume that the scene data leaves empty.
pub(super) fn empty_costume(id: i64) -> String {
    let id = if id == 0 { 1 } else { id };
    format!("{id:03}_casual")
}

/// The row of `character2ds` that the client reads for a character.
pub(super) fn table_character(id: i64) -> i64 {
    id.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_effects_are_placed_by_their_names() {
        assert_eq!(se_bundles("se00528"), [SE_PACK]);
        assert_eq!(se_bundles("se00529"), [SE_PACK_B]);
        assert_eq!(se_bundles("se00012_b"), [SE_PACK_B]);
        assert_eq!(se_bundles("se_run_a"), [SE_PACK_B]);
        assert_eq!(
            se_bundles("se_event_a_1_001"),
            [
                "event_story/event_a_1/scenario_se",
                "event_story/a_1/scenario_se"
            ]
        );
        for other in ["se0001", "bgm00001", "se_event_", "se_event_x_y"] {
            assert!(se_bundles(other).is_empty(), "{other}");
        }
    }

    #[test]
    fn voices_follow_the_bundle_of_the_scene_data() {
        for (scenario, family) in [
            ("scenario/unitstory/a", "scenario"),
            ("event_story/a/scenario", "scenario"),
            ("character/member/a", "card_scenario"),
            ("character/member_scenario/a", "card_scenario"),
            ("scenario/actionset/group0", "actionset"),
            ("scenario/tutorial_story", "tutorial_scenario"),
        ] {
            assert_eq!(
                voice_bundle(scenario, "x"),
                format!("sound/{family}/voice/x")
            );
        }
    }

    #[test]
    fn part_voices_have_two_places() {
        assert_eq!(part_voice_bundle("a", "b"), "sound/scenario/part_voice/a_b");
        assert_eq!(
            part_voice_bundle("v2_a", "b"),
            "sound/scenario/voice/part_voice_v2_a_b"
        );
        assert_eq!(
            part_voice_bundle("clb1_a", "b"),
            "sound/scenario/voice/part_voice_clb1_a_b"
        );
    }

    #[test]
    fn special_stories_try_their_exceptions_first() {
        assert_eq!(
            special_scenarios("op_02", "s", Some("e")),
            [
                TUTORIAL_SCENARIO,
                "scenario/special/e",
                "scenario/special/s"
            ]
        );
        assert_eq!(special_scenarios("x", "", Some("")), Vec::<String>::new());
    }

    #[test]
    fn an_area_without_a_playlist_plays_the_default_music() {
        assert_eq!(
            area_bgm(None),
            (
                "sound/area_bgm/normal_area".to_owned(),
                "BGM_AREA_2".to_owned()
            )
        );
        assert_eq!(area_bgm(Some(("a", ""))).1, "BGM_AREA_2");
        assert_eq!(
            area_bgm(Some(("a", "c"))),
            ("sound/area_bgm/a".to_owned(), "c".to_owned())
        );
    }

    #[test]
    fn character_ids_of_at_most_one_read_the_first_row() {
        assert_eq!(table_character(0), 1);
        assert_eq!(table_character(366), 366);
        assert_eq!(empty_costume(0), "001_casual");
        assert_eq!(empty_costume(1086), "1086_casual");
    }
}
