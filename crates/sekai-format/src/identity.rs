//! What identifies the state of a region.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// What decides the addresses of a manifest and its bundles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AssetIdentity {
    /// A version of the `cp` source.
    Cp {
        /// The asset version.
        asset_version: String,
        /// The asset hash.
        asset_hash: String,
        /// The profile of the distribution hosts.
        profile: String,
        /// The hash in the names of the distribution hosts.
        host_hash: String,
    },
    /// A version of the `nuverse` source.
    Nuverse {
        /// The application version the distribution paths use.
        application_version: String,
        /// The value of the version endpoint.
        value: String,
    },
    /// The contents of a local directory.
    Dir {
        /// The hash of the names and bytes of the files.
        value: String,
    },
    /// A client package: its player data and the cue sheets of its streaming assets.
    Client {
        /// The version of the application, as its player settings give it.
        application_version: String,
        /// The content of the package: the SHA-256 of a line `<bundle> <SHA-256 of the bundle>`
        /// for each of its bundles, as lowercase hexadecimal.
        value: String,
    },
}

impl AssetIdentity {
    /// The name of the version: the fields of the identity that name its files.
    pub fn name(&self) -> String {
        match self {
            AssetIdentity::Cp {
                asset_version,
                asset_hash,
                ..
            } => format!("{asset_version}-{asset_hash}"),
            AssetIdentity::Nuverse {
                application_version,
                value,
            } => format!("{application_version}-{value}"),
            AssetIdentity::Dir { value } => value.clone(),
            AssetIdentity::Client {
                application_version,
                value,
            } => {
                let start: String = value.chars().take(12).collect();
                format!("{application_version}-{start}")
            }
        }
    }
}

/// The client a version was read through.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientIdentity {
    /// The application version.
    pub application_version: String,
    /// The application hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application_hash: Option<String>,
    /// The multiplayer version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplayer_version: Option<String>,
}

/// The state of a region as a source reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionIdentity {
    /// The asset identity.
    pub asset: AssetIdentity,
    /// The client identity.
    pub client: ClientIdentity,
    /// The revision of the game data tables, when the region has a table source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tables: Option<String>,
}

impl VersionIdentity {
    /// The name of the version: the name of the asset identity and, when there is one, the start
    /// of the table revision.
    pub fn name(&self) -> String {
        match &self.tables {
            Some(tables) => {
                let start: String = tables.chars().take(12).collect();
                format!("{}-{start}", self.asset.name())
            }
            None => self.asset.name(),
        }
    }

    /// Whether the two identities name the same assets and the same tables.
    pub fn same_content(&self, other: &Self) -> bool {
        self.asset == other.asset && self.tables == other.tables
    }
}

/// A version as a run records it: its identity, and the identity of the evaluation that gave its
/// assets, by part.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedVersion {
    /// The identity of the version.
    #[serde(flatten)]
    pub identity: VersionIdentity,
    /// The identity of the evaluation, by part. A record without it comes from a run that
    /// evaluated differently from any other.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub evaluation: BTreeMap<String, String>,
}

impl RecordedVersion {
    /// The parts of the evaluation `evaluation` whose identity differs from that of the recorded
    /// one, in name order.
    pub fn changed_parts<'a>(&'a self, evaluation: &'a BTreeMap<String, String>) -> Vec<&'a str> {
        let parts: BTreeSet<&str> = self
            .evaluation
            .keys()
            .chain(evaluation.keys())
            .map(String::as_str)
            .collect();
        let changed = |part: &&str| self.evaluation.get(*part) != evaluation.get(*part);
        parts.into_iter().filter(changed).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_survives_a_round_trip() {
        let identity = VersionIdentity {
            asset: AssetIdentity::Nuverse {
                application_version: "1.2.0".to_owned(),
                value: "7".to_owned(),
            },
            client: ClientIdentity {
                application_version: "1.2.3".to_owned(),
                application_hash: None,
                multiplayer_version: None,
            },
            tables: Some("0123456789abcdef".to_owned()),
        };
        assert_eq!(identity.asset.name(), "1.2.0-7");
        assert_eq!(identity.name(), "1.2.0-7-0123456789ab");
        let text = serde_json::to_string(&identity).unwrap();
        assert_eq!(
            serde_json::from_str::<VersionIdentity>(&text).unwrap(),
            identity
        );
    }

    #[test]
    fn a_client_package_is_named_by_its_version_and_its_content() {
        let asset = AssetIdentity::Client {
            application_version: "7.0.0".to_owned(),
            value: "3fa9c1d2e4b5f60718293a4b5c6d7e8f".to_owned(),
        };
        assert_eq!(asset.name(), "7.0.0-3fa9c1d2e4b5");
        let text = serde_json::to_string(&asset).unwrap();
        assert!(text.contains(r#""kind":"client""#), "{text}");
        assert_eq!(serde_json::from_str::<AssetIdentity>(&text).unwrap(), asset);
    }

    fn dir_version() -> VersionIdentity {
        VersionIdentity {
            asset: AssetIdentity::Dir {
                value: "ab".to_owned(),
            },
            client: ClientIdentity {
                application_version: "1.0.0".to_owned(),
                application_hash: None,
                multiplayer_version: None,
            },
            tables: None,
        }
    }

    fn evaluation(parts: &[(&str, &str)]) -> BTreeMap<String, String> {
        let owned = parts
            .iter()
            .map(|(part, identity)| (part.to_string(), identity.to_string()));
        owned.collect()
    }

    #[test]
    fn a_recorded_version_keeps_the_fields_of_its_identity_beside_its_evaluation() {
        let recorded = RecordedVersion {
            identity: dir_version(),
            evaluation: evaluation(&[("form audio", "audio 5\n"), ("unit", "unit 3\n")]),
        };
        let value = serde_json::to_value(&recorded).unwrap();
        assert_eq!(value["asset"]["kind"], "dir", "{value}");
        assert_eq!(value["evaluation"]["unit"], "unit 3\n", "{value}");
        assert_eq!(
            serde_json::from_value::<RecordedVersion>(value).unwrap(),
            recorded
        );
    }

    #[test]
    fn a_version_identity_reads_as_a_version_recorded_without_its_evaluation() {
        let text = serde_json::to_string(&dir_version()).unwrap();
        let recorded: RecordedVersion = serde_json::from_str(&text).unwrap();
        assert_eq!(recorded.identity, dir_version());
        assert!(recorded.evaluation.is_empty());
    }

    #[test]
    fn the_changed_parts_are_those_that_differ_or_are_on_one_side_only() {
        let recorded = RecordedVersion {
            identity: dir_version(),
            evaluation: evaluation(&[("form a", "1"), ("form b", "1"), ("unit", "3")]),
        };
        let current = evaluation(&[("form a", "1"), ("form b", "2"), ("form c", "1")]);
        assert_eq!(
            recorded.changed_parts(&current),
            ["form b", "form c", "unit"]
        );
        assert!(recorded.changed_parts(&recorded.evaluation).is_empty());
    }
}
