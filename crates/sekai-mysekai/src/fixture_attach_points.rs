//! The attach points of every piece of furniture of a version.
//!
//! [`FixtureAttachPoints`] produces the `fixture-attach-points` form, specified in
//! `docs/spec/fixture-attach-points.md`: the `attach-points` member of every `fixture-model`
//! asset, joined under the last segment of its package.

use std::collections::BTreeMap;

use sekai_convert::ConvertError;
use sekai_convert::tables::{TableContext, TableConverter, TablePlan};
use serde::Serialize;
use serde_json::value::RawValue;

use crate::scripts::{NoAssets, OneAsset, document, malformed};

const FORM: &str = "fixture-model";
const MEMBER: &str = "attach-points";
const NAME: &str = "mysekai-runtime/fixture-attach-points";
const PACKAGE_PREFIX: &str = "mysekai/fixture/";

/// The attach points of every piece of furniture of a version joined into one document.
#[derive(Debug, Clone, Copy, Default)]
pub struct FixtureAttachPoints;

impl TableConverter for FixtureAttachPoints {
    fn name(&self) -> &'static str {
        "fixture-attach-points"
    }

    fn version(&self) -> u32 {
        1
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        if models(cx).next().is_none() {
            return Ok(Box::new(NoAssets));
        }
        Ok(Box::new(OneAsset::new(NAME, move || build(cx))))
    }
}

/// The last segment of the package and the name of each `fixture-model` asset of the version.
fn models<'c>(cx: &'c dyn TableContext) -> impl Iterator<Item = (&'c str, &'c str)> + 'c {
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

/// The joined document. Each member is kept as its text, so that its numbers are not read and
/// written again.
#[derive(Serialize)]
struct Joined<'c> {
    packages: BTreeMap<&'c str, Box<RawValue>>,
}

fn build(cx: &dyn TableContext) -> Result<sekai_convert::Asset, ConvertError> {
    let mut packages = BTreeMap::new();
    for (package, asset) in models(cx) {
        let text = String::from_utf8(cx.member(asset, MEMBER)?)
            .map_err(|e| malformed(format!("{asset}: {e}")))?;
        let points = RawValue::from_string(text).map_err(|e| malformed(format!("{asset}: {e}")))?;
        packages.insert(package, points);
    }
    document(&Joined { packages })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scripts::testing::{FakeVersion, json_of};
    use serde_json::{Value as Json, json};

    fn points(n: usize) -> Json {
        json!({
            "root": {"sourceObject": {"file": "f", "gameObjectId": "1", "transformId": "2"}, "scale": [1.0, 1.0, 1.0]},
            "collider": null,
            "points": vec![json!({"start": {"name": "loc_start001"}, "end": null}); n],
        })
    }

    #[test]
    fn the_packages_join_under_their_last_segment() {
        let version = FakeVersion::default()
            .with_asset("mysekai/fixture/a", "mysekai/fixture/a/a", FORM, points(2))
            .with_asset("mysekai/fixture/b", "mysekai/fixture/b/b", FORM, points(0))
            .with_asset(
                "mysekai/fixture/c",
                "mysekai/fixture/c/fixture_metadata",
                "fixture-meta",
                json!({}),
            )
            .with_asset(
                "mysekai/fixture_timeline/d",
                "mysekai/fixture_timeline/d/d",
                FORM,
                points(1),
            );
        let plan = FixtureAttachPoints.plan(&version).unwrap();
        assert_eq!(plan.names(), [NAME]);
        assert_eq!(
            json_of(&plan.convert(0).unwrap().asset),
            json!({"packages": {"a": points(2), "b": points(0)}})
        );
    }

    #[test]
    fn a_member_is_joined_as_its_text() {
        // Read and written again, this number would come out as 7.888609000000001e-33.
        let member = br#"{"root":null,"collider":null,"points":[{"start":{"position":[7.888609e-33,0.0,-0.46294108]},"end":null}]}"#;
        let version = FakeVersion::default().with_raw_asset(
            "mysekai/fixture/a",
            "mysekai/fixture/a/a",
            FORM,
            member.to_vec(),
        );
        let plan = FixtureAttachPoints.plan(&version).unwrap();
        let joined = plan.convert(0).unwrap().asset;
        let sekai_convert::Content::File { bytes, .. } = &joined.members[0].content else {
            panic!("the document is a file")
        };
        let mut expected = br#"{"packages":{"a":"#.to_vec();
        expected.extend_from_slice(member);
        expected.extend_from_slice(b"}}");
        assert_eq!(*bytes, expected);
    }

    #[test]
    fn a_member_that_is_not_json_is_an_error() {
        let version = FakeVersion::default().with_raw_asset(
            "mysekai/fixture/a",
            "mysekai/fixture/a/a",
            FORM,
            b"not json".to_vec(),
        );
        let plan = FixtureAttachPoints.plan(&version).unwrap();
        assert_eq!(plan.convert(0).unwrap_err().code, "malformed");
    }

    #[test]
    fn a_version_without_any_furniture_model_has_no_asset() {
        assert!(
            FixtureAttachPoints
                .plan(&FakeVersion::default())
                .unwrap()
                .names()
                .is_empty()
        );
    }
}
