//! The facial tables of the characters.
//!
//! [`FacialTable`] produces the `facial-table` form and [`FacialTables`] the `facial-tables` form,
//! both specified in `docs/spec/facial-tables.md`.

use crate::roots::facial_table;
use sekai_convert::fields::fields_document;
use sekai_convert::tables::{TableContext, TableConverter, TablePlan};
use sekai_convert::{Asset, Context, ConvertError, Converter, ObjectId, ObjectIndex};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::scripts::{DOCUMENT, NoAssets, OneAsset, document, malformed};

const FORM: &str = "facial-table";
const BUNDLE: &str = "mysekai/character_settings";
const NAME: &str = "mysekai-runtime/facial-tables";

const EYE_TABLE: &str = "npcavatareyedata";
const LIP_TABLE: &str = "npcavatarlipsyncdata";
const DEFAULT_TABLE: &str = "npcavatardefaultfacialdata";

/// One facial table as a JSON document.
#[derive(Debug, Clone, Copy, Default)]
pub struct FacialTable;

impl Converter for FacialTable {
    fn name(&self) -> &'static str {
        FORM
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(
        &self,
        object: &ObjectIndex,
        script: Option<&str>,
        container_paths: &[String],
    ) -> bool {
        facial_table(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        fields_document(root, cx, DOCUMENT).map(Some)
    }
}

/// The three facial tables of a version joined into one document.
#[derive(Debug, Clone, Copy, Default)]
pub struct FacialTables;

impl TableConverter for FacialTables {
    fn name(&self) -> &'static str {
        "facial-tables"
    }

    fn version(&self) -> u32 {
        1
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        if cx.assets(BUNDLE).iter().all(|entry| entry.form != FORM) {
            return Ok(Box::new(NoAssets));
        }
        Ok(Box::new(OneAsset::new(NAME, move || build(cx))))
    }
}

fn build(cx: &dyn TableContext) -> Result<Asset, ConvertError> {
    document(&Document {
        eyes: table_rows(cx, EYE_TABLE)?,
        lips: table_rows(cx, LIP_TABLE)?,
        defaults: table_rows(cx, DEFAULT_TABLE)?,
    })
}

/// The rows of the facial table `name`.
fn table_rows<T: DeserializeOwned>(
    cx: &dyn TableContext,
    name: &str,
) -> Result<Vec<T>, ConvertError> {
    let asset = format!("{BUNDLE}/{name}");
    let bytes = cx.member(&asset, DOCUMENT)?;
    let table: Table<T> =
        serde_json::from_slice(&bytes).map_err(|e| malformed(format!("{asset}: {e}")))?;
    Ok(table.rows)
}

/// The document of a facial table: its one list of rows, under the name its script gives it.
#[derive(Deserialize)]
struct Table<T> {
    #[serde(
        alias = "EyeDataList",
        alias = "LipSyncDataList",
        alias = "NPCAvatarDefaultFacialData"
    )]
    rows: Vec<T>,
}

