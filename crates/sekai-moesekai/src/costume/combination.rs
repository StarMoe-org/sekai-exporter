//! The roles of the 3D costume screen and the looks each costume gives them.
//!
//! `CostumeRoles` produces the `costume-role` form and `Costumes` the `costume` form, both
//! specified in `docs/spec/costume.md`. They are built from the game data tables and from the
//! `costume-model`, `costume-motion` and `texture` assets of the version.

use std::collections::{BTreeMap, BTreeSet};

use sekai_convert::tables::{TableAsset, TableContext, TableConverter, TablePlan};
use sekai_convert::{Asset, Content, ConvertError, Member};
use sekai_format::costume::{
    COSTUME_FORMAT, Colors, CostumeDocument, Look, LookModels, Part, PartModel, ROLE_FORMAT, Role,
    RoleDocument, Skin, Slot, VERSION,
};

use super::sources::{Sources, Version};
use super::tables::{self, DefaultCostume, Figure, PartRow};

/// The head types whose face has a variant of its own.
const FACE_VARIANTS: &[&str] = &["head_all", "head_front", "head_back"];
/// The default face model and the model next to it.
const FIRST_FACE: &str = "0000";
const SECOND_FACE: &str = "0001";
/// The breast sizes the body model of the figure does not add.
const NO_BREAST_SIZE: &[&str] = &["none", ""];

/// The roles of the 3D costume screen.
#[derive(Debug, Clone, Copy, Default)]
pub struct CostumeRoles;

impl TableConverter for CostumeRoles {
    fn name(&self) -> &'static str {
        ROLE_FORMAT
    }

    fn version(&self) -> u32 {
        VERSION
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        let sources = Sources::read(cx)?;
        let roles = RoleSource::read_all(&sources, cx)?;
        let names = roles.iter().map(RoleSource::asset_name).collect();
        Ok(Box::new(RolePlan { names, roles }))
    }
}

/// The costumes of the 3D costume screen.
#[derive(Debug, Clone, Copy, Default)]
pub struct Costumes;

impl TableConverter for Costumes {
    fn name(&self) -> &'static str {
        COSTUME_FORMAT
    }

    fn version(&self) -> u32 {
        VERSION
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        let sources = Sources::read(cx)?;
        let roles = RoleSource::read_all(&sources, cx)?;
        let numbers = CostumePlan::numbers(&roles);
        let names = numbers
            .iter()
            .map(|number| costume_asset(*number))
            .collect();
        Ok(Box::new(CostumePlan {
            names,
            numbers,
            roles,
            version: sources.version,
        }))
    }
}

/// The `costume-role` assets, one per role.
struct RolePlan {
    names: Vec<String>,
    roles: Vec<RoleSource>,
}

impl TablePlan for RolePlan {
    fn names(&self) -> &[String] {
        &self.names
    }

    fn convert(&self, index: usize) -> Result<TableAsset, ConvertError> {
        let (document, referrals) = self.roles[index].document();
        Ok(TableAsset {
            asset: member(ROLE_FORMAT, document.to_bytes().map_err(malformed)?),
            references: referrals.into(),
        })
    }
}

/// The `costume` assets, one per costume number, with what their looks need of the version.
struct CostumePlan {
    names: Vec<String>,
    numbers: Vec<i64>,
    roles: Vec<RoleSource>,
    version: Version,
}

impl CostumePlan {
    /// The number of each costume that a role has parts of, ordered.
    fn numbers(roles: &[RoleSource]) -> Vec<i64> {
        roles
            .iter()
            .flat_map(RoleSource::costume_numbers)
            .collect::<BTreeSet<i64>>()
            .into_iter()
            .collect()
    }
}

impl TablePlan for CostumePlan {
    fn names(&self) -> &[String] {
        &self.names
    }

    fn convert(&self, index: usize) -> Result<TableAsset, ConvertError> {
        let number = self.numbers.get(index).copied().ok_or_else(|| {
            malformed_table("a costume plan names a costume that the roles do not have")
        })?;
        let costume = CostumeSource {
            number,
            roles: &self.roles,
            version: &self.version,
        };
        let (document, referrals) = costume.document();
        Ok(TableAsset {
            asset: member(COSTUME_FORMAT, document.to_bytes().map_err(malformed)?),
            references: referrals.into(),
        })
    }
}

fn role_asset(character: i64, unit: &str) -> String {
    format!("costume/role/{character}-{unit}")
}

