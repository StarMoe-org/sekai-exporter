//! The `costume-role` and `costume` documents, specified in `docs/spec/costume.md`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The value of the `format` key of a role.
pub const ROLE_FORMAT: &str = "costume-role";
/// The value of the `format` key of a costume.
pub const COSTUME_FORMAT: &str = "costume";
/// The version of both document formats.
pub const VERSION: u32 = 1;

/// One role: a character in a unit, the parts it can wear and the motions it plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleDocument {
    /// Always [`ROLE_FORMAT`].
    pub format: String,
    /// The version of the document format.
    pub version: u32,
    /// The role.
    pub role: Role,
    /// The parts the role can wear, ordered by `costume3dId`.
    pub parts: Vec<Part>,
    /// The `[head, hair]` pairs of parts that are not worn together, ordered.
    pub not_available: Vec<[i64; 2]>,
    /// Head `costume3dId` to the `costume3dId` of the hair worn with it.
    pub default_hairs: BTreeMap<i64, i64>,
}

impl RoleDocument {
    /// The bytes of the document.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// Reads a document.
    pub fn from_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

/// A character in a unit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Role {
    /// The character.
    pub character_id: i64,
    /// The unit.
    pub unit: String,
    /// The character's figure.
    pub figure: String,
    /// The character's breast size.
    pub breast_size: String,
    /// The skin colour codes.
    pub skin: Skin,
    /// The `costume3dId` of the body of the default costume.
    pub default_body: i64,
    /// The `costume3dId` of the head of the default costume.
    pub default_head: i64,
    /// The `costume3dId` of the hair of the default costume.
    pub default_hair: i64,
    /// Clip name to the name of the `costume-motion` asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motions: Option<BTreeMap<String, String>>,
}

/// The skin colour codes of a role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skin {
    /// The skin colour.
    pub default: String,
    /// The first shadow.
    pub shadow1: String,
    /// The second shadow.
    pub shadow2: String,
}

/// A part the role can wear.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    /// The `costume3dId`.
    #[serde(rename = "costume3dId")]
    pub costume3d_id: i64,
    /// The slot the part fills.
    pub slot: Slot,
    /// The costume the part belongs to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub costume_number: Option<i64>,
    /// The colour.
    pub color_id: i64,
    /// The name of the colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_name: Option<String>,
    /// The name of the costume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// For a head, the type of the head.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_type: Option<String>,
    /// When the part was released, in milliseconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<i64>,
    /// The name of the part's thumbnail image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// What shows the part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<PartModel>,
    /// What could not be resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

impl Part {
    /// The colour of the part in the default colour.
    pub const DEFAULT_COLOR: i64 = 1;
}

/// The slot a part fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Slot {
    /// A body.
    Body,
    /// A hair.
    Hair,
    /// A head: an accessory, or a head with its hair.
    Head,
}

/// What shows a part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartModel {
    /// The name of a `costume-model` asset.
    pub asset: String,
    /// The colour-variation textures of the part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colors: Option<Colors>,
    /// The name of the child of the face's `Head` node an accessory hangs from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach: Option<String>,
}

/// The colour-variation textures of a part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Colors {
    /// The name of the texture asset whose name ends in `_C`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// The name of the texture asset whose name ends in `_S`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<String>,
    /// The name of the texture asset whose name ends in `_H`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

/// One costume: the look of each role and colour that wear it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostumeDocument {
    /// Always [`COSTUME_FORMAT`].
    pub format: String,
    /// The version of the document format.
    pub version: u32,
    /// The costume.
    pub costume_number: i64,
    /// The name of the costume.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The looks, ordered by character, unit and colour.
    pub variants: Vec<Look>,
}

impl CostumeDocument {
    /// The bytes of the document.
    pub fn to_bytes(&self) -> serde_json::Result<Vec<u8>> {
        serde_json::to_vec(self)
    }

    /// Reads a document.
    pub fn from_bytes(bytes: &[u8]) -> serde_json::Result<Self> {
        serde_json::from_slice(bytes)
    }
}

/// One role in one costume in one colour.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Look {
    /// The name of the role's `costume-role` asset.
    pub role: String,
    /// The character.
    pub character_id: i64,
    /// The unit.
    pub unit: String,
    /// The colour.
    pub color_id: i64,
    /// The `costume3dId` of the body worn.
    pub body: i64,
    /// The `costume3dId` of the head worn.
    pub head: i64,
    /// The `costume3dId` of the hair worn.
    pub hair: i64,
    /// The `costume3dId`s of the costume's other heads in this colour or in colour 1, ordered.
    pub alternative_heads: Vec<i64>,
    /// The parts that show the look.
    pub models: LookModels,
    /// What could not be resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing: Vec<String>,
}

