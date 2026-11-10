//! The `costume-role` and `costume` forms on tables and assets built in memory.

#![cfg(feature = "costume")]

use std::collections::{BTreeMap, HashMap};

use sekai_convert::tables::{AssetEntry, TableAsset, TableContext, TableConverter, TablePlan};
use sekai_convert::{Content, ConvertError};
use sekai_format::AssetDocument;
use sekai_format::asset::MemberFile;
use sekai_format::costume::{CostumeDocument, RoleDocument, Slot};
use sekai_moesekai::costume::{CostumeRoles, Costumes};
use serde_json::{Value, json};

const MODEL: &str = "costume-model";
const TEXTURE: &str = "texture";
const MOTION: &str = "costume-motion";
const NEW: &str = "live_pv/model/characterv2/";

/// A version built in memory. Every table not set is empty.
#[derive(Default)]
struct Version {
    tables: HashMap<String, Value>,
    bundles: BTreeMap<String, Vec<AssetEntry>>,
    members: HashMap<String, Vec<String>>,
}

impl Version {
    fn table(mut self, name: &str, rows: Value) -> Self {
        self.tables.insert(name.to_owned(), rows);
        self
    }

    /// A bundle with one model, or one texture.
    fn asset(mut self, bundle: &str, file: &str, form: &'static str) -> Self {
        let name = format!("{bundle}/{file}");
        let assets = self.bundles.entry(bundle.to_owned()).or_default();
        assets.push(AssetEntry { name, form });
        assets.sort_by(|a, b| a.name.cmp(&b.name));
        self
    }

    /// A bundle with one motion that holds the clips as members.
    fn motion(mut self, bundle: &str, file: &str, clips: &[&str]) -> Self {
        let name = format!("{bundle}/{file}");
        let assets = self.bundles.entry(bundle.to_owned()).or_default();
        assets.push(AssetEntry {
            name: name.clone(),
            form: MOTION,
        });
        assets.sort_by(|a, b| a.name.cmp(&b.name));
        let members = clips.iter().map(|clip| (*clip).to_owned()).collect();
        self.members.insert(name, members);
        self
    }

    /// A model bundle of the newer directory and its one model.
    fn model(self, bundle: &str, file: &str) -> Self {
        self.asset(&format!("{NEW}{bundle}"), file, MODEL)
    }
}

impl TableContext for Version {
    fn table(&self, name: &str) -> Result<Vec<u8>, ConvertError> {
        let rows = self.tables.get(name).cloned().unwrap_or_else(|| json!([]));
        Ok(serde_json::to_vec(&rows).expect("the tables are JSON"))
    }

    fn bundles(&self) -> Vec<&str> {
        self.bundles.keys().map(String::as_str).collect()
    }

    fn assets(&self, bundle: &str) -> &[AssetEntry] {
        self.bundles.get(bundle).map_or(&[], Vec::as_slice)
    }

    fn document(&self, name: &str) -> Result<AssetDocument, ConvertError> {
        let members = self.members.get(name).map_or(&[][..], Vec::as_slice);
        Ok(AssetDocument {
            format: String::new(),
            version: 1,
            name: name.to_owned(),
            form: MOTION.to_owned(),
            form_version: 1,
            fingerprint: String::new(),
            sources: Vec::new(),
            resources: Vec::new(),
            members: members
                .iter()
                .map(|role| MemberFile {
                    role: role.clone(),
                    sha256: String::new(),
                    size: 0,
                    extension: "json".to_owned(),
                    media_type: String::new(),
                })
                .collect(),
            references: Vec::new(),
            error: None,
        })
    }

    fn member(&self, name: &str, role: &str) -> Result<Vec<u8>, ConvertError> {
        Err(ConvertError::new("store_failed", format!("{name} {role}")))
    }
}

/// A row of `costume3ds`.
fn part(id: i64, group: i64, part_type: &str, color: i64, name: &str) -> Value {
    json!({
        "id": id,
        "costume3dGroupId": group,
        "partType": part_type,
        "colorId": color,
        "characterId": 1,
        "name": name,
    })
}

/// A row of `costume3dModels` of the light unit.
fn model(id: i64, name: Option<&str>, head_type: Option<&str>) -> Value {
    model_of(id, "light", name, head_type)
}

/// A row of `costume3dModels` of another unit.
fn other_unit(id: i64, name: &str) -> Value {
    model_of(id, "street", Some(name), None)
}

/// A row of `costume3dModels`.
fn model_of(id: i64, unit: &str, name: Option<&str>, head_type: Option<&str>) -> Value {
    json!({
        "costume3dId": id,
        "unit": unit,
        "assetbundleName": name,
        "headCostume3dAssetbundleType": head_type,
    })
}