fn costume_asset(number: i64) -> String {
    format!("costume/{number}")
}

/// A malformed table.
fn malformed_table(message: &'static str) -> ConvertError {
    ConvertError::new("malformed", message)
}

/// The names of the assets a table asset refers to.
#[derive(Debug, Default)]
struct Referrals(Vec<String>);

impl Referrals {
    /// Adds the model and its colour textures, if any.
    fn model(&mut self, model: Option<&PartModel>) {
        self.0.extend(model.into_iter().flat_map(model_assets));
    }

    /// Adds the names the iterator gives.
    fn add(&mut self, names: impl IntoIterator<Item = String>) {
        self.0.extend(names);
    }
}

impl From<Referrals> for Vec<String> {
    fn from(referrals: Referrals) -> Self {
        referrals.0
    }
}

/// The names of the assets that show a model and its colour textures.
fn model_assets(model: &PartModel) -> Vec<String> {
    let textures = model.colors.iter().flat_map(textures_of);
    std::iter::once(model.asset.clone())
        .chain(textures)
        .collect()
}

fn textures_of(colors: &Colors) -> Vec<String> {
    [&colors.color, &colors.shadow, &colors.value]
        .into_iter()
        .flatten()
        .cloned()
        .collect()
}

/// The names of the assets that show a look.
fn look_assets(look: &Look) -> Vec<String> {
    let mut referrals = Referrals::default();
    referrals.model(look.models.body.as_ref());
    referrals.add(look.models.face.clone());
    referrals.model(look.models.accessory.as_ref());
    referrals.into()
}

/// One costume and the roles that have parts of it.
struct CostumeSource<'r> {
    number: i64,
    roles: &'r [RoleSource],
    version: &'r Version,
}

impl<'r> CostumeSource<'r> {
    /// The roles that have parts of the costume, in the order of their roles.
    fn roles(&self) -> impl Iterator<Item = &'r RoleSource> {
        self.roles
            .iter()
            .filter(|role| !role.parts_of(self.number).is_empty())
    }

    /// The costume as a document, and the names of the assets it refers to.
    fn document(&self) -> (CostumeDocument, Referrals) {
        let looks: Vec<Look> = self
            .roles()
            .flat_map(|role| role.looks(self.number, self.version))
            .collect();
        let mut referrals = Referrals::default();
        referrals.add(self.roles().map(RoleSource::asset_name));
        referrals.add(looks.iter().flat_map(look_assets));
        let document = CostumeDocument {
            format: COSTUME_FORMAT.to_owned(),
            version: VERSION,
            costume_number: self.number,
            name: self.roles().find_map(|role| role.name(self.number)),
            variants: looks,
        };
        (document, referrals)
    }
}

/// A role, with what the tables and the assets of the version say about it.
struct RoleSource {
    character_id: i64,
    unit: String,
    default: DefaultCostume,
    figure: Figure,
    skin: Option<tables::Skin>,
    parts: Vec<PartSource>,
    not_available: Vec<[i64; 2]>,
    default_hairs: BTreeMap<i64, i64>,
    motions: BTreeMap<String, String>,
}

impl RoleSource {
    /// The roles of `character3ds`, in the order of their pairs of a character and a unit.
    fn read_all(sources: &Sources, cx: &dyn TableContext) -> Result<Vec<Self>, ConvertError> {
        sources
            .tables
            .defaults
            .iter()
            .map(|default| Self::read(default, sources, cx))
            .collect()
    }

    fn read(
        default: &DefaultCostume,
        sources: &Sources,
        cx: &dyn TableContext,
    ) -> Result<Self, ConvertError> {
        let tables = &sources.tables;
        let rows = tables.parts_of(default.character_id, &default.unit);
        let figure = tables.characters.get(&default.character_id).cloned();
        let parts: Vec<PartSource> = rows
            .iter()
            .map(|row| PartSource::read(row, figure.as_ref(), &sources.version))
            .collect();
        let figure = figure.unwrap_or_default();
        let unit_ids = tables.unit_ids(default.character_id, &default.unit);
        Ok(Self {
            character_id: default.character_id,
            unit: default.unit.clone(),
            skin: tables
                .skin_row(default.character_id, &default.unit)
                .map(|row| row.skin.clone()),
            default_hairs: tables.default_hairs_of(&default.unit, &rows),
            not_available: tables.not_available_of(&default.unit),
            motions: sources.motions(cx, &unit_ids, &figure)?,
            figure,
            default: default.clone(),
            parts,
        })
    }