/// The parts that show a look.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookModels {
    /// The body's `model`, as a part has it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<PartModel>,
    /// The name of the `costume-model` asset of the face worn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face: Option<String>,
    /// The accessory's `model`, absent when the head is a head with its hair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accessory: Option<PartModel>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role() -> RoleDocument {
        RoleDocument {
            format: ROLE_FORMAT.to_owned(),
            version: VERSION,
            role: Role {
                character_id: 1,
                unit: "light_sound".to_owned(),
                figure: "ladies".to_owned(),
                breast_size: "m".to_owned(),
                skin: Skin {
                    default: "#ffffff".to_owned(),
                    shadow1: "#eeeeee".to_owned(),
                    shadow2: "#dddddd".to_owned(),
                },
                default_body: 2,
                default_head: 1,
                default_hair: 3,
                motions: Some(BTreeMap::from([("face".to_owned(), "a".to_owned())])),
            },
            parts: Vec::new(),
            not_available: vec![[1, 3]],
            default_hairs: BTreeMap::from([(1, 3)]),
        }
    }

    fn costume() -> CostumeDocument {
        CostumeDocument {
            format: COSTUME_FORMAT.to_owned(),
            version: VERSION,
            costume_number: 1,
            name: None,
            variants: Vec::new(),
        }
    }

    #[test]
    fn the_role_bytes_are_fixed() {
        assert_eq!(
            String::from_utf8(role().to_bytes().unwrap()).unwrap(),
            r##"{"format":"costume-role","version":1,"role":{"characterId":1,"unit":"light_sound","figure":"ladies","breastSize":"m","skin":{"default":"#ffffff","shadow1":"#eeeeee","shadow2":"#dddddd"},"defaultBody":2,"defaultHead":1,"defaultHair":3,"motions":{"face":"a"}},"parts":[],"notAvailable":[[1,3]],"defaultHairs":{"1":3}}"##
        );
    }

    #[test]
    fn a_role_reads_back() {
        let document = role();
        assert_eq!(
            RoleDocument::from_bytes(&document.to_bytes().unwrap()).unwrap(),
            document
        );
    }

    #[test]
    fn the_costume_bytes_are_fixed() {
        assert_eq!(
            String::from_utf8(costume().to_bytes().unwrap()).unwrap(),
            r#"{"format":"costume","version":1,"costumeNumber":1,"variants":[]}"#
        );
    }

    #[test]
    fn a_costume_reads_back() {
        let document = costume();
        assert_eq!(
            CostumeDocument::from_bytes(&document.to_bytes().unwrap()).unwrap(),
            document
        );
    }

    #[test]
    fn a_part_writes_its_id_and_keys_as_the_spec_names_them() {
        let part = Part {
            costume3d_id: 7,
            slot: Slot::Head,
            costume_number: Some(3),
            color_id: 2,
            color_name: None,
            name: None,
            head_type: Some("head_only".to_owned()),
            published_at: Some(1),
            thumbnail: None,
            model: None,
            missing: Vec::new(),
        };
        assert_eq!(
            serde_json::to_string(&part).unwrap(),
            "{\"costume3dId\":7,\"slot\":\"head\",\"costumeNumber\":3,\"colorId\":2,\"headType\":\"head_only\",\"publishedAt\":1}"
        );
    }

    #[test]
    fn a_look_writes_its_keys_as_the_spec_names_them() {
        let look = Look {
            role: "r".to_owned(),
            character_id: 1,
            unit: "u".to_owned(),
            color_id: 2,
            body: 3,
            head: 4,
            hair: 5,
            alternative_heads: vec![6],
            models: LookModels {
                body: None,
                face: Some("f".to_owned()),
                accessory: None,
            },
            missing: Vec::new(),
        };
        assert_eq!(
            serde_json::to_string(&look).unwrap(),
            "{\"role\":\"r\",\"characterId\":1,\"unit\":\"u\",\"colorId\":2,\"body\":3,\"head\":4,\"hair\":5,\"alternativeHeads\":[6],\"models\":{\"face\":\"f\"}}"
        );
    }

    #[test]
    fn a_part_model_omits_what_is_absent() {
        let model = PartModel {
            asset: "a".to_owned(),
            colors: Some(Colors {
                color: Some("a_C".to_owned()),
                shadow: None,
                value: None,
            }),
            attach: None,
        };
        assert_eq!(
            serde_json::to_string(&model).unwrap(),
            r#"{"asset":"a","colors":{"color":"a_C"}}"#
        );
    }
}