/// A `costume3dModels` table of rows of parts the role's unit wears.
fn models(rows: &[(i64, &str, Option<&str>)]) -> Value {
    Value::Array(
        rows.iter()
            .map(|(id, name, head_type)| model(*id, Some(name), *head_type))
            .collect(),
    )
}

/// A version of one role: a character in the light unit, whose default costume is the body 1,
/// the accessory 2 and the hair 3 of the light unit.
fn role_version() -> Version {
    Version::default()
        .table(
            "character3ds",
            json!([{
                "id": 1, "characterId": 1, "unit": "light",
                "bodyCostume3dId": 1, "headCostume3dId": 2, "hairCostume3dId": 3
            }]),
        )
        .table(
            "gameCharacters",
            json!([{"id": 1, "figure": "ladies", "breastSize": "m"}]),
        )
        .table(
            "gameCharacterUnits",
            json!([{
                "id": 3, "gameCharacterId": 1, "unit": "light",
                "skinColorCode": "#fff", "skinShadowColorCode1": "#eee",
                "skinShadowColorCode2": "#ddd"
            }]),
        )
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair")
            ]),
        )
        .table(
            "costume3dModels",
            models(&[
                (1, "01/0001", None),
                (2, "01/0100", Some("head_only")),
                (3, "01/0000", Some("head_and_hair")),
            ]),
        )
        .model("body/01/0001/ladies_m", "body")
        .model("head_optional/01/0100", "a")
        .model("face/01/0001", "face")
}

fn converted(plan: &dyn TablePlan) -> Vec<(String, TableAsset)> {
    let names = plan.names().to_vec();
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| (name, plan.convert(index).expect("the asset is built")))
        .collect()
}

fn bytes_of(asset: &TableAsset) -> Vec<u8> {
    match &asset.asset.members[0].content {
        Content::File { bytes, .. } => bytes.clone(),
        _ => panic!("the member is a file"),
    }
}

/// The documents of all `costume-role` assets.
fn roles(version: &Version) -> Vec<(String, RoleDocument)> {
    let plan = CostumeRoles.plan(version).expect("the tables are read");
    converted(plan.as_ref())
        .into_iter()
        .map(|(name, asset)| {
            let document = RoleDocument::from_bytes(&bytes_of(&asset)).expect("a role");
            (name, document)
        })
        .collect()
}

/// The only `costume-role` asset.
fn role(version: &Version) -> RoleDocument {
    let mut found = roles(version);
    assert_eq!(found.len(), 1, "one role");
    found.remove(0).1
}

/// The `costume-role` asset `asset`, which is in the version.
fn role_of(version: &Version, asset: &str) -> RoleDocument {
    let found = roles(version);
    let document = found.iter().find(|(name, _)| name == asset);
    match document {
        Some((_, document)) => document.clone(),
        None => panic!("no role {asset}: {:?}", role_names(&found)),
    }
}

/// The names of `costume-role` assets.
fn role_names(found: &[(String, RoleDocument)]) -> Vec<&str> {
    found.iter().map(|(name, _)| name.as_str()).collect()
}

/// The documents of all `costume` assets.
fn costumes(version: &Version) -> Vec<(String, CostumeDocument)> {
    let plan = Costumes.plan(version).expect("the tables are read");
    converted(plan.as_ref())
        .into_iter()
        .map(|(name, asset)| {
            let document = CostumeDocument::from_bytes(&bytes_of(&asset)).expect("a costume");
            (name, document)
        })
        .collect()
}

/// The `costume` asset of the costume `number`, which the version has.
fn costume(version: &Version, number: i64) -> CostumeDocument {
    let found = costumes(version);
    let wanted = format!("costume/{number}");
    let document = found.iter().find(|(name, _)| name == &wanted);
    match document {
        Some((_, document)) => document.clone(),
        None => panic!("no {wanted}: {:?}", costume_names(&found)),
    }
}

/// The names of `costume` assets.
fn costume_names(found: &[(String, CostumeDocument)]) -> Vec<&str> {
    found.iter().map(|(name, _)| name.as_str()).collect()
}

/// The model of the part with the `costume3dId` `id`.
fn part_model(role: &RoleDocument, id: i64) -> Option<String> {
    role.parts
        .iter()
        .find(|part| part.costume3d_id == id)
        .and_then(|part| part.model.as_ref())
        .map(|model| model.asset.clone())
}

