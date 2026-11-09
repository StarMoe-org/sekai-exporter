//! The placement grids of the furniture.
//!
//! [`FixtureMeta`] produces the `fixture-meta` form and [`FixtureAreas`] the `fixture-areas` form,
//! both specified in `docs/spec/fixture-areas.md`.

use std::collections::BTreeMap;

use sekai_convert::roots::fixture_meta;
use sekai_convert::tables::{TableContext, TableConverter, TablePlan};
use sekai_convert::{Asset, Context, ConvertError, Converter, ObjectId, ObjectIndex, Value};
use serde_json::{Map, Value as Json, json};

use crate::particle::{array, field};
use crate::scripts::{DOCUMENT, NoAssets, OneAsset, document, malformed};

const FORM: &str = "fixture-meta";
const NAME: &str = "mysekai-runtime/fixture-areas";
const PACKAGE_PREFIX: &str = "mysekai/fixture/";

const GRIDS: [&str; 4] = ["stackEnables", "motionArea", "cutsceneArea", "AddUsingGrid"];

/// The grids of one piece of furniture as a JSON document.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixtureMeta;

impl Converter for FixtureMeta {
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
        fixture_meta(object, script, container_paths)
    }

    fn convert(&self, root: ObjectId, cx: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        let object = cx.object(root)?;
        let grids = GRIDS
            .iter()
            .map(|name| Ok((*name, grid(&object.fields, name)?)))
            .collect::<Result<BTreeMap<_, _>, ConvertError>>()?;
        document(&grids).map(Some)
    }
}

/// The rows of the grid `name` of `meta`.
fn grid(meta: &Value, name: &str) -> Result<Vec<Vec<bool>>, ConvertError> {
    let rows = array(field(meta, &format!("{name}.array"))?, name)?
        .iter()
        .map(|row| cells(row, name))
        .collect::<Result<Vec<_>, _>>()?;
    if rows.iter().any(|row| row.len() != rows[0].len()) {
        return Err(malformed(format!("{name} is not rectangular")));
    }
    Ok(rows)
}

fn cells(row: &Value, name: &str) -> Result<Vec<bool>, ConvertError> {
    let columns = field(row, "columns")?
        .as_bytes()
        .ok_or_else(|| malformed(format!("the columns of {name} are not cells")))?;
    Ok(columns.iter().map(|cell| *cell != 0).collect())
}

/// The grids of every piece of furniture of a version joined into one document.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixtureAreas;

impl TableConverter for FixtureAreas {
    fn name(&self) -> &'static str {
        "fixture-areas"
    }

    fn version(&self) -> u32 {
        1
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        if meta_assets(cx).next().is_none() {
            return Ok(Box::new(NoAssets));
        }
        Ok(Box::new(OneAsset::new(NAME, move || build(cx))))
    }
}

/// The last segment of the package and the name of each `fixture-meta` asset of the version.
fn meta_assets<'c>(cx: &'c dyn TableContext) -> impl Iterator<Item = (&'c str, &'c str)> + 'c {
    cx.bundles()
        .into_iter()
        .filter_map(|bundle| Some((bundle, bundle.strip_prefix(PACKAGE_PREFIX)?)))
        .flat_map(move |(bundle, package)| {
            cx.assets(bundle)
                .iter()
                .filter(|entry| entry.form == FORM)
                .map(move |entry| (package, entry.name.as_str()))
        })
}

fn build(cx: &dyn TableContext) -> Result<Asset, ConvertError> {
    let mut packages = Map::new();
    for (package, asset) in meta_assets(cx) {
        let meta: Json = serde_json::from_slice(&cx.member(asset, DOCUMENT)?)
            .map_err(|e| malformed(format!("{asset}: {e}")))?;
        packages.insert(package.to_owned(), meta);
    }
    document(&json!({ "packages": packages }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripts::testing::{FakeVersion, json_of, structure};

    fn row(cells: &[u8]) -> Value {
        structure(&[("columns", Value::Bytes(cells.to_vec()))])
    }

    fn meta(rows: Vec<Value>) -> Value {
        structure(&[("motionArea", structure(&[("array", Value::Array(rows))]))])
    }

    #[test]
    fn a_grid_holds_its_cells_by_row() {
        assert_eq!(
            grid(&meta(vec![row(&[1, 0]), row(&[0, 2])]), "motionArea").unwrap(),
            [[true, false], [false, true]]
        );
    }

    #[test]
    fn a_grid_without_rows_is_empty() {
        assert!(grid(&meta(Vec::new()), "motionArea").unwrap().is_empty());
    }

    #[test]
    fn a_grid_that_is_not_rectangular_is_an_error() {
        let ragged = meta(vec![row(&[1, 0]), row(&[1])]);
        assert_eq!(grid(&ragged, "motionArea").unwrap_err().code, "malformed");
    }

    #[test]
    fn a_missing_grid_is_an_error() {
        assert_eq!(
            grid(&meta(Vec::new()), "stackEnables").unwrap_err().code,
            "malformed"
        );
    }

    #[test]
    fn the_packages_join_under_their_last_segment() {
        let version = FakeVersion::default()
            .with_asset(
                "mysekai/fixture/a",
                "mysekai/fixture/a/meta",
                FORM,
                json!({"motionArea": []}),
            )
            .with_asset(
                "mysekai/fixture/b",
                "mysekai/fixture/b/meta",
                FORM,
                json!({"motionArea": [[true]]}),
            )
            .with_asset(
                "mysekai/fixture/c",
                "mysekai/fixture/c/model",
                "model",
                json!({}),
            )
            .with_asset(
                "mysekai/fixture_timeline/d",
                "mysekai/fixture_timeline/d/meta",
                FORM,
                json!({}),
            );
        let plan = FixtureAreas.plan(&version).unwrap();
        assert_eq!(plan.names(), [NAME]);
        assert_eq!(
            json_of(&plan.convert(0).unwrap().asset),
            json!({"packages": {"a": {"motionArea": []}, "b": {"motionArea": [[true]]}}})
        );
    }

    #[test]
    fn a_version_without_any_grids_has_no_asset() {
        assert!(
            FixtureAreas
                .plan(&FakeVersion::default())
                .unwrap()
                .names()
                .is_empty()
        );
    }
}
