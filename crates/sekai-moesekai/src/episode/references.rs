//! What a scene data document refers to.
//!
//! Arrays are walked in index order and every list keeps first-seen order without repeats, so the
//! result is a function of the document. A missing field reads as empty.

use sekai_format::episode::{Warning, WarningKind};
use serde_json::Value;

use super::rules::{self, BACKGROUND_PREFIX, MOVIE_PREFIX};

/// The highest `SpecialEffectType` the client handles.
const MAX_EFFECT_TYPE: i64 = 44;
const CHANGE_BACKGROUND: i64 = 7;
const PLAY_EFFECT: i64 = 15;
const STOP_EFFECT: i64 = 16;
const MOVIE: i64 = 19;
const ATTACH_SHADER: i64 = 22;
const FULL_SCREEN_TEXT: i64 = 24;
const MUSIC_VIDEO: i64 = 37;
/// The one character shader that has a bundle.
const HOLOGRAM: &str = "hologram";
const HOLOGRAM_BUNDLE: &str = "scenario/effect/hologram";

/// The references of one scene data.
#[derive(Debug, Default)]
pub(super) struct References {
    pub(super) characters: Vec<CharacterRefs>,
    pub(super) backgrounds: Vec<Background>,
    pub(super) movies: Vec<String>,
    pub(super) voices: Vec<Voice>,
    pub(super) bgm: Vec<String>,
    pub(super) se: Vec<SoundEffect>,
    /// Effect name and the bundle that holds it.
    pub(super) effects: Vec<(String, String)>,
    pub(super) music_videos: Vec<String>,
    pub(super) warnings: Vec<Warning>,
}

/// What the scene data uses of one character.
#[derive(Debug)]
pub(super) struct CharacterRefs {
    pub(super) id: i64,
    pub(super) costumes: Vec<String>,
    pub(super) motions: Vec<String>,
}

/// A background: the bundle `name`, the file `file` in it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Background {
    pub(super) name: String,
    pub(super) file: String,
}

impl Background {
    fn new(name: &str, file: &str) -> Self {
        Self {
            name: name.to_owned(),
            file: file.to_owned(),
        }
    }

    /// The key of the background in the document.
    pub(super) fn key(&self) -> String {
        if self.file == self.name {
            self.name.clone()
        } else {
            format!("{}/{}", self.name, self.file)
        }
    }
}

/// A voice and who says it, 0 for nobody.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Voice {
    pub(super) id: String,
    pub(super) character: i64,
}

/// A sound effect and the bundle the scene data names for it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct SoundEffect {
    pub(super) cue: String,
    pub(super) bundle: Option<String>,
}

/// Extracts every reference of a scene data.
pub(super) fn extract(scene: &Value) -> References {
    let mut refs = References::default();
    for set in array(scene, "AppearCharacters") {
        refs.appear(set);
    }
    for key in ["FirstLayout", "LayoutData"] {
        for (i, layout) in array(scene, key).iter().enumerate() {
            refs.layout(layout, &format!("{key}[{i}]"));
        }
    }
    for (i, talk) in array(scene, "TalkData").iter().enumerate() {
        refs.talk(talk, i);
    }
    if let Some(first) = named(scene, "FirstBackground") {
        add(&mut refs.backgrounds, Background::new(first, first));
    }
    for bundle in strings(scene, "NeedBundleNames") {
        refs.needed_bundle(bundle);
    }
    if let Some(video) = named(scene, "EpisodeMusicVideoId") {
        add(&mut refs.music_videos, video.to_owned());
    }
    for (i, effect) in array(scene, "SpecialEffectData").iter().enumerate() {
        refs.effect(effect, i);
    }
    for bundle in strings(scene, "IncludeSoundDataBundleNames") {
        add(&mut refs.effects, effect_of_bundle(None, bundle));
    }
    if let Some(first) = named(scene, "FirstBgm") {
        add(&mut refs.bgm, first.to_owned());
    }
    for sound in array(scene, "SoundData") {
        refs.sound(sound);
    }
    refs
}

impl References {
    fn character_mut(&mut self, id: i64) -> Option<&mut CharacterRefs> {
        self.characters.iter_mut().find(|known| known.id == id)
    }

    fn appear(&mut self, set: &Value) {
        let id = int(set, "Character2dId");
        let costume =
            named(set, "CostumeType").map_or_else(|| rules::empty_costume(id), str::to_owned);
        match self.character_mut(id) {
            Some(known) => add(&mut known.costumes, costume),
            None => self.characters.push(CharacterRefs {
                id,
                costumes: vec![costume],
                motions: Vec::new(),
            }),
        }
    }

