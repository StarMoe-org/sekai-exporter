//! The game data tables that the `costume-role` and `costume` forms are built from.

use std::collections::{BTreeMap, BTreeSet};

use sekai_convert::ConvertError;
use sekai_convert::tables::TableContext;
use sekai_format::costume::Slot;
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// A row of `character3ds`: the default costume of a character in a unit.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Character3dRow {
    id: i64,
    character_id: i64,
    unit: String,
    head_costume3d_id: i64,
    hair_costume3d_id: i64,
    body_costume3d_id: i64,
}

/// A row of `costume3ds`, before the rows of `costume3dGroups` fill what a compact table leaves
/// out.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Costume3dRow {
    id: i64,
    costume3d_group_id: i64,
    part_type: String,
    #[serde(default)]
    color_id: i64,
    #[serde(default)]
    color_name: Option<String>,
    #[serde(default)]
    character_id: Option<i64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    published_at: Option<i64>,
}

/// A row of `costume3dGroups`: what the rows of a compact `costume3ds` share.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GroupRow {
    group_id: i64,
    #[serde(default)]
    character_id: Option<i64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    published_at: Option<i64>,
}

/// A row of `costume3dModels`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelRow {
    costume3d_id: i64,
    unit: String,
    #[serde(default)]
    assetbundle_name: Option<String>,
    #[serde(default)]
    color_assetbundle_name: Option<String>,
    #[serde(default)]
    head_costume3d_assetbundle_type: Option<String>,
    #[serde(default)]
    thumbnail_assetbundle_name: Option<String>,
    #[serde(default)]
    part: Option<String>,
}

/// A row of `costume3dModelDefaultHairs` or `costume3dModelNotAvailablePatterns`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeadHairRow {
    head_costume3d_id: i64,
    hair_costume3d_id: i64,
    unit: String,
}

/// A row of `gameCharacters`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CharacterRow {
    id: i64,
    #[serde(default)]
    figure: String,
    #[serde(default)]
    breast_size: String,
}

/// A row of `gameCharacterUnits`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CharacterUnitRow {
    id: i64,
    game_character_id: i64,
    unit: String,
    #[serde(default)]
    skin_color_code: String,
    #[serde(default)]
    skin_shadow_color_code1: String,
    #[serde(default)]
    skin_shadow_color_code2: String,
}

/// A row of `character3ds`: the default costume of a character in a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DefaultCostume {
    pub(super) character_id: i64,
    pub(super) unit: String,
    pub(super) body: i64,
    pub(super) head: i64,
    pub(super) hair: i64,
}

/// A row of `costume3ds`, with what its `costume3dGroups` row fills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct CostumeRow {
    pub(super) id: i64,
    pub(super) group_id: i64,
    pub(super) part_type: String,
    pub(super) color_id: i64,
    pub(super) color_name: Option<String>,
    pub(super) character: i64,
    pub(super) name: Option<String>,
    pub(super) published_at: Option<i64>,
}

impl CostumeRow {
    /// The costume the row belongs to. The rows of the default and unit costumes, whose group is
    /// below [`COSTUME_MINIMUM_GROUP`], are all of costume 0.
    pub(super) fn costume_number(&self) -> i64 {
        self.group_id.max(COSTUME_MINIMUM_GROUP - 1) / COSTUME_MINIMUM_GROUP
    }

    /// The slot the row fills; `None` for a part type that is not a slot.
    pub(super) fn slot(&self) -> Option<Slot> {
        match self.part_type.as_str() {
            "body" => Some(Slot::Body),
            "hair" => Some(Slot::Hair),
            "head" => Some(Slot::Head),
            _ => None,
        }
    }
}

/// The lowest group ID of a costume.
const COSTUME_MINIMUM_GROUP: i64 = 1000;

/// A row of `costume3dModels`, with the row of `costume3ds` it models, of a type that is a slot.
/// A part can be modelled differently in each unit, so each row is of one unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PartRow {
    pub(super) costume: CostumeRow,
    pub(super) slot: Slot,
    pub(super) unit: String,
    pub(super) assetbundle_name: Option<String>,
    pub(super) color_assetbundle_name: Option<String>,
    pub(super) head_type: Option<String>,
    pub(super) thumbnail: Option<String>,
    pub(super) part: Option<String>,
}

impl PartRow {
    /// The `costume3dId`.
    pub(super) fn id(&self) -> i64 {
        self.costume.id
    }

    /// The colour.
    pub(super) fn color_id(&self) -> i64 {
        self.costume.color_id
    }

    /// The slot the part fills.
    pub(super) fn slot(&self) -> Slot {
        self.slot
    }

