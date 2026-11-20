//! Story episodes.
//!
//! [`Episode`] produces the `episode` form, specified in `docs/spec/episode.md`.

mod catalog;
mod references;
mod rules;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

use sekai_convert::tables::{
    AssetEntry, Shared, SharedEntries, TableAsset, TableContext, TableConverter, TablePlan,
};
use sekai_convert::{Asset, Content, ConvertError, Member};
use sekai_format::audio::CueTable;
use sekai_format::episode::{
    AreaCostume, AreaTalk, Character, Costume, Effect, EpisodeDocument, FORMAT, MotionRef, Sound,
    VERSION, Warning, WarningKind,
};
use sekai_format::story::{ActionSet, ScenarioBackground};
use serde::Serialize;
use serde_json::Value;

use catalog::{AreaSource, Character2d, Costume2d, Row};
use references::{CharacterRefs, References};

const AUDIO: &str = "audio";
const SCENARIO: &str = "scenario";
const TEXTURE: &str = "texture";
const VIDEO: &str = "video";
const MODEL: &str = "live2d-model";
const MOTIONS: &str = "live2d-motions";
const ACTION_SET: &str = "action-set";
const SCENARIO_BACKGROUND: &str = "scenario-background";
const SCENARIO_PREFAB: &str = "scenario-prefab";

/// The motions and facials of an asset: lower-case name to role.
type MotionIndex = HashMap<String, String>;
/// The bundles to search for each cue.
type SearchPlan<'r> = BTreeMap<&'r str, Vec<String>>;
type Cache<T> = Mutex<HashMap<String, Arc<T>>>;

/// The episodes of the game data tables.
#[derive(Debug, Clone, Copy, Default)]
pub struct Episode;

impl TableConverter for Episode {
    fn name(&self) -> &'static str {
        "episode"
    }

    fn version(&self) -> u32 {
        3
    }

    fn plan(&self, cx: &dyn TableContext) -> Result<Box<dyn TablePlan>, ConvertError> {
        let catalog = catalog::load(cx)?;
        let located = catalog
            .rows
            .into_iter()
            .filter_map(|row| Entry::locate(cx, row));
        let mut entries = BTreeMap::new();
        for entry in located {
            let name = entry.asset_name();
            if entries.contains_key(&name) {
                return Err(malformed(format!("two episodes are named {name}")));
            }
            entries.insert(name, entry);
        }
        let library = Library::default();
        Ok(Box::new(Plan {
            names: entries.keys().cloned().collect(),
            part_voices: Shared::new("part_voices", library.part_voice_index(cx)?),
            entries: Shared::new("entries", entries),
            character2ds: Shared::new("character2ds", catalog.character2ds),
            costume2ds: Shared::new("costume2ds", catalog.costume2ds),
            library,
        }))
    }
}

/// The last segment of an asset name, without an `@` suffix.
fn stem(name: &str) -> &str {
    let file = name.rsplit('/').next().unwrap_or(name);
    file.split('@').next().unwrap_or(file)
}

fn of_form<'a>(entries: &'a [AssetEntry], form: &'a str) -> impl Iterator<Item = &'a AssetEntry> {
    entries.iter().filter(move |entry| entry.form == form)
}

fn malformed(e: impl std::fmt::Display) -> ConvertError {
    ConvertError::new("malformed", e.to_string())
}

/// An episode whose scene data the version holds.
#[derive(Serialize)]
struct Entry {
    row: Row,
    scenario: String,
    bundle: String,
}

impl Entry {
    /// Finds the scene data of `row`.
    fn locate(cx: &dyn TableContext, row: Row) -> Option<Self> {
        let (scenario, bundle) = row.scenario_bundles.iter().find_map(|bundle| {
            of_form(cx.assets(bundle), SCENARIO)
                .find(|entry| stem(&entry.name).eq_ignore_ascii_case(&row.story.scenario_id))
                .map(|entry| (entry.name.clone(), bundle.clone()))
        })?;
        Some(Self {
            row,
            scenario,
            bundle,
        })
    }