    fn layout(&mut self, layout: &Value, context: &str) {
        let character = self.character_mut(int(layout, "Character2dId"));
        if let (Some(character), Some(costume)) = (character, named(layout, "CostumeType")) {
            add(&mut character.costumes, costume.to_owned());
        }
        self.motions(layout, context);
    }

    fn talk(&mut self, talk: &Value, index: usize) {
        for (i, motion) in array(talk, "Motions").iter().enumerate() {
            self.motions(motion, &format!("TalkData[{index}].Motions[{i}]"));
        }
        for voice in array(talk, "Voices") {
            if let Some(id) = named(voice, "VoiceId") {
                let character = int(voice, "Character2dId");
                add(
                    &mut self.voices,
                    Voice {
                        id: id.to_owned(),
                        character,
                    },
                );
            }
        }
    }

    /// Adds the motion and facial names of `holder` to its character.
    fn motions(&mut self, holder: &Value, context: &str) {
        let id = int(holder, "Character2dId");
        let names: Vec<&str> = ["MotionName", "FacialName"]
            .into_iter()
            .filter_map(|key| named(holder, key))
            .collect();
        if names.is_empty() {
            return;
        }
        let Some(character) = self.character_mut(id) else {
            self.warnings.push(Warning::new(
                WarningKind::UnknownCharacter,
                format!("{context}: Character2dId {id} is not in AppearCharacters ({names:?})"),
            ));
            return;
        };
        for name in names {
            add(&mut character.motions, name.to_owned());
        }
    }

    fn needed_bundle(&mut self, bundle: &str) {
        if let Some(name) = bundle.strip_prefix(BACKGROUND_PREFIX) {
            add(&mut self.backgrounds, Background::new(name, name));
        } else if let Some(name) = bundle.strip_prefix(MOVIE_PREFIX) {
            add(&mut self.movies, name.to_owned());
        }
    }

    fn effect(&mut self, effect: &Value, index: usize) {
        let kind = int(effect, "EffectType");
        let value = named(effect, "StringVal");
        let sub = named(effect, "StringValSub");
        match (kind, value, sub) {
            (CHANGE_BACKGROUND, Some(name), file) => {
                let background = Background::new(name, file.unwrap_or(name));
                add(&mut self.backgrounds, background);
            }
            (MOVIE, Some(name), _) => add(&mut self.movies, name.to_owned()),
            (FULL_SCREEN_TEXT, _, Some(voice)) => add(
                &mut self.voices,
                Voice {
                    id: voice.to_owned(),
                    character: 0,
                },
            ),
            (MUSIC_VIDEO, _, _) => add(&mut self.music_videos, int(effect, "IntVal").to_string()),
            (PLAY_EFFECT | STOP_EFFECT, name, Some(bundle)) => {
                add(&mut self.effects, effect_of_bundle(name, bundle));
            }
            (PLAY_EFFECT, name, None) => self.warnings.push(Warning::new(
                WarningKind::AssetMissing,
                format!("SpecialEffectData[{index}]: EffectType 15 {name:?} names no bundle"),
            )),
            (ATTACH_SHADER, Some(HOLOGRAM), _) => add(
                &mut self.effects,
                effect_of_bundle(Some(HOLOGRAM), HOLOGRAM_BUNDLE),
            ),
            (other, _, _) if !(0..=MAX_EFFECT_TYPE).contains(&other) => {
                self.warnings.push(Warning::new(
                    WarningKind::UnknownEffectType,
                    format!("SpecialEffectData[{index}]: EffectType {other}"),
                ));
            }
            _ => {}
        }
    }

    fn sound(&mut self, sound: &Value) {
        if let Some(name) = named(sound, "Bgm") {
            add(&mut self.bgm, name.to_owned());
        }
        if let Some(cue) = named(sound, "Se") {
            add(
                &mut self.se,
                SoundEffect {
                    cue: cue.to_owned(),
                    bundle: named(sound, "SeBundleName").map(str::to_owned),
                },
            );
        }
    }
}

/// Adds `item` unless the list has it, which keeps the order of first appearance.
fn add<T: PartialEq>(list: &mut Vec<T>, item: T) {
    if !list.contains(&item) {
        list.push(item);
    }
}

/// An effect named `name`, or by the last segment of its bundle when it has no name.
fn effect_of_bundle(name: Option<&str>, bundle: &str) -> (String, String) {
    let last_segment = bundle.rsplit('/').next().unwrap_or(bundle);
    (name.unwrap_or(last_segment).to_owned(), bundle.to_owned())
}

fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// The nonempty strings of the array `key`.
fn strings<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a str> {
    array(value, key)
        .iter()
        .filter_map(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// The string `key`, unless it is missing or empty.
fn named<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

fn int(value: &Value, key: &str) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(0)
}
