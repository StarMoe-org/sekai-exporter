//! The `nuverse` protocol: a version number at a path keyed by the application version.

use sekai_format::identity::{AssetIdentity, VersionIdentity};

use super::crypto::{Keys, Secrets};
use super::discover::{Candidate, first_confirmed};
use super::http::{self, Headers, Http};
use super::manifest::{self, Declared};
use super::region::{Region, render};
use super::{BundleEntry, Source, SourceError};
use crate::sink::Sink;

/// The marker a bundle file has in front of the bytes the manifest counts.
const MARKER_LEN: u64 = 4;

pub(super) struct Nuverse {
    http: Http,
    region: Region,
    keys: Keys,
}

/// The application versions to try for `version`: itself, then `x.y.0`.
fn path_versions(version: &str) -> Vec<String> {
    let mut versions = vec![version.to_owned()];
    let parts: Vec<&str> = version.split('.').collect();
    if let [major, minor, patch] = parts[..]
        && patch != "0"
    {
        versions.push(format!("{major}.{minor}.0"));
    }
    versions
}

impl Nuverse {
    pub(super) fn new(region: Region, secrets: &Secrets) -> Result<Self, SourceError> {
        if region.hosts.is_empty() {
            return Err(SourceError::new("config", "the region lists no hosts"));
        }
        Ok(Self {
            http: Http::new(&region.network)?,
            keys: Keys::read(&region.secrets, secrets)?,
            region,
        })
    }

    fn confirm(&self, candidate: &Candidate) -> Result<VersionIdentity, SourceError> {
        let mut last = None;
        for version in path_versions(&candidate.application_version) {
            let url = render(
                &self.region.urls.version,
                &[
                    ("host", &self.region.hosts[0]),
                    ("application_version", &version),
                    ("platform", &self.region.platform),
                ],
            );
            match self.http.bytes(&url, &Headers::new()) {
                Ok(body) => {
                    let text = String::from_utf8_lossy(&body);
                    let value: u64 = text.trim().parse().map_err(|_| {
                        SourceError::new("decode", format!("{url}: not a version number"))
                    })?;
                    return Ok(VersionIdentity {
                        asset: AssetIdentity::Nuverse {
                            application_version: version,
                            value: value.to_string(),
                        },
                        client: candidate.client(),
                    });
                }
                Err(error) => last = Some(error),
            }
        }
        Err(last.unwrap_or_else(|| SourceError::new("no_version", "no application version")))
    }

    fn address(
        &self,
        template: &str,
        identity: &VersionIdentity,
        host: usize,
        entry: Option<&BundleEntry>,
    ) -> String {
        let AssetIdentity::Nuverse {
            application_version,
            value,
        } = &identity.asset
        else {
            return template.to_owned();
        };
        let (download_path, bundle) = entry.map_or(("", ""), |entry| {
            (entry.download_path.as_deref().unwrap_or(""), &entry.name)
        });
        render(
            template,
            &[
                ("host", &self.region.hosts[host % self.region.hosts.len()]),
                ("application_version", application_version),
                ("platform", &self.region.platform),
                ("value", value),
                ("download_path", download_path),
                ("bundle", bundle),
            ],
        )
    }
}

impl Source for Nuverse {
    fn identity(&self, previous: Option<&VersionIdentity>) -> Result<VersionIdentity, SourceError> {
        first_confirmed(&self.region, &self.http, previous, |candidate| {
            self.confirm(candidate)
        })
    }

    fn manifest(&self, identity: &VersionIdentity) -> Result<Vec<BundleEntry>, SourceError> {
        let url = self.address(&self.region.urls.manifest, identity, 0, None);
        let body = self.http.bytes(&url, &Headers::new())?;
        manifest::read(&self.keys, &body, Declared::Crc)
    }

    fn fetch(
        &self,
        identity: &VersionIdentity,
        entry: &BundleEntry,
        sink: &mut dyn Sink,
    ) -> Result<u64, SourceError> {
        let first = entry.name.bytes().map(usize::from).sum::<usize>();
        self.http.attempts(|number| {
            if number > 0 {
                http::restart(sink)?;
            }
            let url = self.address(
                &self.region.urls.bundle,
                identity,
                first + number as usize,
                Some(entry),
            );
            let response = self.http.get(&url, &Headers::new())?;
            http::copy(&url, response, sink, Some(entry.size + MARKER_LEN))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_patch_version_falls_back_to_the_minor_version() {
        assert_eq!(path_versions("6.4.2"), ["6.4.2", "6.4.0"]);
        assert_eq!(path_versions("6.4.0"), ["6.4.0"]);
        assert_eq!(path_versions("6.4"), ["6.4"]);
    }
}