    /// The costume the part belongs to.
    pub(super) fn costume_number(&self) -> i64 {
        self.costume.costume_number()
    }

    /// Whether the part is a head with its hair, which shows alone and replaces the hair.
    pub(super) fn is_complete_head(&self) -> bool {
        self.slot == Slot::Head && self.head_type.as_deref() == Some(COMPLETE_HEAD)
    }
}

/// The head type of a head with its hair.
const COMPLETE_HEAD: &str = "head_and_hair";

/// What `gameCharacters` says about a character.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Figure {
    pub(super) figure: String,
    pub(super) breast_size: String,
}

/// A row of `gameCharacterUnits`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UnitRow {
    pub(super) id: i64,
    pub(super) character_id: i64,
    pub(super) unit: String,
    pub(super) skin: Skin,
}

/// The skin colour codes of a row of `gameCharacterUnits`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Skin {
    pub(super) default: String,
    pub(super) shadow1: String,
    pub(super) shadow2: String,
}

/// A pair of a head and the hair worn with it.
pub(super) type HeadHair = [i64; 2];

/// The tables, as the roles and costumes read them.
#[derive(Debug, Default)]
pub(super) struct Tables {
    pub(super) defaults: Vec<DefaultCostume>,
    pub(super) parts: Vec<PartRow>,
    pub(super) default_hairs: Vec<(String, HeadHair)>,
    pub(super) not_available: Vec<(String, HeadHair)>,
    pub(super) characters: BTreeMap<i64, Figure>,
    pub(super) units: Vec<UnitRow>,
}

impl Tables {
    /// The parts the character can wear in the unit, as the unit models them, ordered by
    /// `costume3dId`.
    pub(super) fn parts_of(&self, character: i64, unit: &str) -> Vec<PartRow> {
        self.parts
            .iter()
            .filter(|part| part.costume.character == character && part.unit == unit)
            .cloned()
            .collect()
    }

    /// The heads and hairs of the unit that are not worn together, ordered.
    pub(super) fn not_available_of(&self, unit: &str) -> Vec<HeadHair> {
        let pairs = self
            .not_available
            .iter()
            .filter(|(row_unit, _)| row_unit == unit)
            .map(|&(_, pair)| pair);
        BTreeSet::from_iter(pairs).into_iter().collect()
    }

    /// The hair worn with each head of `parts`, ordered by head.
    pub(super) fn default_hairs_of(&self, unit: &str, parts: &[PartRow]) -> BTreeMap<i64, i64> {
        let heads: BTreeSet<i64> = parts
            .iter()
            .filter(|part| part.slot() == Slot::Head)
            .map(PartRow::id)
            .collect();
        self.default_hairs
            .iter()
            .filter(|(row_unit, _)| row_unit == unit)
            .map(|&(_, [head, hair])| (head, hair))
            .filter(|(head, _)| heads.contains(head))
            .collect()
    }

    /// The row of the character in the unit, else the character's first row.
    pub(super) fn skin_row(&self, character: i64, unit: &str) -> Option<&UnitRow> {
        self.units_of(character)
            .find(|row| row.unit == unit)
            .or_else(|| self.units_of(character).min_by_key(|row| row.id))
    }

    /// The ID of the row of the character in the unit, then of the character's first row.
    pub(super) fn unit_ids(&self, character: i64, unit: &str) -> Vec<i64> {
        let own = self
            .units_of(character)
            .find(|row| row.unit == unit)
            .map(|row| row.id);
        let first = self.units_of(character).map(|row| row.id).min();
        let ids: BTreeSet<i64> = own.into_iter().chain(first).collect();
        ids.into_iter().collect()
    }

    fn units_of(&self, character: i64) -> impl Iterator<Item = &UnitRow> {
        self.units
            .iter()
            .filter(move |row| row.character_id == character)
    }
}

fn table<T: DeserializeOwned>(cx: &dyn TableContext, name: &str) -> Result<Vec<T>, ConvertError> {
    serde_json::from_slice(&cx.table(name)?)
        .map_err(|e| ConvertError::new("malformed", format!("table {name}: {e}")))
}

/// Reads the tables. A `null` field is read as an absent one.
pub(super) fn load(cx: &dyn TableContext) -> Result<Tables, ConvertError> {
    let costumes = costumes(cx)?;
    Ok(Tables {
        defaults: defaults(table(cx, "character3ds")?),
        parts: parts(table(cx, "costume3dModels")?, &costumes),
        default_hairs: head_hairs(table(cx, "costume3dModelDefaultHairs")?),
        not_available: head_hairs(table(cx, "costume3dModelNotAvailablePatterns")?),
        characters: characters(table(cx, "gameCharacters")?),
        units: unit_rows(table(cx, "gameCharacterUnits")?),
    })
}