    fn asset_name(&self) -> String {
        role_asset(self.character_id, &self.unit)
    }

    /// The role as a document, and the names of the assets it refers to.
    fn document(&self) -> (RoleDocument, Referrals) {
        let document = RoleDocument {
            format: ROLE_FORMAT.to_owned(),
            version: VERSION,
            role: Role {
                character_id: self.character_id,
                unit: self.unit.clone(),
                figure: self.figure.figure.clone(),
                breast_size: self.figure.breast_size.clone(),
                skin: self.skin(),
                default_body: self.default.body,
                default_head: self.default.head,
                default_hair: self.default.hair,
                motions: (!self.motions.is_empty()).then(|| self.motions.clone()),
            },
            parts: self.parts.iter().map(PartSource::part).collect(),
            not_available: self.not_available.clone(),
            default_hairs: self.default_hairs.clone(),
        };
        let mut referrals = Referrals::default();
        referrals.add(
            self.parts
                .iter()
                .flat_map(PartSource::assets)
                .chain(self.motions.values().cloned()),
        );
        (document, referrals)
    }

    fn skin(&self) -> Skin {
        let codes = self.skin.clone().unwrap_or_default();
        Skin {
            default: codes.default,
            shadow1: codes.shadow1,
            shadow2: codes.shadow2,
        }
    }

    /// The number of each costume the role has parts of; default and unit costumes have none.
    fn costume_numbers(&self) -> BTreeSet<i64> {
        self.parts.iter().filter_map(PartSource::number).collect()
    }

    /// The parts the role can wear of the costume `number`, ordered by `costume3dId`.
    fn parts_of(&self, number: i64) -> Vec<&PartSource> {
        self.parts
            .iter()
            .filter(|part| part.number() == Some(number))
            .collect()
    }

    /// The part the role can wear with the `costume3dId` `id`.
    fn part(&self, id: i64) -> Option<&PartSource> {
        self.parts.iter().find(|part| part.id() == id)
    }

    /// The name of the costume `number`: the name of its first body part, else of its first
    /// part.
    fn name(&self, number: i64) -> Option<String> {
        let parts = self.parts_of(number);
        let first = parts
            .iter()
            .find(|part| part.slot() == Slot::Body)
            .or(parts.first())?;
        first.row.costume.name.clone()
    }

    /// The role's looks of the costume `number`, in each of its colours, in the order of the
    /// character, the unit and the colour.
    fn looks(&self, number: i64, version: &Version) -> Vec<Look> {
        let costume = self.parts_of(number);
        colors_of(&costume)
            .into_iter()
            .map(|color| self.look(color, &costume, version))
            .collect()
    }

    /// The look of the costume `costume` in `color`.
    fn look(&self, color: i64, costume: &[&PartSource], version: &Version) -> Look {
        let body = self.body(costume, color);
        let head = self.head(costume, color);
        let hair = self.hair(costume, color, head);
        let face = Face::of(head, hair, version);
        // The look shows the body, the accessory and the face; the face stands for the hair, or
        // for a head with its hair.
        let accessory = (!head.is_complete_head()).then_some(head);
        let mut missing: Vec<String> = [Some(body), accessory]
            .into_iter()
            .flatten()
            .flat_map(SlotPart::missing)
            .chain(face.missing)
            .collect();
        if self.not_available.contains(&[head.id(), hair.id()]) {
            missing.push(format!(
                "head {} and hair {} are not worn together",
                head.id(),
                hair.id()
            ));
        }
        Look {
            role: self.asset_name(),
            character_id: self.character_id,
            unit: self.unit.clone(),
            color_id: color,
            body: body.id(),
            head: head.id(),
            hair: hair.id(),
            alternative_heads: alternative_heads(color, head, costume),
            models: LookModels {
                body: body.model(),
                face: face.asset,
                accessory: accessory_of(head),
            },
            missing,
        }
    }