#[derive(Serialize)]
struct Document {
    #[serde(rename = "NPCAvatarEyeData")]
    eyes: Vec<EyePattern>,
    #[serde(rename = "NPCAvatarLipSyncData")]
    lips: Vec<LipPattern>,
    #[serde(rename = "NPCAvatarDefaultFacialData")]
    defaults: Vec<DefaultFace>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct EyePattern {
    pattern_name: String,
    open_eye_index: i32,
    close_eye_index: i32,
    blink_enabled: u8,
    /// A table without this column belongs to a version with one eye texture.
    #[serde(default)]
    texture_index: i32,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LipPattern {
    name: String,
    open_lip_sync_index: i32,
    middle_lip_sync_index: i32,
    close_lip_sync_index: i32,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct DefaultFace {
    character_unit_id: u32,
    eye_pattern_name: String,
    mouth_pattern_name: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekai_convert::Content;
    use sekai_convert::tables::AssetEntry;
    use sekai_format::AssetDocument;
    use serde_json::{Value, json};

    /// A version whose settings package holds the tables `documents`.
    struct Settings {
        assets: Vec<AssetEntry>,
        documents: Vec<(String, Value)>,
    }

    impl Settings {
        fn new(documents: &[(&str, Value)]) -> Self {
            let documents: Vec<(String, Value)> = documents
                .iter()
                .map(|(table, value)| (format!("{BUNDLE}/{table}"), value.clone()))
                .collect();
            let assets = documents
                .iter()
                .map(|(name, _)| AssetEntry {
                    name: name.clone(),
                    form: FORM,
                })
                .collect();
            Self { assets, documents }
        }
    }

    impl TableContext for Settings {
        fn table(&self, name: &str) -> Result<Vec<u8>, ConvertError> {
            Err(ConvertError::new("missing_table", name))
        }

        fn bundles(&self) -> Vec<&str> {
            vec![BUNDLE]
        }

        fn assets(&self, bundle: &str) -> &[AssetEntry] {
            if bundle == BUNDLE { &self.assets } else { &[] }
        }

        fn document(&self, name: &str) -> Result<AssetDocument, ConvertError> {
            Err(ConvertError::new("missing_asset", name))
        }

        fn member(&self, name: &str, _role: &str) -> Result<Vec<u8>, ConvertError> {
            self.documents
                .iter()
                .find(|(asset, _)| asset == name)
                .map(|(_, value)| value.to_string().into_bytes())
                .ok_or_else(|| ConvertError::new("missing_asset", name))
        }
    }

    fn eye_table() -> Value {
        json!({ "EyeDataList": [{
            "PatternName": "normal",
            "OpenEyeIndex": 1,
            "CloseEyeIndex": 5,
            "BlinkEnabled": 1,
            "TextureIndex": 0
        }] })
    }

    fn lip_table() -> Value {
        json!({ "LipSyncDataList": [{
            "Name": "smile01",
            "OpenLipSyncIndex": 3,
            "MiddleLipSyncIndex": -1,
            "CloseLipSyncIndex": 1
        }] })
    }

    fn default_table() -> Value {
        json!({ "NPCAvatarDefaultFacialData": [{
            "CharacterUnitId": 7,
            "EyePatternName": "normal",
            "MouthPatternName": "smile01"
        }] })
    }

    fn built(cx: &dyn TableContext) -> Result<Value, ConvertError> {
        let asset = FacialTables.plan(cx)?.convert(0)?.asset;
        match &asset.members[0].content {
            Content::File { bytes, .. } => Ok(serde_json::from_slice(bytes).unwrap()),
            _ => panic!("the member is not a file"),
        }
    }

    #[test]
    fn joins_the_three_tables_under_their_asset_names() {
        let settings = Settings::new(&[
            (EYE_TABLE, eye_table()),
            (LIP_TABLE, lip_table()),
            (DEFAULT_TABLE, default_table()),
        ]);
        let document = built(&settings).unwrap();
        assert_eq!(document["NPCAvatarEyeData"], eye_table()["EyeDataList"]);
        assert_eq!(
            document["NPCAvatarLipSyncData"],
            lip_table()["LipSyncDataList"]
        );
        assert_eq!(
            document["NPCAvatarDefaultFacialData"],
            default_table()["NPCAvatarDefaultFacialData"]
        );
    }

    #[test]
    fn a_version_without_the_tables_has_no_asset() {
        let settings = Settings::new(&[]);
        assert!(FacialTables.plan(&settings).unwrap().names().is_empty());
    }

    #[test]
    fn a_missing_table_is_an_error() {
        let settings = Settings::new(&[(EYE_TABLE, eye_table())]);
        assert_eq!(built(&settings).unwrap_err().code, "missing_asset");
    }

    #[test]
    fn a_table_without_texture_indices_uses_the_first_texture() {
        let eyes = json!({ "EyeDataList": [{
            "PatternName": "normal",
            "OpenEyeIndex": 1,
            "CloseEyeIndex": 5,
            "BlinkEnabled": 0
        }] });
        let settings = Settings::new(&[
            (EYE_TABLE, eyes),
            (LIP_TABLE, lip_table()),
            (DEFAULT_TABLE, default_table()),
        ]);
        let document = built(&settings).unwrap();
        assert_eq!(document["NPCAvatarEyeData"][0]["TextureIndex"], 0);
    }

    #[test]
    fn a_missing_column_is_malformed() {
        let lips = json!({ "LipSyncDataList": [{ "Name": "smile01" }] });
        let settings = Settings::new(&[
            (EYE_TABLE, eye_table()),
            (LIP_TABLE, lips),
            (DEFAULT_TABLE, default_table()),
        ]);
        assert_eq!(built(&settings).unwrap_err().code, "malformed");
    }
}