/// The role of each pair of a character and a unit in `character3ds`, with the default costume
/// of the row of the lowest `id`.
fn defaults(mut rows: Vec<Character3dRow>) -> Vec<DefaultCostume> {
    rows.sort_by(|a, b| (a.character_id, &a.unit, a.id).cmp(&(b.character_id, &b.unit, b.id)));
    rows.dedup_by(|later, first| {
        later.character_id == first.character_id && later.unit == first.unit
    });
    rows.into_iter()
        .map(|row| DefaultCostume {
            character_id: row.character_id,
            unit: row.unit,
            body: row.body_costume3d_id,
            head: row.head_costume3d_id,
            hair: row.hair_costume3d_id,
        })
        .collect()
}

/// The rows of `costume3ds`, by `id`. When some row has no `characterId`, each row takes the
/// `characterId`, `name` and `publishedAt` it lacks from the `costume3dGroups` row of its group;
/// a row that still has no `characterId` is left out.
fn costumes(cx: &dyn TableContext) -> Result<BTreeMap<i64, CostumeRow>, ConvertError> {
    let rows: Vec<Costume3dRow> = table(cx, "costume3ds")?;
    let compact = rows.iter().any(|row| row.character_id.is_none());
    let groups = match compact {
        true => groups(cx)?,
        false => BTreeMap::new(),
    };
    let filled = rows.into_iter().filter_map(|row| {
        let group = groups.get(&row.costume3d_group_id);
        fill(row, group)
    });
    Ok(filled.map(|row| (row.id, row)).collect())
}

fn groups(cx: &dyn TableContext) -> Result<BTreeMap<i64, GroupRow>, ConvertError> {
    let groups: Vec<GroupRow> = table(cx, "costume3dGroups")?;
    Ok(groups
        .into_iter()
        .map(|group| (group.group_id, group))
        .collect())
}

/// The row with the character, name and release its group gives it, unless it has no character.
fn fill(row: Costume3dRow, group: Option<&GroupRow>) -> Option<CostumeRow> {
    let Costume3dRow {
        id,
        costume3d_group_id: group_id,
        part_type,
        color_id,
        color_name,
        character_id,
        name,
        published_at,
    } = row;
    Some(CostumeRow {
        id,
        group_id,
        part_type,
        color_id,
        color_name,
        character: character_id.or(group.and_then(|group| group.character_id))?,
        name: name.or(group.and_then(|group| group.name.clone())),
        published_at: published_at.or(group.and_then(|group| group.published_at)),
    })
}

/// The rows of `costume3dModels` whose `costume3ds` row exists and is of a type that is a slot,
/// ordered by `costume3dId`.
fn parts(models: Vec<ModelRow>, costumes: &BTreeMap<i64, CostumeRow>) -> Vec<PartRow> {
    let mut parts: Vec<PartRow> = models
        .into_iter()
        .filter_map(|model| {
            let costume = costumes.get(&model.costume3d_id)?;
            Some(PartRow {
                slot: costume.slot()?,
                costume: costume.clone(),
                unit: model.unit,
                assetbundle_name: model.assetbundle_name,
                color_assetbundle_name: model.color_assetbundle_name,
                head_type: model.head_costume3d_assetbundle_type,
                thumbnail: model.thumbnail_assetbundle_name,
                part: model.part,
            })
        })
        .collect();
    parts.sort_by_key(PartRow::id);
    parts
}

fn head_hairs(rows: Vec<HeadHairRow>) -> Vec<(String, HeadHair)> {
    rows.into_iter()
        .map(|row| {
            let pair = [row.head_costume3d_id, row.hair_costume3d_id];
            (row.unit, pair)
        })
        .collect()
}

fn characters(rows: Vec<CharacterRow>) -> BTreeMap<i64, Figure> {
    rows.into_iter()
        .map(|row| {
            let figure = Figure {
                figure: row.figure,
                breast_size: row.breast_size,
            };
            (row.id, figure)
        })
        .collect()
}

fn unit_rows(rows: Vec<CharacterUnitRow>) -> Vec<UnitRow> {
    rows.into_iter()
        .map(|row| UnitRow {
            id: row.id,
            character_id: row.game_character_id,
            unit: row.unit,
            skin: Skin {
                default: row.skin_color_code,
                shadow1: row.skin_shadow_color_code1,
                shadow2: row.skin_shadow_color_code2,
            },
        })
        .collect()
}