#[test]
fn roles_are_the_pairs_of_a_character_and_a_unit_with_their_default_costume() {
    let version = Version::default()
        .table(
            "character3ds",
            json!([
                {"id": 20, "characterId": 1, "unit": "light",
                 "bodyCostume3dId": 9, "headCostume3dId": 8, "hairCostume3dId": 7},
                {"id": 10, "characterId": 1, "unit": "light",
                 "bodyCostume3dId": 1, "headCostume3dId": 2, "hairCostume3dId": 3},
                {"id": 11, "characterId": 2, "unit": "street",
                 "bodyCostume3dId": 4, "headCostume3dId": 5, "hairCostume3dId": 6}
            ]),
        )
        .table(
            "gameCharacters",
            json!([{"id": 1, "figure": "ladies", "breastSize": "m"}]),
        )
        .table(
            "gameCharacterUnits",
            json!([{
                "id": 3, "gameCharacterId": 1, "unit": "light",
                "skinColorCode": "#fff", "skinShadowColorCode1": "#eee",
                "skinShadowColorCode2": "#ddd"
            }]),
        );

    let documents = roles(&version);
    let found = role_names(&documents);
    assert_eq!(found, ["costume/role/1-light", "costume/role/2-street"]);

    let light = role_of(&version, "costume/role/1-light");
    assert_eq!(light.format, "costume-role");
    assert_eq!(light.version, 1);
    assert_eq!(light.role.character_id, 1);
    assert_eq!(light.role.unit, "light");
    assert_eq!(light.role.figure, "ladies");
    assert_eq!(light.role.breast_size, "m");
    assert_eq!(
        (
            light.role.skin.default.as_str(),
            light.role.skin.shadow2.as_str()
        ),
        ("#fff", "#ddd")
    );
    assert_eq!(
        (
            light.role.default_body,
            light.role.default_head,
            light.role.default_hair
        ),
        (1, 2, 3),
        "the row with the lowest id gives the default costume"
    );
    assert_eq!(light.role.motions, None, "no motion bundle in the version");
}

#[test]
fn a_part_has_a_slot_an_optional_costume_and_what_shows_it() {
    let version = role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair"),
                part(2010, 2001, "hair", 1, "twin"),
                part(4, 4, "unknown", 1, "strange")
            ]),
        )
        .table(
            "costume3dModels",
            models(&[
                (1, "01/0001", None),
                (2, "01/0100", Some("head_only")),
                (3, "01/0000", Some("head_and_hair")),
                (2010, "20/0000", Some("head_and_hair")),
                (4, "00/0000", None),
            ]),
        );
    let light = role(&version);
    let ids: Vec<i64> = light.parts.iter().map(|part| part.costume3d_id).collect();
    assert_eq!(
        ids,
        [1, 2, 3, 2010],
        "ordered, without the parts of unknown types"
    );
    assert_eq!(light.parts[0].slot, Slot::Body);
    assert_eq!(light.parts[2].slot, Slot::Hair);
    assert_eq!(light.parts[3].slot, Slot::Hair);
    assert_eq!(light.parts[0].costume_number, None);
    assert_eq!(light.parts[3].costume_number, Some(2));
    assert_eq!(light.parts[1].head_type.as_deref(), Some("head_only"));
    assert_eq!(
        part_model(&light, 2).as_deref(),
        Some(&format!("{NEW}head_optional/01/0100/a")[..])
    );
    assert_eq!(
        light.parts[1]
            .model
            .as_ref()
            .and_then(|model| model.attach.as_deref()),
        Some("0100"),
        "attach is the last segment of the name when the part is absent"
    );
    assert!(light.parts[0].missing.is_empty());
}

#[test]
fn the_parts_take_the_rows_of_the_unit_and_the_character() {
    let version = role_version().table(
        "costume3dModels",
        json!([
            model(1, Some("01/0001"), None),
            model(2, Some("01/0100"), Some("head_only")),
            model(3, Some("01/0000"), Some("head_and_hair")),
            other_unit(4, "02/0001")
        ]),
    );
    let light = role(&version);
    let ids: Vec<i64> = light.parts.iter().map(|part| part.costume3d_id).collect();
    assert_eq!(ids, [1, 2, 3], "the row of another unit is left out");
    assert!(
        part_model(&light, 4).is_none(),
        "no asset of another unit shows a part"
    );
}

#[test]
fn a_part_takes_the_model_of_the_role_s_unit() {
    let version = role_version().table(
        "costume3dModels",
        json!([
            other_unit(1, "02/0001"),
            model(1, Some("01/0001"), None),
            model(2, Some("01/0100"), Some("head_only")),
            model(3, Some("01/0000"), Some("head_and_hair")),
            other_unit(3, "05/0000")
        ]),
    );
    let light = role(&version);
    let ids: Vec<i64> = light.parts.iter().map(|part| part.costume3d_id).collect();
    assert_eq!(ids, [1, 2, 3], "one part for each row of the unit");
    assert_eq!(
        part_model(&light, 1).as_deref(),
        Some(&format!("{NEW}body/01/0001/ladies_m/body")[..]),
        "a unit can model a part differently, and the role takes its own unit's row"
    );
}