    /// The body the look wears: the costume's in `color`, else in colour 1, else the one of the
    /// lowest `costume3dId`; else the role's default body, which the look writes whether or not
    /// the role has it.
    fn body<'r>(&'r self, costume: &[&'r PartSource], color: i64) -> SlotPart<'r> {
        let worn = pick(&of_slot(costume, Slot::Body), color);
        SlotPart::of(worn, self.default.body, self)
    }

    /// The head the look wears: among the accessories, or among the heads with their hair when
    /// the costume has no accessory, in `color`, else in colour 1, else the one of the lowest
    /// `costume3dId`; else the role's default head.
    fn head<'r>(&'r self, costume: &[&'r PartSource], color: i64) -> SlotPart<'r> {
        let worn = head_of(&of_slot(costume, Slot::Head), color);
        SlotPart::of(worn, self.default.head, self)
    }

    /// The hair the look wears: the costume's in `color`, else in colour 1, else the one of the
    /// lowest `costume3dId`; else the default hair of the head when it is a part of the role;
    /// else the role's default hair.
    fn hair<'r>(
        &'r self,
        costume: &[&'r PartSource],
        color: i64,
        head: SlotPart<'_>,
    ) -> SlotPart<'r> {
        let of_costume = pick(&of_slot(costume, Slot::Hair), color);
        let of_head = self
            .default_hairs
            .get(&head.id())
            .and_then(|id| self.part(*id));
        SlotPart::of(of_costume.or(of_head), self.default.hair, self)
    }
}

/// The part a slot of a look wears, which the look writes by its `costume3dId`.
#[derive(Debug, Clone, Copy)]
enum SlotPart<'r> {
    /// A part of the role, from its parts.
    Role(&'r PartSource),
    /// A part of the role's default costume, which is not among the role's parts.
    Unheld(i64),
}

impl<'r> SlotPart<'r> {
    /// The part `worn`, else the role's part with the `costume3dId` `default`, else `Unheld`.
    fn of(worn: Option<&'r PartSource>, default: i64, role: &'r RoleSource) -> Self {
        match worn.or_else(|| role.part(default)) {
            Some(part) => Self::Role(part),
            None => Self::Unheld(default),
        }
    }

    fn id(self) -> i64 {
        match self {
            Self::Role(part) => part.id(),
            Self::Unheld(id) => id,
        }
    }

    /// The part of the role, when the role has it.
    fn part(self) -> Option<&'r PartSource> {
        match self {
            Self::Role(part) => Some(part),
            Self::Unheld(_) => None,
        }
    }

    /// What could not be resolved of the part.
    fn missing(self) -> Vec<String> {
        match self {
            Self::Role(part) => part.missing(),
            Self::Unheld(id) => vec![unheld(id)],
        }
    }

    fn model(self) -> Option<PartModel> {
        self.part().and_then(|part| part.model.clone())
    }

    fn is_complete_head(self) -> bool {
        self.part().is_some_and(PartSource::is_complete_head)
    }
}

/// The face of a look, and what is missing of it.
struct Face {
    asset: Option<String>,
    missing: Vec<String>,
}

impl Face {
    /// The face a look wears: `face/<model><suffix>`, where the model is the `assetbundleName` of
    /// the hair, or of the head of the type `head_and_hair`, and the suffix is `_<headType>` of
    /// the head worn when that type has a variant and the model contains `0000` or `0001`. A
    /// face without its suffix is shown instead, which the look records.
    fn of(head: SlotPart<'_>, hair: SlotPart<'_>, version: &Version) -> Self {
        let source = match head.is_complete_head() {
            true => head,
            false => hair,
        };
        let id = source.id();
        let model = match source.part() {
            Some(part) => match &part.row.assetbundle_name {
                Some(model) => model,
                None => return Self::absent(format!("the hair {id} has no model")),
            },
            None => return Self::absent(unheld(id)),
        };
        let head_type = head.part().and_then(|head| head.row.head_type.as_deref());
        find_face(version, model, head_type)
    }

    fn absent(message: String) -> Self {
        Self {
            asset: None,
            missing: vec![message],
        }
    }
}

/// What is missing of a part of the role's default costume that is not among the role's parts.
fn unheld(id: i64) -> String {
    format!("the part {id} is not among the parts of the role")
}

/// The face of the model `model` for the head type `head_type`, and what could not be resolved.
/// A face with a suffix that the version lacks is shown without it; a face that the version
/// lacks under any name is missing.
fn find_face(version: &Version, model: &str, head_type: Option<&str>) -> Face {
    let suffix = head_suffix(model, head_type);
    let of = |model: &str| match &suffix {
        Some(suffix) => format!("face/{model}{suffix}"),
        None => format!("face/{model}"),
    };
    let next = renamed(model);
    let found = [of(model), of(&next)]
        .into_iter()
        .find_map(|bundle| version.model(&bundle));
    if let Some(asset) = found {
        return Face {
            asset: Some(asset),
            missing: Vec::new(),
        };
    }
    let plain = [format!("face/{model}"), format!("face/{next}")];
    let found = plain.into_iter().find_map(|bundle| version.model(&bundle));
    match (suffix, found) {
        (Some(suffix), Some(asset)) => Face {
            asset: Some(asset),
            missing: vec![format!(
                "the face {model} has no {suffix} variant and is shown without it"
            )],
        },
        _ => Face::absent(format!("the face {model} is not in the version")),
    }
}

