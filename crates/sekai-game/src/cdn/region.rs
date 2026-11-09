//! The configuration of a region: where it is distributed and how its client identifies itself.
//!
//! The configuration comes from the deployer; the exporter ships no addresses.

use serde::Deserialize;
use serde_json::Value;

use super::SourceError;

/// The distribution protocol of a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The `cp` protocol.
    Cp,
    /// The `nuverse` protocol.
    Nuverse,
}

/// The address templates of a region. `{name}` is replaced by the value of that name.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Urls {
    /// The endpoint that issues the signed cookies of the downloads, if the region has one.
    #[serde(default)]
    pub signature: Option<String>,
    /// The version endpoint.
    pub version: String,
    /// The manifest.
    pub manifest: String,
    /// A bundle.
    pub bundle: String,
}

/// The identity of the client, as far as it is known before discovery.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Client {
    /// The application version.
    pub application_version: Option<String>,
    /// The application hash.
    pub application_hash: Option<String>,
    /// The asset version.
    pub asset_version: Option<String>,
    /// The asset hash.
    pub asset_hash: Option<String>,
    /// The Unity version sent with requests.
    pub unity_version: String,
    /// The user agent sent with requests.
    pub user_agent: String,
    /// The device model sent with requests.
    pub device_model: String,
    /// The operating system sent with requests.
    pub operating_system: String,
}

/// The names of the secrets a region uses.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SecretNames {
    /// The AES key of manifests and interface responses, as hexadecimal.
    pub key: String,
    /// The AES initialization vector, as hexadecimal.
    pub iv: String,
    /// The key of encrypted HCA audio, a 64-bit number in hexadecimal. Without it, encrypted
    /// audio is reported as `missing_key`.
    #[serde(default)]
    pub hca: Option<String>,
}

/// How a source uses the network.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Network {
    /// An HTTP or SOCKS proxy. Without one, the proxy environment variables apply.
    pub proxy: Option<String>,
    /// The number of bundles downloaded at the same time. Without one, the thread count applies.
    pub concurrency: Option<usize>,
    /// The most requests started per second.
    pub rate_limit: Option<f64>,
    /// How many times a failed request is tried again.
    pub retries: u32,
    /// The first pause before trying again, in milliseconds. Each retry doubles it.
    pub back_off_ms: u64,
}

impl Default for Network {
    fn default() -> Self {
        Self {
            proxy: None,
            concurrency: None,
            rate_limit: None,
            retries: 4,
            back_off_ms: 500,
        }
    }
}

/// One region of the configuration.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Region {
    /// The distribution protocol.
    pub kind: Kind,
    /// The platform of the bundles.
    pub platform: String,
    /// The hosts that serve the same files, used in rotation for `{host}`.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// The address templates.
    pub urls: Urls,
    /// The client identity.
    #[serde(default)]
    pub client: Client,
    /// The names of the secrets.
    pub secrets: SecretNames,
    /// The address of a feed of current versions.
    #[serde(default)]
    pub feed: Option<String>,
    /// Use the client identity as it is given: it is not replaced by discovery.
    #[serde(default)]
    pub force: bool,
    /// How the source uses the network.
    #[serde(default)]
    pub network: Network,
}

impl Region {
    /// The region `name` of `config`, a JSON document of the form
    /// `{"regions": {"<name>": {...}}}`.
    pub fn load(name: &str, config: &str) -> Result<Self, SourceError> {
        let bad = |e: serde_json::Error| SourceError::new("config", format!("{name}: {e}"));
        let config: Value = serde_json::from_str(config).map_err(bad)?;
        let region = config
            .get("regions")
            .and_then(|regions| regions.get(name))
            .ok_or_else(|| {
                SourceError::new("config", format!("the configuration has no region {name}"))
            })?;
        Self::deserialize(region).map_err(bad)
    }
}

/// `template` with each `{name}` replaced by its value.
pub(super) fn render(template: &str, values: &[(&str, &str)]) -> String {
    let mut text = template.to_owned();
    for (name, value) in values {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"{"regions": {"a": {
        "kind": "nuverse",
        "platform": "ios",
        "hosts": ["h"],
        "urls": {"version": "v", "manifest": "m", "bundle": "b"},
        "client": {"application_version": "1.2.3"},
        "secrets": {"key": "K", "iv": "I", "hca": "H"},
        "network": {"retries": 1}
    }}}"#;

    #[test]
    fn a_region_is_read_from_the_configuration() {
        let region = Region::load("a", CONFIG).unwrap();
        assert_eq!(region.kind, Kind::Nuverse);
        assert_eq!(region.client.application_version.unwrap(), "1.2.3");
        assert_eq!(region.network.retries, 1);
        assert_eq!(region.network.back_off_ms, 500);
        assert_eq!(region.secrets.hca.as_deref(), Some("H"));
    }

    #[test]
    fn a_region_that_is_not_configured_is_an_error() {
        assert_eq!(Region::load("b", CONFIG).unwrap_err().code, "config");
    }

    #[test]
    fn an_unknown_entry_is_an_error() {
        let config = CONFIG.replace("\"platform\"", "\"plaform\"");
        assert_eq!(Region::load("a", &config).unwrap_err().code, "config");
    }

    #[test]
    fn templates_replace_every_name() {
        assert_eq!(render("{a}/{b}/{a}", &[("a", "1"), ("b", "2")]), "1/2/1");
    }
}