#[test]
fn a_part_names_its_own_face_without_a_suffix() {
    let light = role(&role_version());
    assert_eq!(
        part_model(&light, 3).as_deref(),
        Some(&format!("{NEW}face/01/0001/face")[..]),
        "`0000` is retried as `0001`, without the type of a head the part does not know"
    );
    assert!(light.parts[2].missing.is_empty());
}

/// A costume of one body whose head is of the type `head_all` and whose hair is `11/0000`.
fn head_all_version() -> Version {
    role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair"),
                part(1001, 1001, "body", 1, "school"),
                part(1002, 1001, "head", 1, "flower"),
                part(1003, 1001, "hair", 1, "twin")
            ]),
        )
        .table(
            "costume3dModels",
            models(&[
                (1, "01/0001", None),
                (2, "01/0100", Some("head_only")),
                (3, "01/0000", Some("head_and_hair")),
                (1001, "99/0001", None),
                (1002, "99/0100", Some("head_all")),
                (1003, "11/0000", Some("head_and_hair")),
            ]),
        )
        .model("body/99/0001/ladies_m", "body")
        .model("head_optional/99/0100", "a")
}

#[test]
fn the_face_takes_the_type_of_the_head_the_look_wears() {
    let suffixed = head_all_version().model("face/11/0001_head_all", "face");
    let look = &costume(&suffixed, 1).variants[0];
    assert_eq!(
        look.models.face.as_deref(),
        Some(&format!("{NEW}face/11/0001_head_all/face")[..])
    );
    assert!(look.missing.is_empty(), "{:?}", look.missing);

    let plain = head_all_version().model("face/11/0001", "face");
    let look = &costume(&plain, 1).variants[0];
    assert_eq!(
        look.models.face.as_deref(),
        Some(&format!("{NEW}face/11/0001/face")[..]),
        "a face without its suffix is shown instead"
    );
    assert!(
        look.missing.iter().any(|text| text.contains("head_all")),
        "{:?}",
        look.missing
    );
}

#[test]
fn a_head_with_its_hair_shows_its_own_face_without_a_suffix() {
    let version = role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair"),
                part(1001, 1001, "body", 1, "school"),
                part(1002, 1001, "head", 1, "whole"),
                part(1003, 1001, "hair", 1, "twin")
            ]),
        )
        .table(
            "costume3dModels",
            models(&[
                (1, "01/0001", None),
                (2, "01/0100", Some("head_only")),
                (3, "01/0000", Some("head_and_hair")),
                (1001, "99/0001", None),
                (1002, "21/0000", Some("head_and_hair")),
                (1003, "11/0000", Some("head_and_hair")),
            ]),
        )
        .model("body/99/0001/ladies_m", "body")
        .model("face/21/0001", "whole");
    let look = &costume(&version, 1).variants[0];
    assert_eq!((look.head, look.hair), (1002, 1003));
    assert_eq!(
        look.models.face.as_deref(),
        Some(&format!("{NEW}face/21/0001/whole")[..]),
        "the face of the head, not of the hair"
    );
    assert_eq!(look.models.accessory, None);
    assert!(look.missing.is_empty(), "{:?}", look.missing);
}

/// A costume of two colours with a body in each colour, a head and a hair.
fn school_version() -> Version {
    role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair"),
                part(1001, 1001, "body", 1, "school"),
                part(1002, 1001, "body", 2, "school"),
                part(1003, 1001, "head", 1, "hood"),
                part(1004, 1001, "head", 3, "hood"),
                part(1005, 1001, "hair", 2, "twin"),
                part(1006, 1001, "hair", 3, "spare")
            ]),
        )
        .table(
            "costume3dModels",
            models(&[
                (1, "01/0001", None),
                (2, "01/0100", Some("head_only")),
                (3, "01/0000", Some("head_and_hair")),
                (1001, "99/0001", None),
                (1002, "99/0001", None),
                (1003, "99/0100", Some("head_only")),
                (1004, "99/0101", Some("head_only")),
                (1005, "11/0000", Some("head_and_hair")),
                (1006, "12/0000", Some("head_and_hair")),
            ]),
        )
        .model("body/99/0001/ladies_m", "body")
        .model("face/12/0001", "spare")
        .model("head_optional/99/0100", "a")
        .model("head_optional/99/0101", "b")
        .model("face/11/0001", "face")
}