    fn asset_name(&self) -> String {
        format!("episode/{}", self.row.story.selector.replacen(':', "/", 1))
    }
}

/// What is read from the assets of the version, kept for the other episodes by the identity of
/// the asset, so that an episode that finds it kept still reads that identity.
#[derive(Default)]
struct Library {
    cue_tables: Cache<CueTable>,
    motions: Cache<MotionIndex>,
}

/// The value of `key` in `cache`, loaded on first use.
fn cached<T>(
    cache: &Cache<T>,
    key: &str,
    load: impl FnOnce() -> Result<T, ConvertError>,
) -> Result<Arc<T>, ConvertError> {
    let lock = || cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(found) = lock().get(key) {
        return Ok(found.clone());
    }
    let value = Arc::new(load()?);
    lock().insert(key.to_owned(), value.clone());
    Ok(value)
}

impl Library {
    fn cue_table(&self, cx: &dyn TableContext, asset: &str) -> Result<Arc<CueTable>, ConvertError> {
        cached(&self.cue_tables, &cx.identity(asset)?, || {
            CueTable::from_bytes(&cx.member(asset, "cues")?).map_err(malformed)
        })
    }

    fn motion_index(
        &self,
        cx: &dyn TableContext,
        asset: &str,
    ) -> Result<Arc<MotionIndex>, ConvertError> {
        cached(&self.motions, &cx.identity(asset)?, || {
            let mut index = MotionIndex::new();
            // Body motions come before facial motions in a document, so a body motion wins.
            for member in cx.document(asset)?.members {
                let name = member
                    .role
                    .strip_prefix("motion/")
                    .or_else(|| member.role.strip_prefix("facial/"));
                if let Some(name) = name {
                    index
                        .entry(name.to_lowercase())
                        .or_insert(member.role.clone());
                }
            }
            Ok(index)
        })
    }

    /// Cue name to the part voice bundles that hold it, in name order.
    fn part_voice_index(
        &self,
        cx: &dyn TableContext,
    ) -> Result<BTreeMap<String, Vec<String>>, ConvertError> {
        let mut index: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for bundle in cx.bundles() {
            if !rules::is_part_voice_bundle(bundle) {
                continue;
            }
            for entry in of_form(cx.assets(bundle), AUDIO) {
                for cue in &self.cue_table(cx, &entry.name)?.cues {
                    let holders = index.entry(cue.name.clone()).or_default();
                    if !holders.iter().any(|known| known == bundle) {
                        holders.push(bundle.to_owned());
                    }
                }
            }
        }
        Ok(index)
    }
}

/// The episodes of a version. An episode reads the plan only through its shared maps.
struct Plan {
    library: Library,
    names: Vec<String>,
    /// The episode of each asset name.
    entries: Shared<String, Entry>,
    character2ds: Shared<i64, Character2d>,
    costume2ds: Shared<i64, Vec<Costume2d>>,
    /// Cue name to the part voice bundles that hold it.
    part_voices: Shared<String, Vec<String>>,
}

impl Plan {
    /// The part voice bundle of what `character` says, for the episode that `cx` builds.
    fn part_voice_bundle(&self, cx: &dyn TableContext, character: i64) -> Option<String> {
        let row = self.character2ds.get(&character, cx)?;
        Some(rules::part_voice_bundle(
            row.asset_name.as_ref()?,
            row.unit.as_ref()?,
        ))
    }
}

impl TablePlan for Plan {
    fn names(&self) -> &[String] {
        &self.names
    }

    fn convert(&self, index: usize, cx: &dyn TableContext) -> Result<TableAsset, ConvertError> {
        let name = &self.names[index];
        let entry = self.entries.get(name, cx).ok_or_else(|| {
            ConvertError::new("missing_asset", format!("the plan has no episode {name}"))
        })?;
        Builder {
            plan: self,
            cx,
            entry,
            warnings: Vec::new(),
            references: BTreeSet::new(),
        }
        .build()
    }

    fn reusable(&self) -> bool {
        true
    }

    fn shared(&self) -> Vec<&dyn SharedEntries> {
        vec![
            &self.entries,
            &self.character2ds,
            &self.costume2ds,
            &self.part_voices,
        ]
    }
}

