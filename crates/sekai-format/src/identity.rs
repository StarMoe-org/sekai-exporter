//! What identifies the state of a region.

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
        };
        assert_eq!(identity.asset.name(), "1.2.0-7");
        let text = serde_json::to_string(&identity).unwrap();
        assert_eq!(
            serde_json::from_str::<VersionIdentity>(&text).unwrap(),
            identity
        );
    }
}