#[test]
fn a_look_takes_its_parts_in_the_colour_then_in_colour_1_then_the_lowest() {
    let version = school_version();
    let school = costume(&version, 1);
    let colors: Vec<i64> = school.variants.iter().map(|look| look.color_id).collect();
    assert_eq!(colors, [1, 2], "the colours of the body parts, ordered");
    assert_eq!(school.costume_number, 1);
    assert_eq!(school.name.as_deref(), Some("school"));

    let first = &school.variants[0];
    assert_eq!(first.role, "costume/role/1-light");
    assert_eq!(
        (first.body, first.head, first.hair),
        (1001, 1003, 1005),
        "the hair in colour 3 is neither this colour nor colour 1, so the hair of the \
         lowest costume3dId is worn, not the head's default hair"
    );
    assert!(
        first.alternative_heads.is_empty(),
        "the other head is in colour 3, neither this colour nor colour 1"
    );
    assert_eq!(
        first.models.face.as_deref(),
        Some(&format!("{NEW}face/11/0001/face")[..]),
        "the face of the hair worn"
    );
    assert_eq!(
        first
            .models
            .accessory
            .as_ref()
            .and_then(|model| model.attach.as_deref()),
        Some("0100")
    );

    let second = &school.variants[1];
    assert_eq!(
        (second.body, second.head, second.hair),
        (1002, 1003, 1005),
        "colour 2 body, the head in colour 1, the hair in colour 2"
    );
}

#[test]
fn a_slot_without_parts_of_the_costume_takes_the_default_part() {
    let version = role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair"),
                part(1002, 1001, "body", 1, "school"),
                part(1003, 1001, "hair", 3, "twin")
            ]),
        )
        .table(
            "costume3dModels",
            models(&[
                (1, "01/0001", None),
                (2, "01/0100", Some("head_only")),
                (3, "01/0000", Some("head_and_hair")),
                (1002, "99/0001", None),
                (1003, "11/0000", Some("head_and_hair")),
            ]),
        )
        .model("body/99/0001/ladies_m", "body")
        .model("face/11/0001", "face");
    let school = costume(&version, 1);
    let look = &school.variants[0];
    assert_eq!(
        look.color_id, 1,
        "the only body of the costume is in colour 1"
    );
    assert_eq!(look.body, 1002, "the body of the costume");
    assert_eq!(look.head, 2, "the role's default head");
    assert_eq!(look.hair, 1003, "the hair of the costume");
    assert!(
        look.missing.is_empty(),
        "taking the default part is the rule, not something missing: {:?}",
        look.missing
    );
}

#[test]
fn the_lowest_id_of_a_slot_is_worn_when_no_colour_fits() {
    let version = role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair"),
                part(1001, 1001, "body", 1, "school"),
                part(1002, 1001, "hair", 2, "lowest"),
                part(1003, 1001, "hair", 2, "higher"),
                part(1004, 1001, "hair", 4, "newest")
            ]),
        )
        .table(
            "costume3dModels",
            models(&[
                (1, "01/0001", None),
                (2, "01/0100", Some("head_only")),
                (3, "01/0000", Some("head_and_hair")),
                (1001, "99/0001", None),
                (1002, "11/0000", Some("head_and_hair")),
                (1003, "12/0000", Some("head_and_hair")),
                (1004, "13/0000", Some("head_and_hair")),
            ]),
        );
    let look = &costume(&version, 1).variants[0];
    assert_eq!(
        look.hair, 1002,
        "colour 2 is neither this colour nor colour 1, so the lowest costume3dId is worn"
    );
}

#[test]
fn a_default_part_that_the_role_does_not_have_is_missing() {
    let version = role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "hair", 1, "default hair"),
                part(1001, 1001, "body", 1, "school")
            ]),
        )
        .table(
            "costume3dModels",
            models(&[(1, "01/0001", None), (1001, "99/0001", None)]),
        )
        .model("body/99/0001/ladies_m", "body");
    let light = role(&version);
    assert_eq!(
        light.role.default_body, 1,
        "the document names the default body even when the role's rows do not"
    );
    let look = &costume(&version, 1).variants[0];
    assert_eq!(look.body, 1001, "the body of the costume");
    assert_eq!(look.head, 2, "the look writes the default's id");
    assert!(
        look.missing
            .iter()
            .any(|text| text == "the part 2 is not among the parts of the role"),
        "{:?}",
        look.missing
    );
}

#[test]
fn a_head_and_its_hair_must_be_worn_together() {
    let version = school_version().table(
        "costume3dModelNotAvailablePatterns",
        json!([{"unit": "light", "headCostume3dId": 1003, "hairCostume3dId": 1005}]),
    );
    let look = &costume(&version, 1).variants[1];
    assert_eq!((look.head, look.hair), (1003, 1005));
    assert_eq!(look.missing.len(), 1, "{:?}", look.missing);
    assert!(look.missing[0].contains("not worn together"));
}