/// One episode being built.
struct Builder<'p> {
    plan: &'p Plan,
    cx: &'p dyn TableContext,
    entry: &'p Entry,
    warnings: Vec<Warning>,
    references: BTreeSet<String>,
}

impl<'p> Builder<'p> {
    fn build(mut self) -> Result<TableAsset, ConvertError> {
        let entry = self.entry;
        let area = entry.row.area.as_ref();
        let mut refs = references::extract(&self.scene()?);
        self.warnings.append(&mut refs.warnings);
        self.references.insert(entry.scenario.clone());

        let characters = self.characters(&refs, area)?;
        let backgrounds = self.backgrounds(&refs, area);
        let movies = self.movies(&refs, &backgrounds.bundles)?;
        let bgm = self.music(&refs, area)?;
        let se = self.sound_effects(&refs)?;
        let voices = self.voices(&refs)?;
        let area_talk = area.map(|area| area_talk(area, &backgrounds.textures, &bgm));
        let effects = self.effects(&refs.effects);

        let document = EpisodeDocument {
            format: FORMAT.to_owned(),
            version: VERSION,
            story: entry.row.story.clone(),
            scenario: entry.scenario.clone(),
            characters,
            backgrounds: backgrounds.textures,
            background_prefabs: backgrounds.prefabs,
            bgm,
            se,
            voices,
            effects,
            movies,
            music_videos: refs.music_videos,
            area_talk,
            warnings: self.warnings,
        };
        Ok(TableAsset {
            asset: Asset {
                members: vec![Member {
                    role: "episode".to_owned(),
                    content: Content::File {
                        extension: "json".to_owned(),
                        bytes: document.to_bytes().map_err(malformed)?,
                    },
                }],
            },
            references: self.references.into_iter().collect(),
        })
    }

    fn scene(&self) -> Result<Value, ConvertError> {
        let bytes = self.cx.member(&self.entry.scenario, SCENARIO)?;
        let scene: Value = serde_json::from_slice(&bytes).map_err(malformed)?;
        if scene.is_object() {
            Ok(scene)
        } else {
            Err(malformed("the scene data is not an object"))
        }
    }