/// `_<headType>` when the head is of a type whose face has a variant and `model` is a default
/// face model; nothing otherwise.
fn head_suffix(model: &str, head_type: Option<&str>) -> Option<String> {
    let head_type = head_type.filter(|head_type| FACE_VARIANTS.contains(head_type))?;
    let default = model.contains(FIRST_FACE) || model.contains(SECOND_FACE);
    default.then(|| format!("_{head_type}"))
}

/// The model name with its first `0000` replaced by `0001`.
fn renamed(model: &str) -> String {
    model.replacen(FIRST_FACE, SECOND_FACE, 1)
}

/// The accessory of a look, absent for a head with its hair.
fn accessory_of(head: SlotPart<'_>) -> Option<PartModel> {
    head.model().filter(|_| !head.is_complete_head())
}

/// The costume's heads other than `head` in `color` or in colour 1, ordered.
fn alternative_heads(color: i64, head: SlotPart<'_>, costume: &[&PartSource]) -> Vec<i64> {
    of_slot(costume, Slot::Head)
        .into_iter()
        .filter(|part| part.id() != head.id())
        .filter(|part| part.color_id() == color || part.color_id() == Part::DEFAULT_COLOR)
        .map(PartSource::id)
        .collect()
}

fn of_slot<'p>(parts: &[&'p PartSource], slot: Slot) -> Vec<&'p PartSource> {
    parts
        .iter()
        .copied()
        .filter(|part| part.slot() == slot)
        .collect()
}

/// The colours of the body parts of `costume`, or of all of them when there is no body among
/// them.
fn colors_of(costume: &[&PartSource]) -> BTreeSet<i64> {
    let bodies: BTreeSet<i64> = of_slot(costume, Slot::Body)
        .iter()
        .map(|part| part.color_id())
        .collect();
    if bodies.is_empty() {
        costume.iter().map(|part| part.color_id()).collect()
    } else {
        bodies
    }
}

/// The first part of `parts` in `color`, else in colour 1, else the one of the lowest
/// `costume3dId`.
fn pick<'p>(parts: &[&'p PartSource], color: i64) -> Option<&'p PartSource> {
    let in_color = |color: i64| parts.iter().copied().find(|part| part.color_id() == color);
    in_color(color)
        .or_else(|| in_color(Part::DEFAULT_COLOR))
        .or_else(|| parts.first().copied())
}

/// The first head of `heads` in `color`, else in colour 1, else the one of the lowest
/// `costume3dId`: among the accessories, or among the heads with their hair when there is no
/// accessory.
fn head_of<'p>(heads: &[&'p PartSource], color: i64) -> Option<&'p PartSource> {
    let accessories: Vec<&PartSource> = heads
        .iter()
        .copied()
        .filter(|head| !head.is_complete_head())
        .collect();
    match accessories.is_empty() {
        true => pick(heads, color),
        false => pick(&accessories, color),
    }
}

/// A part the role can wear, with what shows it.
#[derive(Debug, Clone)]
struct PartSource {
    row: PartRow,
    model: Option<PartModel>,
    missing: Vec<String>,
}

impl PartSource {
    fn read(row: &PartRow, figure: Option<&Figure>, version: &Version) -> Self {
        let (model, missing) = match row.slot() {
            Slot::Body => body_model(row, figure, version),
            Slot::Hair => face_model(row, None, version),
            Slot::Head if row.is_complete_head() => face_model(row, Some(row), version),
            Slot::Head => accessory_model(row, version),
        };
        Self {
            row: row.clone(),
            model,
            missing,
        }
    }

    fn id(&self) -> i64 {
        self.row.id()
    }

    fn color_id(&self) -> i64 {
        self.row.color_id()
    }

    fn slot(&self) -> Slot {
        self.row.slot()
    }

    /// The number of the costume of the part, absent for default and unit costumes.
    fn number(&self) -> Option<i64> {
        Some(self.row.costume_number()).filter(|number| *number != 0)
    }

    fn is_complete_head(&self) -> bool {
        self.row.is_complete_head()
    }