#[test]
fn a_head_with_its_hair_shows_alone_and_replaces_the_hair() {
    let version = role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair"),
                part(1001, 1001, "body", 1, "school"),
                part(1002, 1001, "head", 1, "whole"),
                part(1003, 1001, "hair", 1, "twin")
            ]),
        )
        .table(
            "costume3dModels",
            models(&[
                (1, "01/0001", None),
                (2, "01/0100", Some("head_only")),
                (3, "01/0000", Some("head_and_hair")),
                (1001, "99/0001", None),
                (1002, "21/0000", Some("head_and_hair")),
                (1003, "11/0000", Some("head_and_hair")),
            ]),
        )
        .model("body/99/0001/ladies_m", "body")
        .model("face/21/0001", "whole")
        .model("face/11/0001", "twin");
    let look = &costume(&version, 1).variants[0];
    assert_eq!((look.head, look.hair), (1002, 1003));
    assert_eq!(
        look.models.face.as_deref(),
        Some(&format!("{NEW}face/21/0001/whole")[..]),
        "the face of the head, not of the hair"
    );
    assert_eq!(look.models.accessory, None);
    assert!(look.missing.is_empty(), "{:?}", look.missing);
}

#[test]
fn an_accessory_has_its_colour_and_the_part_it_hangs_from() {
    let version = role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair"),
                part(1001, 1001, "body", 1, "school"),
                part(1002, 1001, "head", 1, "hat"),
                part(1003, 1001, "hair", 1, "twin")
            ]),
        )
        .table(
            "costume3dModels",
            json!([
                model(1, Some("01/0001"), None),
                model(2, Some("01/0100"), Some("head_only")),
                model(3, Some("01/0000"), Some("head_and_hair")),
                model(1001, Some("99/0001"), None),
                {"costume3dId": 1002, "unit": "light", "assetbundleName": "99/0200",
                 "colorAssetbundleName": "02", "headCostume3dAssetbundleType": "head_only",
                 "part": "pivot"},
                model(1003, Some("11/0000"), Some("head_and_hair"))
            ]),
        )
        .asset(
            &format!("{NEW}color_variation/head_optional/99/0200/02"),
            "hat_C",
            TEXTURE,
        )
        .asset(
            &format!("{NEW}color_variation/head_optional/99/0200/02"),
            "hat_S",
            TEXTURE,
        )
        .asset(
            &format!("{NEW}color_variation/head_optional/99/0200/02"),
            "hat_H",
            TEXTURE,
        )
        .model("head_optional/99/0200", "a")
        .model("face/11/0001", "face");
    let look = &costume(&version, 1).variants[0];
    let accessory = look.models.accessory.as_ref().expect("an accessory");
    assert_eq!(accessory.asset, format!("{NEW}head_optional/99/0200/a"));
    assert_eq!(
        accessory.attach.as_deref(),
        Some("pivot"),
        "the part comes first"
    );
    let colors = accessory
        .colors
        .as_ref()
        .expect("the colour of the accessory");
    assert_eq!(
        colors.color.as_deref(),
        Some(&format!("{NEW}color_variation/head_optional/99/0200/02/hat_C")[..])
    );
    assert_eq!(
        colors.shadow.as_deref(),
        Some(&format!("{NEW}color_variation/head_optional/99/0200/02/hat_S")[..])
    );
    assert_eq!(
        colors.value.as_deref(),
        Some(&format!("{NEW}color_variation/head_optional/99/0200/02/hat_H")[..])
    );
}

#[test]
fn a_colour_bundle_needs_one_texture_for_each_suffix() {
    let version = role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair"),
                part(1001, 1001, "body", 2, "school")
            ]),
        )
        .table(
            "costume3dModels",
            json!([
                model(1, Some("01/0001"), None),
                model(2, Some("01/0100"), Some("head_only")),
                model(3, Some("01/0000"), Some("head_and_hair")),
                {"costume3dId": 1001, "unit": "light", "assetbundleName": "99/0001",
                 "colorAssetbundleName": "02"}
            ]),
        )
        .model("body/99/0001/ladies_m", "body")
        .asset(
            &format!("{NEW}color_variation/body/99/0001/ladies_m/02"),
            "body_C",
            TEXTURE,
        );
    let light = role(&version);
    let body = light
        .parts
        .iter()
        .find(|part| part.costume3d_id == 1001)
        .expect("the body of colour 2");
    let colors = body.model.as_ref().and_then(|model| model.colors.as_ref());
    assert_eq!(
        colors, None,
        "a colour bundle without its shadow and value textures has no colours"
    );
    assert_eq!(
        body.missing,
        ["the colour color_variation/body/99/0001/ladies_m/02 is not in the version"]
    );
}

