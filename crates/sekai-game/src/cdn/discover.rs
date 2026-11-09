//! Version discovery: which client identity a region is read through.

use sekai_format::identity::{AssetIdentity, ClientIdentity, VersionIdentity};
use serde_json::Value;

use super::SourceError;
use super::http::Http;
use super::region::Region;

/// A client identity that has not been confirmed yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Candidate {
    pub(super) application_version: String,
    pub(super) application_hash: Option<String>,
    pub(super) asset_version: Option<String>,
    pub(super) asset_hash: Option<String>,
    pub(super) multiplayer_version: Option<String>,
}

impl Candidate {
    pub(super) fn client(&self) -> ClientIdentity {
        ClientIdentity {
            application_version: self.application_version.clone(),
            application_hash: self.application_hash.clone(),
            multiplayer_version: self.multiplayer_version.clone(),
        }
    }
}

fn configured(region: &Region) -> Option<Candidate> {
    let client = &region.client;
    Some(Candidate {
        application_version: client.application_version.clone()?,
        application_hash: client.application_hash.clone(),
        asset_version: client.asset_version.clone(),
        asset_hash: client.asset_hash.clone(),
        multiplayer_version: None,
    })
}

/// The row of `platform` in a feed document, or the document itself when rows are not split by
/// platform.
fn from_feed(document: &Value, platform: &str) -> Option<Candidate> {
    let row = document
        .get(format!("production_{platform}"))
        .unwrap_or(document);
    let text = |name: &str| row.get(name)?.as_str().map(str::to_owned);
    Some(Candidate {
        application_version: text("appVersion")?,
        application_hash: text("appHash"),
        asset_version: text("assetVersion"),
        asset_hash: text("assetHash"),
        multiplayer_version: text("multiPlayVersion"),
    })
}

fn from_previous(previous: &VersionIdentity) -> Candidate {
    let (asset_version, asset_hash) = match &previous.asset {
        AssetIdentity::Cp {
            asset_version,
            asset_hash,
            ..
        } => (Some(asset_version.clone()), Some(asset_hash.clone())),
        AssetIdentity::Nuverse { .. } | AssetIdentity::Dir { .. } => (None, None),
    };
    Candidate {
        application_version: previous.client.application_version.clone(),
        application_hash: previous.client.application_hash.clone(),
        asset_version,
        asset_hash,
        multiplayer_version: previous.client.multiplayer_version.clone(),
    }
}

/// The numbers of a dotted version, for ordering.
fn numbers(version: &str) -> Vec<u64> {
    version
        .split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}

fn read_feed(url: &str, region: &Region, http: &Http) -> Result<Candidate, SourceError> {
    let body = http.bytes(url, &Vec::new())?;
    let document: Value = serde_json::from_slice(&body)
        .map_err(|e| SourceError::new("decode", format!("{url}: {e}")))?;
    from_feed(&document, &region.platform)
        .ok_or_else(|| SourceError::new("decode", format!("{url}: no application version")))
}

/// The candidates in the order they are tried: newest first, and for one version the
/// configuration before the feed before the last complete run. With `force` the configuration is
/// the only candidate.
fn candidates(
    region: &Region,
    http: &Http,
    previous: Option<&VersionIdentity>,
    reasons: &mut Vec<String>,
) -> Vec<Candidate> {
    let mut list: Vec<Candidate> = configured(region).into_iter().collect();
    if region.force {
        return list;
    }
    if let Some(url) = &region.feed {
        match read_feed(url, region, http) {
            Ok(candidate) => list.push(candidate),
            Err(error) => reasons.push(format!("feed: {error}")),
        }
    }
    list.extend(previous.map(from_previous));
    list.sort_by_key(|candidate| std::cmp::Reverse(numbers(&candidate.application_version)));
    list
}

/// The result of `confirm` for the first candidate it accepts.
pub(super) fn first_confirmed<T>(
    region: &Region,
    http: &Http,
    previous: Option<&VersionIdentity>,
    confirm: impl Fn(&Candidate) -> Result<T, SourceError>,
) -> Result<T, SourceError> {
    let mut reasons = Vec::new();
    for candidate in candidates(region, http, previous, &mut reasons) {
        match confirm(&candidate) {
            Ok(value) => return Ok(value),
            Err(error) => reasons.push(format!("{}: {error}", candidate.application_version)),
        }
    }
    Err(SourceError::new(
        "no_version",
        if reasons.is_empty() {
            "the configuration gives no application version and no feed".to_owned()
        } else {
            reasons.join("; ")
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_by_number() {
        assert!(numbers("6.10.0") > numbers("6.9.3"));
        assert!(numbers("6.9") < numbers("6.9.0"));
    }

    #[test]
    fn a_feed_row_can_be_under_the_platform() {
        let flat = serde_json::json!({"appVersion": "1.0.0", "appHash": "a"});
        let split = serde_json::json!({"production_ios": {"appVersion": "2.0.0"}});
        assert_eq!(
            from_feed(&flat, "ios").unwrap().application_version,
            "1.0.0"
        );
        assert_eq!(
            from_feed(&split, "ios").unwrap().application_version,
            "2.0.0"
        );
        assert!(from_feed(&split, "android").is_none());
    }
}