    fn assets(&self, bundle: &str) -> &'p [AssetEntry] {
        self.cx.assets(bundle)
    }

    fn warn(&mut self, kind: WarningKind, detail: String) {
        self.warnings.push(Warning::new(kind, detail));
    }

    /// Records that the episode uses `asset`, and gives its name.
    fn refer(&mut self, asset: &str) -> String {
        self.references.insert(asset.to_owned());
        asset.to_owned()
    }

    /// Whether the version holds `bundle`; warns when it does not.
    fn require(&mut self, bundle: &str, what: &str) -> bool {
        let present = !self.assets(bundle).is_empty();
        if !present {
            self.warn(
                WarningKind::AssetMissing,
                format!("{bundle} ({what}) is not in the version"),
            );
        }
        present
    }

    fn characters(
        &mut self,
        refs: &References,
        area: Option<&AreaSource>,
    ) -> Result<BTreeMap<i64, Character>, ConvertError> {
        let groups = match area {
            Some(area) => self.area_groups(area)?,
            None => BTreeMap::new(),
        };
        refs.characters
            .iter()
            .map(|character| {
                let built = self.character(character, groups.get(&character.id).copied())?;
                Ok((character.id, built))
            })
            .collect()
    }

    fn character(
        &mut self,
        refs: &CharacterRefs,
        group: Option<i64>,
    ) -> Result<Character, ConvertError> {
        let motion_set = self.motion_set(refs.id);
        let motion_set_name = motion_set.as_deref();
        let area_costumes = match group {
            Some(group) => self.area_costumes(refs, group, motion_set_name)?,
            None => Vec::new(),
        };
        let costumes = refs
            .costumes
            .iter()
            .map(|costume| self.costume(refs, costume, motion_set_name))
            .collect::<Result<Vec<_>, _>>()?;

        if motion_set.is_some() {
            let resolved: BTreeSet<&String> = costumes
                .iter()
                .flat_map(|costume| costume.motions.keys())
                .chain(
                    area_costumes
                        .iter()
                        .flat_map(|costume| costume.motions.keys()),
                )
                .collect();
            for name in refs.motions.iter().filter(|name| !resolved.contains(name)) {
                let detail = format!("Character2dId {}: {name} is not found", refs.id);
                self.warn(WarningKind::MotionNotFound, detail);
            }
        }
        let row = self
            .plan
            .character2ds
            .get(&rules::table_character(refs.id), self.cx);
        Ok(Character {
            asset_name: row.and_then(|row| row.asset_name.clone()),
            motion_set,
            costumes,
            area_costumes,
        })
    }

    fn costume(
        &mut self,
        refs: &CharacterRefs,
        costume_type: &str,
        motion_set: Option<&str>,
    ) -> Result<Costume, ConvertError> {
        let model = self.model(costume_type, refs.id);
        Ok(Costume {
            costume_type: costume_type.to_owned(),
            motions: self.motions(&refs.motions, model.as_deref(), motion_set)?,
            model,
        })
    }

    /// The `costume2ds` candidates of a character in an area conversation, in table order.
    fn area_costumes(
        &mut self,
        refs: &CharacterRefs,
        group: i64,
        motion_set: Option<&str>,
    ) -> Result<Vec<AreaCostume>, ConvertError> {
        let rows = self.plan.costume2ds.get(&refs.id, self.cx);
        let rows = rows.into_iter().flatten();
        rows.filter(|row| row.group == group)
            .map(|row| {
                let model = row.model.as_deref().and_then(|m| self.model(m, refs.id));
                let motions = match &model {
                    Some(model) => self.motions(&refs.motions, Some(model), motion_set)?,
                    None => BTreeMap::new(),
                };
                Ok(AreaCostume {
                    from: row.from.clone(),
                    to: row.to.clone(),
                    model,
                    motions,
                })
            })
            .collect()
    }

    /// The costume group of each character of an area conversation; the first detail decides.
    fn area_groups(&mut self, area: &AreaSource) -> Result<BTreeMap<i64, i64>, ConvertError> {
        let found = of_form(self.assets(&area.data_bundle), ACTION_SET)
            .find(|entry| stem(&entry.name).eq_ignore_ascii_case(&area.script_id));
        let Some(entry) = found else {
            let detail = format!(
                "the action set {} of {} is not in the version",
                area.script_id, area.data_bundle
            );
            self.warn(WarningKind::AssetMissing, detail);
            return Ok(BTreeMap::new());
        };
        let bytes = self.cx.member(&entry.name, ACTION_SET)?;
        let mut groups = BTreeMap::new();
        for detail in ActionSet::from_bytes(&bytes).map_err(malformed)?.details {
            groups
                .entry(detail.area_character_id)
                .or_insert(detail.costume2d_group_id);
        }
        Ok(groups)
    }

    fn model(&mut self, costume: &str, id: i64) -> Option<String> {
        let bundle = format!("live2d/model/{costume}");
        let Some(entry) = of_form(self.assets(&bundle), MODEL).next() else {
            let detail = format!("{bundle} (model of Character2dId {id}) is not in the version");
            self.warn(WarningKind::ModelMissing, detail);
            return None;
        };
        Some(self.refer(&entry.name))
    }

    fn motion_set(&mut self, id: i64) -> Option<String> {
        let table_character = rules::table_character(id);
        let Some(row) = self.plan.character2ds.get(&table_character, self.cx) else {
            let detail = format!("Character2dId {id} is not in character2ds");
            self.warn(WarningKind::TableMismatch, detail);
            return None;
        };
        let Some(name) = &row.asset_name else {
            let detail = format!("Character2dId {id}: character2ds row has no assetName");
            self.warn(WarningKind::MotionSetUnresolved, detail);
            return None;
        };
        let bundle = format!("live2d/motion/{name}_motion_base");
        let Some(entry) = of_form(self.assets(&bundle), MOTIONS).next() else {
            let detail = format!("{bundle} (motions of Character2dId {id}) is not in the version");
            self.warn(WarningKind::MotionSetUnresolved, detail);
            return None;
        };
        Some(self.refer(&entry.name))
    }

    /// Where each name is: first in the model, then in the motion set.
    fn motions(
        &self,
        names: &[String],
        model: Option<&str>,
        motion_set: Option<&str>,
    ) -> Result<BTreeMap<String, MotionRef>, ConvertError> {
        let sources = [model, motion_set]
            .into_iter()
            .flatten()
            .map(|asset| Ok((asset, self.plan.library.motion_index(self.cx, asset)?)))
            .collect::<Result<Vec<_>, ConvertError>>()?;
        let found = names.iter().filter_map(|name| {
            let key = name.to_lowercase();
            let (asset, role) = sources
                .iter()
                .find_map(|(asset, index)| Some((*asset, index.get(&key)?)))?;
            let motion = MotionRef {
                asset: asset.to_owned(),
                role: role.clone(),
            };
            Some((name.clone(), motion))
        });
        Ok(found.collect())
    }

    /// The textures and the prefabs of the backgrounds, and the bundles that hold them.
    fn backgrounds(&mut self, refs: &References, area: Option<&AreaSource>) -> Backgrounds {
        let area_image = area.map(|area| rules::area_background(area.area_id));
        let wanted = area_image
            .iter()
            .map(|file| {
                (
                    file.clone(),
                    rules::AREA_BACKGROUND.to_owned(),
                    file.as_str(),
                )
            })
            .chain(refs.backgrounds.iter().map(|background| {
                let bundle = format!("{}{}", rules::BACKGROUND_PREFIX, background.name);
                (background.key(), bundle, background.file.as_str())
            }));
        let mut backgrounds = Backgrounds::default();
        for (key, bundle, file) in wanted {
            if !self.require(&bundle, "background") {
                continue;
            }
            if let Some(texture) = self.texture(&key, &bundle, file) {
                backgrounds.textures.insert(key.clone(), texture);
            }
            if let Some(prefab) = self.background_prefab(&bundle) {
                backgrounds.prefabs.insert(key, prefab);
            }
            if !backgrounds.bundles.contains(&bundle) {
                backgrounds.bundles.push(bundle);
            }
        }
        backgrounds
    }

    /// The scenario prefab of the build data that `bundle` holds when it is a prefab background.
    fn background_prefab(&mut self, bundle: &str) -> Option<String> {
        let entry = of_form(self.assets(bundle), SCENARIO_PREFAB).next()?;
        Some(self.refer(&entry.name))
    }

    /// The texture `file` of `bundle`, or the only texture of the bundle.
    fn texture(&mut self, key: &str, bundle: &str, file: &str) -> Option<String> {
        let textures: Vec<&AssetEntry> = of_form(self.assets(bundle), TEXTURE).collect();
        let named = textures
            .iter()
            .find(|entry| stem(&entry.name).eq_ignore_ascii_case(file));
        let only = textures.first().filter(|_| textures.len() == 1);
        let Some(entry) = named.or(only) else {
            let detail = format!(
                "background {key}: no texture {file:?} in {bundle} ({} textures)",
                textures.len()
            );
            self.warn(WarningKind::AssetMissing, detail);
            return None;
        };
        Some(self.refer(&entry.name))
    }

    /// The videos of the movies of the scene data and of the background prefabs.
    fn movies(
        &mut self,
        refs: &References,
        background_bundles: &[String],
    ) -> Result<BTreeMap<String, Vec<String>>, ConvertError> {
        let prefab_movies = self.prefab_movies(background_bundles)?;
        let of_scene = refs
            .movies
            .iter()
            .map(|name| (name.clone(), format!("{}{name}", rules::MOVIE_PREFIX)));
        let of_prefabs = prefab_movies.into_iter().map(|bundle| {
            let name = bundle
                .strip_prefix(rules::MOVIE_PREFIX)
                .unwrap_or(&bundle)
                .to_owned();
            (name, bundle)
        });
        let mut seen = HashSet::new();
        let mut movies = BTreeMap::new();
        for (name, bundle) in of_scene.chain(of_prefabs) {
            if !seen.insert(bundle.clone()) || !self.require(&bundle, "movie") {
                continue;
            }
            let videos: Vec<String> = of_form(self.assets(&bundle), VIDEO)
                .map(|entry| self.refer(&entry.name))
                .collect();
            if videos.is_empty() {
                let detail = format!("{bundle} (movie) holds no video in the version");
                self.warn(WarningKind::AssetMissing, detail);
            } else {
                movies.insert(name, videos);
            }
        }
        Ok(movies)
    }

    /// The movies the background prefabs of `bundles` play.
    fn prefab_movies(&self, bundles: &[String]) -> Result<Vec<String>, ConvertError> {
        let mut movies = Vec::new();
        for bundle in bundles {
            for entry in of_form(self.assets(bundle), SCENARIO_BACKGROUND) {
                let bytes = self.cx.member(&entry.name, SCENARIO_BACKGROUND)?;
                movies.extend(
                    ScenarioBackground::from_bytes(&bytes)
                        .map_err(malformed)?
                        .movies,
                );
            }
        }
        Ok(movies)
    }

    /// The effects `effects` of the scene data, as names and bundles, by key: the name, or the
    /// bundle for a name used with a second bundle.
    fn effects(&mut self, effects: &[(String, String)]) -> BTreeMap<String, Effect> {
        let mut keyed: BTreeMap<&str, (&str, &str)> = BTreeMap::new();
        for (name, bundle) in effects {
            let key = match keyed.get(name.as_str()) {
                Some((_, known)) if known != bundle => bundle,
                _ => name,
            };
            keyed.insert(key, (name, bundle));
        }
        keyed
            .into_iter()
            .map(|(key, (name, bundle))| (key.to_owned(), self.effect(name, bundle)))
            .collect()
    }

    /// The effect `name` of `bundle`, with its prefab and the sounds of the bundle.
    fn effect(&mut self, name: &str, bundle: &str) -> Effect {
        let prefab = if self.require(bundle, "effect") {
            self.effect_prefab(name, bundle)
        } else {
            None
        };
        let sounds = of_form(self.assets(bundle), AUDIO)
            .map(|entry| self.refer(&entry.name))
            .collect();
        Effect {
            bundle: bundle.to_owned(),
            prefab,
            sounds,
        }
    }

    /// The scenario prefab of the effect `name` in `bundle`: the one of that name, or the only one
    /// of the bundle.
    fn effect_prefab(&mut self, name: &str, bundle: &str) -> Option<String> {
        let prefabs: Vec<&AssetEntry> = of_form(self.assets(bundle), SCENARIO_PREFAB).collect();
        let named = prefabs
            .iter()
            .find(|entry| stem(&entry.name).eq_ignore_ascii_case(name));
        let only = prefabs.first().filter(|_| prefabs.len() == 1);
        let Some(entry) = named.or(only) else {
            let detail = format!(
                "effect {name}: no prefab {name:?} in {bundle} ({} prefabs)",
                prefabs.len()
            );
            self.warn(WarningKind::AssetMissing, detail);
            return None;
        };
        Some(self.refer(&entry.name))
    }

    fn music(
        &mut self,
        refs: &References,
        area: Option<&AreaSource>,
    ) -> Result<BTreeMap<String, Sound>, ConvertError> {
        let mut planned = SearchPlan::new();
        for name in &refs.bgm {
            planned
                .entry(name)
                .or_default()
                .push(rules::scenario_bgm(name));
        }
        if let Some(area) = area {
            planned
                .entry(&area.bgm_cue)
                .or_default()
                .push(area.bgm_bundle.clone());
        }
        self.sounds(planned, "music")
    }

    fn sound_effects(
        &mut self,
        refs: &References,
    ) -> Result<BTreeMap<String, Sound>, ConvertError> {
        let event_bundles = &self.entry.row.se_bundles;
        let mut planned = SearchPlan::new();
        for effect in &refs.se {
            let placed = rules::se_bundles(&effect.cue);
            let searched = if placed.is_empty() {
                let packs = rules::FALLBACK_SE.iter().map(|pack| (*pack).to_owned());
                event_bundles.iter().cloned().chain(packs).collect()
            } else {
                placed
            };
            let candidates = planned.entry(&effect.cue).or_default();
            candidates.extend(effect.bundle.iter().cloned());
            candidates.extend(searched);
        }
        self.sounds(planned, "sound effect")
    }

    fn voices(&mut self, refs: &References) -> Result<BTreeMap<String, Sound>, ConvertError> {
        let (plan, cx) = (self.plan, self.cx);
        let own = rules::voice_bundle(&self.entry.bundle, &self.entry.row.story.scenario_id);
        let mut planned = SearchPlan::new();
        for voice in &refs.voices {
            let candidates = planned.entry(&voice.id).or_default();
            candidates.push(own.clone());
            if rules::is_part_voice(&voice.id) {
                candidates.extend(plan.part_voice_bundle(cx, voice.character));
            }
            let holders = plan.part_voices.get(&voice.id, cx);
            candidates.extend(holders.into_iter().flatten().cloned());
        }
        self.sounds(planned, "voice")
    }

    /// Searches the bundles the version holds for each cue.
    fn sounds(
        &mut self,
        planned: SearchPlan<'_>,
        what: &str,
    ) -> Result<BTreeMap<String, Sound>, ConvertError> {
        let mut found = BTreeMap::new();
        for (cue, candidates) in planned {
            let mut seen = HashSet::new();
            let bundles: Vec<String> = candidates
                .into_iter()
                .filter(|bundle| !self.assets(bundle).is_empty() && seen.insert(bundle.clone()))
                .collect();
            if bundles.is_empty() {
                let detail = format!("{what} {cue:?}: no bundle is in the version");
                self.warn(WarningKind::AssetMissing, detail);
            } else if let Some(sound) = self.sound(&bundles, cue, what)? {
                found.insert(cue.to_owned(), sound);
            }
        }
        Ok(found)
    }

    /// The first sound of `bundles` with the cue `cue`; an asset of the scene is tried first.
    fn sound(
        &mut self,
        bundles: &[String],
        cue: &str,
        what: &str,
    ) -> Result<Option<Sound>, ConvertError> {
        let scenario_id = &self.entry.row.story.scenario_id;
        for bundle in bundles {
            let mut assets: Vec<&AssetEntry> = of_form(self.assets(bundle), AUDIO).collect();
            assets.sort_by_key(|entry| !stem(&entry.name).eq_ignore_ascii_case(scenario_id));
            for entry in assets {
                let table = self.plan.library.cue_table(self.cx, &entry.name)?;
                if let Some(found) = table.cues.iter().find(|known| known.name == cue) {
                    return Ok(Some(Sound {
                        asset: self.refer(&entry.name),
                        cue: cue.to_owned(),
                        waveforms: found.waveforms.clone(),
                        arrangement: found.arrangement.clone(),
                    }));
                }
            }
        }
        self.warn(
            WarningKind::CueNotFound,
            format!("{what} {cue}: not in {bundles:?}"),
        );
        Ok(None)
    }
}

/// How an area conversation plays, naming only what the document holds.
fn area_talk(
    area: &AreaSource,
    backgrounds: &BTreeMap<String, String>,
    bgm: &BTreeMap<String, Sound>,
) -> AreaTalk {
    let image = rules::area_background(area.area_id);
    AreaTalk {
        action_set_id: area.action_set_id,
        area_id: area.area_id,
        background: backgrounds.contains_key(&image).then_some(image),
        bgm: bgm
            .contains_key(&area.bgm_cue)
            .then(|| area.bgm_cue.clone()),
    }
}

/// The backgrounds of an episode.
#[derive(Default)]
struct Backgrounds {
    /// The texture of each background key.
    textures: BTreeMap<String, String>,
    /// The scenario prefab of the build data of each prefab background, by background key.
    prefabs: BTreeMap<String, String>,
    /// The bundles that hold the backgrounds, in order of first use.
    bundles: Vec<String>,
}