#[test]
fn motions_take_the_unit_row_then_the_character_then_the_figure() {
    let moving = role_version()
        .table(
            "gameCharacterUnits",
            json!([
                {"id": 30, "gameCharacterId": 1, "unit": "street"},
                {"id": 4, "gameCharacterId": 1, "unit": "light"}
            ]),
        )
        .motion(
            "character/motion/costume_setting/04_00",
            "set",
            &["motion", "face", "motion_loop", "face_loop"],
        )
        .motion(
            "character/motion/costume_setting/30_00",
            "other",
            &["motion"],
        )
        .motion(
            "character/motion/costume_setting/ladies",
            "ladies",
            &["motion"],
        );
    let light = role(&moving);
    let motions = light.role.motions.as_ref().expect("the motions");
    assert_eq!(
        motions["motion"],
        "character/motion/costume_setting/04_00/set"
    );
    assert_eq!(motions.len(), 4);
    assert_eq!(
        motions["face_loop"],
        "character/motion/costume_setting/04_00/set"
    );

    let piapro = role_version()
        .table(
            "gameCharacterUnits",
            json!([{"id": 7, "gameCharacterId": 1, "unit": "piapro"}]),
        )
        .motion(
            "character/motion/costume_setting/ladies",
            "ladies",
            &["motion"],
        );
    let light = role(&piapro);
    let motions = light.role.motions.as_ref().expect("the motions");
    assert_eq!(motions.len(), 1);
    assert_eq!(
        motions["motion"], "character/motion/costume_setting/ladies/ladies",
        "the character's first row has no bundle, so the figure's motions are used"
    );
}

#[test]
fn compact_rows_take_what_they_lack_from_their_group() {
    let version = role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                {"id": 1001, "costume3dGroupId": 1001, "partType": "body", "colorId": 1},
                {"id": 1002, "costume3dGroupId": 1001, "partType": "hair", "colorId": 1},
                {"id": 9001, "costume3dGroupId": 9001, "partType": "body", "colorId": 1}
            ]),
        )
        .table(
            "costume3dGroups",
            json!([
                {"groupId": 1001, "characterId": 1, "name": "School",
                 "publishedAt": 1600000000000_i64},
                {"groupId": 9001}
            ]),
        )
        .table(
            "costume3dModels",
            models(&[
                (1, "01/0001", None),
                (1001, "99/0001", None),
                (1002, "11/0000", Some("head_and_hair")),
                (9001, "97/0001", None),
            ]),
        )
        .model("body/99/0001/ladies_m", "body")
        .model("face/11/0001", "face");
    let light = role(&version);
    let ids: Vec<i64> = light.parts.iter().map(|part| part.costume3d_id).collect();
    assert_eq!(
        ids,
        [1, 1001, 1002],
        "a row without a character is left out"
    );
    let school = &light.parts[1];
    assert_eq!(school.costume_number, Some(1));
    assert_eq!(school.name.as_deref(), Some("School"));
    assert_eq!(school.published_at, Some(1_600_000_000_000));
    assert_eq!(
        part_model(&light, 1001).as_deref(),
        Some(&format!("{NEW}body/99/0001/ladies_m/body")[..])
    );

    assert_eq!(costume_names(&costumes(&version)), ["costume/1"]);
}

#[test]
fn missing_records_the_parts_that_the_version_does_not_have() {
    let version = role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair"),
                part(1001, 1001, "body", 1, "school"),
                part(1002, 1001, "head", 1, "gone"),
                part(1003, 1001, "hair", 1, "twin")
            ]),
        )
        .table(
            "costume3dModels",
            models(&[
                (1, "01/0001", None),
                (2, "01/0100", Some("head_only")),
                (3, "01/0000", Some("head_and_hair")),
                (1001, "99/0001", None),
                (1002, "99/0300", Some("head_only")),
                (1003, "11/0000", Some("head_and_hair")),
            ]),
        )
        .model("face/11/0001", "face");
    let light = role(&version);
    let body = light
        .parts
        .iter()
        .find(|part| part.costume3d_id == 1001)
        .expect("the body");
    assert_eq!(body.model, None, "the body is not in the version");
    assert!(
        body.missing.iter().any(|text| text.contains("99/0001")),
        "{:?}",
        body.missing
    );
    let head = light
        .parts
        .iter()
        .find(|part| part.costume3d_id == 1002)
        .expect("the accessory");
    assert_eq!(head.model, None, "the accessory is not in the version");
    assert!(
        head.missing.iter().any(|text| text.contains("99/0300")),
        "{:?}",
        head.missing
    );

    let look = &costume(&version, 1).variants[0];
    assert_eq!(look.models.body, None);
    assert_eq!(look.models.accessory, None);
    assert!(
        look.missing.iter().any(|text| text.contains("99/0001")),
        "the missing parts of the worn body and accessory are the look's: {:?}",
        look.missing
    );
    assert!(
        look.missing.iter().any(|text| text.contains("99/0300")),
        "{:?}",
        look.missing
    );
    assert_eq!(
        look.models.face.as_deref(),
        Some(&format!("{NEW}face/11/0001/face")[..])
    );
}

