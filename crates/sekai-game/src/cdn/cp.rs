//! The `cp` protocol: signed downloads whose addresses come from an encrypted version endpoint.

use std::sync::{PoisonError, RwLock};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sekai_format::identity::{AssetIdentity, ClientIdentity, VersionIdentity};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::crypto::{Keys, Secrets};
use super::discover::{Candidate, first_confirmed};
use super::http::{self, Failure, Headers, Http};
use super::manifest::{self, Declared};
use super::region::{Region, render};
use super::{BundleEntry, Source, SourceError};
use crate::sink::Sink;

const FORBIDDEN: u16 = 403;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameVersion {
    profile: String,
    assetbundle_host_hash: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NewAccount<'a> {
    platform: &'a str,
    device_model: &'a str,
    operating_system: &'a str,
}

#[derive(Deserialize)]
struct Account {
    credential: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Login<'a> {
    credential: &'a str,
    device_id: Option<&'a str>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    asset_version: String,
    asset_hash: String,
}

pub(super) struct Cp {
    http: Http,
    region: Region,
    keys: Keys,
    cookie: RwLock<Option<String>>,
    credential: Option<String>,
}

/// The user id inside an account credential, a token of three dotted parts.
fn user_id(credential: &str) -> Result<String, SourceError> {
    let bad = || SourceError::new("secret", "the account credential holds no user id");
    let payload = credential.split('.').nth(1).ok_or_else(bad)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).map_err(|_| bad())?;
    let claims: Value = serde_json::from_slice(&bytes).map_err(|_| bad())?;
    match claims.get("userId") {
        Some(Value::Number(id)) => Ok(id.to_string()),
        Some(Value::String(id)) => Ok(id.clone()),
        _ => Err(bad()),
    }
}

/// Creates the guest account of `region` and stores its credential. Returns the name of the
/// secret.
pub(super) fn create_account(mut region: Region, secrets: &Secrets) -> Result<String, SourceError> {
    let name = region
        .secrets
        .account
        .take()
        .ok_or_else(|| SourceError::new("config", "the region names no account secret"))?;
    let path = secrets.vacant(&name)?;
    let source = Cp::new(region, secrets)?;
    let credential = source.register()?;
    std::fs::write(&path, credential)
        .map_err(|e| SourceError::new("secret", format!("{}: {e}", path.display())))?;
    Ok(name)
}

/// The signed cookies among `set_cookie` as the value of a `Cookie` header.
fn signed_cookies(set_cookie: &[String]) -> Option<String> {
    let pairs: Vec<&str> = set_cookie
        .iter()
        .flat_map(|value| value.split([';', ',']))
        .map(str::trim)
        .filter(|part| part.starts_with("CloudFront-") && part.contains('='))
        .collect();
    (!pairs.is_empty()).then(|| pairs.join("; "))
}

impl Cp {
    pub(super) fn new(region: Region, secrets: &Secrets) -> Result<Self, SourceError> {
        Ok(Self {
            http: Http::new(&region.network)?,
            keys: Keys::read(&region.secrets, secrets)?,
            credential: region
                .secrets
                .account
                .as_deref()
                .map(|name| secrets.text(name))
                .transpose()?,
            region,
            cookie: RwLock::new(None),
        })
    }

    fn headers(&self, client: &ClientIdentity) -> Headers {
        let configured = &self.region.client;
        let mut headers = vec![
            ("Accept", "application/octet-stream".to_owned()),
            ("Content-Type", "application/octet-stream".to_owned()),
            ("Accept-Encoding", "identity".to_owned()),
            ("User-Agent", configured.user_agent.clone()),
            ("X-Platform", configured.operating_system.clone()),
            ("X-OperatingSystem", configured.operating_system.clone()),
            ("X-DeviceModel", configured.device_model.clone()),
            ("X-Unity-Version", configured.unity_version.clone()),
            ("X-App-Version", client.application_version.clone()),
            (
                "X-App-Hash",
                client.application_hash.clone().unwrap_or_default(),
            ),
        ];
        let cookie = self.cookie.read().unwrap_or_else(PoisonError::into_inner);
        headers.extend(cookie.iter().map(|value| ("Cookie", value.clone())));
        headers
    }

    /// Asks for new signed cookies, if the region uses them.
    fn sign(&self, client: &ClientIdentity) -> Result<(), SourceError> {
        let Some(url) = &self.region.urls.signature else {
            return Ok(());
        };
        let set_cookie = self.http.cookies(url, &self.headers(client))?;
        let cookie = signed_cookies(&set_cookie).ok_or_else(|| {
            SourceError::new("decode", "the signature endpoint set no signed cookies")
        })?;
        *self.cookie.write().unwrap_or_else(PoisonError::into_inner) = Some(cookie);
        Ok(())
    }