    /// What could not be resolved of the part.
    fn missing(&self) -> Vec<String> {
        self.missing.clone()
    }

    /// The names of the assets that show the part.
    fn assets(&self) -> Vec<String> {
        self.model.iter().flat_map(model_assets).collect()
    }

    fn part(&self) -> Part {
        let row = &self.row;
        Part {
            costume3d_id: row.id(),
            slot: row.slot(),
            costume_number: self.number(),
            color_id: row.color_id(),
            color_name: row.costume.color_name.clone(),
            name: row.costume.name.clone(),
            head_type: row.head_type.clone(),
            published_at: row.costume.published_at,
            thumbnail: row.thumbnail.clone(),
            model: self.model.clone(),
            missing: self.missing.clone(),
        }
    }
}

/// The body of the part and, when its colour has textures of its own, their names.
fn body_model(
    row: &PartRow,
    figure: Option<&Figure>,
    version: &Version,
) -> (Option<PartModel>, Vec<String>) {
    let figure = figure_name(figure);
    let name = match &row.assetbundle_name {
        Some(name) => name.as_str(),
        None => return (None, Vec::new()),
    };
    let mut missing = Vec::new();
    let bundle = format!("body/{name}/{figure}");
    let asset = version.model(&bundle);
    if asset.is_none() {
        missing.push(format!("the body {bundle} is not in the version"));
    }
    let colors = colour_of(
        version,
        row,
        &format!("color_variation/body/{name}/{figure}"),
        &mut missing,
    );
    let model = asset.map(|asset| PartModel {
        asset,
        colors,
        attach: None,
    });
    (model, missing)
}

/// The unsuffixed face of a hair, or of a head with its hair.
fn face_model(
    row: &PartRow,
    head: Option<&PartRow>,
    version: &Version,
) -> (Option<PartModel>, Vec<String>) {
    let head_type = head.and_then(|head| head.head_type.as_deref());
    let name = match &row.assetbundle_name {
        Some(name) => name.as_str(),
        None => return (None, Vec::new()),
    };
    let face = find_face(version, name, head_type);
    let model = face.asset.map(|asset| PartModel {
        asset,
        colors: None,
        attach: None,
    });
    (model, face.missing)
}

/// The accessory of a head and, when its colour has textures of its own, their names; `attach`
/// is the head's part, else the last segment of its name.
fn accessory_model(row: &PartRow, version: &Version) -> (Option<PartModel>, Vec<String>) {
    let name = match &row.assetbundle_name {
        Some(name) => name.as_str(),
        None => return (None, Vec::new()),
    };
    let mut missing = Vec::new();
    let bundle = format!("head_optional/{name}");
    let asset = version.model(&bundle);
    if asset.is_none() {
        missing.push(format!("the accessory {bundle} is not in the version"));
    }
    let colors = colour_of(
        version,
        row,
        &format!("color_variation/head_optional/{name}"),
        &mut missing,
    );
    let attach = row
        .part
        .clone()
        .or_else(|| name.rsplit('/').next().map(str::to_owned));
    let model = asset.map(|asset| PartModel {
        asset,
        colors,
        attach,
    });
    (model, missing)
}

/// The colour textures of `prefix/<colorAssetbundleName>`, when the part names a colour.
fn colour_of(
    version: &Version,
    row: &PartRow,
    prefix: &str,
    missing: &mut Vec<String>,
) -> Option<Colors> {
    let color = row.color_assetbundle_name.as_deref()?;
    let bundle = format!("{prefix}/{color}");
    let colors = version.colors(&bundle);
    if colors.is_none() {
        missing.push(format!("the colour {bundle} is not in the version"));
    }
    colors
}

/// `figure`, or `figure_breastSize` unless the breast size is `none`.
fn figure_name(figure: Option<&Figure>) -> String {
    let figure = figure.cloned().unwrap_or_default();
    match NO_BREAST_SIZE.contains(&figure.breast_size.as_str()) {
        true => figure.figure,
        false => format!("{}_{}", figure.figure, figure.breast_size),
    }
}

/// The member of a table asset: a JSON document.
fn member(role: &str, bytes: Vec<u8>) -> Asset {
    Asset {
        members: vec![Member {
            role: role.to_owned(),
            content: Content::File {
                extension: "json".to_owned(),
                bytes,
            },
        }],
    }
}

fn malformed(e: serde_json::Error) -> ConvertError {
    ConvertError::new("malformed", format!("the document cannot be written: {e}"))
}