#[test]
fn a_table_that_is_not_rows_fails_the_plan() {
    let version = role_version().table("costume3ds", json!({"not": "rows"}));
    assert!(CostumeRoles.plan(&version).is_err());
    assert!(Costumes.plan(&version).is_err());
}

#[test]
fn the_same_inputs_give_the_same_bytes() {
    let version = school_version();
    let first = role_documents(&version);
    let second = role_documents(&version);
    assert_eq!(first, second);

    let first = costume_documents(&version);
    let second = costume_documents(&version);
    assert_eq!(first, second);
}

fn role_documents(version: &Version) -> Vec<(String, Vec<u8>)> {
    let plan = CostumeRoles.plan(version).expect("the tables are read");
    converted(plan.as_ref())
        .into_iter()
        .map(|(name, asset)| (name, bytes_of(&asset)))
        .collect()
}

fn costume_documents(version: &Version) -> Vec<(String, Vec<u8>)> {
    let plan = Costumes.plan(version).expect("the tables are read");
    converted(plan.as_ref())
        .into_iter()
        .map(|(name, asset)| (name, bytes_of(&asset)))
        .collect()
}

#[test]
fn a_look_names_every_asset_it_shows() {
    let version = school_version();
    let plan = Costumes.plan(&version).expect("the tables are read");
    let assets = converted(plan.as_ref());
    let (_, school) = assets
        .iter()
        .find(|(name, _)| name == "costume/1")
        .expect("the costume is in the version");
    for name in [
        "costume/role/1-light",
        &format!("{NEW}body/99/0001/ladies_m/body"),
        &format!("{NEW}head_optional/99/0100/a"),
        &format!("{NEW}face/11/0001/face"),
    ] {
        assert!(school.references.contains(&name.to_owned()), "{name}");
    }

    let plan = CostumeRoles.plan(&version).expect("the tables are read");
    let assets = converted(plan.as_ref());
    let (_, light) = assets
        .iter()
        .find(|(name, _)| name.starts_with("costume/role/"))
        .expect("the role is in the version");
    assert!(
        light
            .references
            .contains(&format!("{NEW}body/01/0001/ladies_m/body"))
    );
    assert!(
        light
            .references
            .contains(&format!("{NEW}head_optional/01/0100/a"))
    );
}

#[test]
fn a_costume_without_a_hair_takes_the_hair_of_its_head() {
    let version = role_version()
        .table(
            "costume3ds",
            json!([
                part(1, 1, "body", 1, "casual"),
                part(2, 1, "head", 1, "default hat"),
                part(3, 1, "hair", 1, "default hair"),
                part(1001, 1001, "body", 1, "school"),
                part(1002, 1001, "head", 1, "hood"),
                part(1003, 1003, "hair", 1, "spare")
            ]),
        )
        .table(
            "costume3dModels",
            models(&[
                (1, "01/0001", None),
                (2, "01/0100", Some("head_only")),
                (3, "01/0000", Some("head_and_hair")),
                (1001, "99/0001", None),
                (1002, "99/0100", Some("head_only")),
                (1003, "12/0000", Some("head_and_hair")),
            ]),
        )
        .table(
            "costume3dModelDefaultHairs",
            json!([{"unit": "light", "headCostume3dId": 1002, "hairCostume3dId": 1003}]),
        )
        .model("body/99/0001/ladies_m", "body")
        .model("head_optional/99/0100", "a")
        .model("face/12/0001", "spare");
    let school = costume(&version, 1);
    assert_eq!(
        (school.variants[0].head, school.variants[0].hair),
        (1002, 1003),
        "the costume has no hair, so the hair of its head is worn"
    );
    assert_eq!(
        school.variants[0].missing,
        Vec::<String>::new(),
        "the hair of the head is a part of the role"
    );
    let light = role(&version);
    assert_eq!(
        light.default_hairs[&1002], 1003,
        "the table pairs take the form of the document"
    );
    assert_eq!(light.role.default_hair, 3);
}
