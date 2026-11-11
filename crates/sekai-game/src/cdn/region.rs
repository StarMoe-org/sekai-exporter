//! The configuration of a region: where it is distributed and how its client identifies itself.
//!
//! The configuration comes from the deployer; the exporter ships no addresses.

use std::path::PathBuf;

use serde::Deserialize;
use serde_json::Value;

use super::SourceError;
use super::tables::TableSource;

/// The distribution protocol of a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// The `cp` protocol.
    Cp,
    /// The `nuverse` protocol.
    Nuverse,
    /// Bundles in a local directory.
    Dir,
}

/// The address templates of a region. `{name}` is replaced by the value of that name.
#[derive(Debug, Clone, Default, Deserialize)]
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
    /// The endpoint that creates a guest account.
    #[serde(default)]
    pub account: Option<String>,
    /// The endpoint that a guest account logs in to.
    #[serde(default)]
    pub login: Option<String>,
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
#[derive(Debug, Clone, Default, Deserialize)]
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
    /// The credential of the guest account. With it, the asset version of a `cp` region comes
    /// from logging in.
    #[serde(default)]
    pub account: Option<String>,
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
    /// The directory of a `dir` region.
    #[serde(default)]
    pub directory: Option<PathBuf>,
    /// The output directories of earlier runs. A bundle with the same declared hash is taken from
    /// the first of them that holds it, before the source is asked for it.
    #[serde(default)]
    pub stores: Vec<PathBuf>,
    /// The platform of the bundles.
    #[serde(default)]
    pub platform: String,
    /// The hosts that serve the same files, used in rotation for `{host}`.
    #[serde(default)]
    pub hosts: Vec<String>,
    /// The address templates.
    #[serde(default)]
    pub urls: Urls,
    /// The client identity.
    #[serde(default)]
    pub client: Client,
    /// The names of the secrets.
    #[serde(default)]
    pub secrets: SecretNames,
    /// The address of a feed of current versions.
    #[serde(default)]
    pub feed: Option<String>,
    /// Use the client identity as it is given: it is not replaced by discovery.
    #[serde(default)]
    pub force: bool,
    /// Where the game data tables come from. A region without one produces no asset that needs
    /// them.
    #[serde(default)]
    pub tables: Option<TableSource>,
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

/// `path` as it is written in a URL: every byte except `/` and the unreserved characters of
/// RFC 3986 becomes `%XX`, so that a name with a space or a non-ASCII character is a valid URI.
pub(super) fn url_path(path: &str) -> String {
    let mut text = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            text.push(char::from(byte));
        } else {
            text.push_str(&format!("%{byte:02X}"));
        }
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

    #[test]
    fn url_paths_escape_what_a_uri_does_not_allow() {
        assert_eq!(url_path("effect/material 1"), "effect/material%201");
        assert_eq!(url_path("a-b_c.d~e/é"), "a-b_c.d~e/%C3%A9");
    }
}