    fn confirm(&self, candidate: &Candidate) -> Result<VersionIdentity, SourceError> {
        let incomplete = |what: &str| SourceError::new("config", format!("no {what}"));
        let application_hash = candidate
            .application_hash
            .as_deref()
            .ok_or_else(|| incomplete("application hash"))?;
        let client = candidate.client();
        if self
            .cookie
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_none()
        {
            self.sign(&client)?;
        }
        let url = render(
            &self.region.urls.version,
            &[
                ("application_version", &candidate.application_version),
                ("application_hash", application_hash),
            ],
        );
        let body = self.http.bytes(&url, &self.headers(&client))?;
        let version: GameVersion = self.keys.decode(&body)?;
        let (asset_version, asset_hash) = match &self.credential {
            Some(credential) if !self.region.force => {
                let session = self.login(credential, &client, &version.profile)?;
                (session.asset_version, session.asset_hash)
            }
            _ => (
                candidate
                    .asset_version
                    .clone()
                    .ok_or_else(|| incomplete("asset version"))?,
                candidate.asset_hash.clone().unwrap_or_default(),
            ),
        };
        Ok(VersionIdentity {
            asset: AssetIdentity::Cp {
                asset_version,
                asset_hash,
                profile: version.profile,
                host_hash: version.assetbundle_host_hash,
            },
            client,
            tables: None,
        })
    }

    /// Logs in with the guest account, which tells the current asset version.
    fn login(
        &self,
        credential: &str,
        client: &ClientIdentity,
        profile: &str,
    ) -> Result<Session, SourceError> {
        let template = self.region.urls.login.as_deref().ok_or_else(|| {
            SourceError::new("config", "a region with an account needs a login address")
        })?;
        let url = render(
            template,
            &[("profile", profile), ("user_id", &user_id(credential)?)],
        );
        let body = self.keys.encode(&Login {
            credential,
            device_id: None,
        })?;
        self.keys
            .decode(&self.http.put(&url, &self.headers(client), &body)?)
    }

    /// Creates a guest account and returns its credential.
    fn register(&self) -> Result<String, SourceError> {
        let template = self.region.urls.account.as_deref().ok_or_else(|| {
            SourceError::new("config", "the region has no address for new accounts")
        })?;
        let identity = self.identity(None)?;
        let url = self.address(template, &identity, "");
        let configured = &self.region.client;
        let body = self.keys.encode(&NewAccount {
            platform: &configured.operating_system,
            device_model: &configured.device_model,
            operating_system: &configured.operating_system,
        })?;
        let reply = self
            .http
            .post(&url, &self.headers(&identity.client), &body)?;
        let account: Account = self.keys.decode(&reply)?;
        Ok(account.credential)
    }

    fn address(&self, template: &str, identity: &VersionIdentity, bundle: &str) -> String {
        let AssetIdentity::Cp {
            asset_version,
            asset_hash,
            profile,
            host_hash,
        } = &identity.asset
        else {
            return template.to_owned();
        };
        render(
            template,
            &[
                ("profile", profile),
                ("host_hash", host_hash),
                ("asset_version", asset_version),
                ("asset_hash", asset_hash),
                ("platform", &self.region.platform),
                ("bundle", bundle),
            ],
        )
    }
}

impl Source for Cp {
    fn identity(&self, previous: Option<&VersionIdentity>) -> Result<VersionIdentity, SourceError> {
        first_confirmed(&self.region, &self.http, previous, |candidate| {
            self.confirm(candidate)
        })
    }

    fn manifest(&self, identity: &VersionIdentity) -> Result<Vec<BundleEntry>, SourceError> {
        let url = self.address(&self.region.urls.manifest, identity, "");
        let body = self.http.bytes(&url, &self.headers(&identity.client))?;
        manifest::read(&self.keys, &body, Declared::Hash)
    }

    fn fetch(
        &self,
        identity: &VersionIdentity,
        entry: &BundleEntry,
        sink: &mut dyn Sink,
    ) -> Result<u64, SourceError> {
        let url = self.address(&self.region.urls.bundle, identity, &entry.name);
        self.http.attempts(|number| {
            if number > 0 {
                http::restart(sink)?;
            }
            let response = match self.http.get(&url, &self.headers(&identity.client)) {
                // Signed cookies expire during a long run.
                Err(Failure::Fatal(error))
                    if error.status == Some(FORBIDDEN)
                        && number == 0
                        && self.region.urls.signature.is_some() =>
                {
                    self.sign(&identity.client).map_err(Failure::Fatal)?;
                    return Err(Failure::Retry(error));
                }
                other => other?,
            };
            let expected = http::content_length(&response);
            http::copy(&url, response, sink, expected)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_user_id_is_read_from_the_middle_of_the_credential() {
        let token = |claims: &str| format!("a.{}.c", URL_SAFE_NO_PAD.encode(claims));
        assert_eq!(user_id(&token(r#"{"userId":123}"#)).unwrap(), "123");
        assert_eq!(user_id(&token(r#"{"userId":"456"}"#)).unwrap(), "456");
        for bad in [token("{}"), "plain".to_owned(), "a.***.c".to_owned()] {
            let error = user_id(&bad).unwrap_err();
            assert_eq!(error.code, "secret");
            assert!(!error.message.contains(&bad));
        }
    }

    #[test]
    fn signed_cookies_are_read_from_one_or_several_headers() {
        let joined = ["CloudFront-Policy=a; Path=/, CloudFront-Signature=b; HttpOnly".to_owned()];
        assert_eq!(
            signed_cookies(&joined).unwrap(),
            "CloudFront-Policy=a; CloudFront-Signature=b"
        );
        let several = [
            "CloudFront-Policy=a; Path=/".to_owned(),
            "CloudFront-Key-Pair-Id=c; Secure".to_owned(),
        ];
        assert_eq!(
            signed_cookies(&several).unwrap(),
            "CloudFront-Policy=a; CloudFront-Key-Pair-Id=c"
        );
        assert_eq!(signed_cookies(&["other=1".to_owned()]), None);
    }
}
