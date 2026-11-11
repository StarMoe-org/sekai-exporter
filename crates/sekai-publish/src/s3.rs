//! S3-compatible stores.
//!
//! A publication to a bucket has the shape of the local output: every object of the store at
//! `<prefix>store/<aa>/<sha256>.<extension>`, and the pointer of a target at
//! `<prefix>published/<target>/current.json`. Requests are signed with AWS Signature Version 4.
//!
//! An object is uploaded only when the bucket does not already hold it with its size. Each upload
//! is signed with the SHA-256 of its bytes, which is also its name, so the bucket rejects bytes
//! that do not match. The pointer is written last, and only when it changes.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::File;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sekai_catalog::store::parse_hex;
use sekai_catalog::{Key, LocalStore};
use sekai_format::publication::{IndexRoot, IndexShard, Pointer};
use sha2::{Digest, Sha256};
use ureq::http::Response;
use ureq::{Agent, AsSendBody, Body};

use crate::index::{Documents, documents};
use crate::plan::{Plan, PublishError, hex};
use crate::sqlite::{self, SqliteBuild, SqliteOptions};

/// The SHA-256 of no bytes.
const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// How many times a request is sent before its failure is reported.
const ATTEMPTS: u32 = 3;
/// Cache policy for immutable content-addressed objects under `store/`.
const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";
/// Cache policy for mutable target pointers (`published/<target>/current.json`).
const POINTER_CACHE_CONTROL: &str = "no-cache";

/// Where a publication goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bucket {
    /// The address of the bucket, path-style (`https://s3.example.com/bucket`) or virtual-hosted
    /// (`https://bucket.s3.example.com`).
    pub endpoint: String,
    /// The region the requests are signed for, such as `us-east-1`.
    pub region: String,
    /// The prefix of every object name: empty, or ending in `/`.
    pub prefix: String,
    /// The number of requests in flight at once.
    pub connections: usize,
}

/// The access key of a bucket.
#[derive(Clone)]
pub struct Credentials {
    /// The access key ID.
    pub access_key_id: String,
    /// The secret access key.
    pub secret_access_key: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials { .. }")
    }
}

/// A client for one bucket.
pub struct S3 {
    agent: Agent,
    bucket: Bucket,
    credentials: Credentials,
    /// `<scheme>://<authority>` of the endpoint.
    origin: String,
    /// The value of the `Host` header.
    host: String,
    /// The path of the endpoint, empty or starting with `/`, without a trailing `/`.
    path: String,
}

impl fmt::Debug for S3 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3").field("bucket", &self.bucket).finish()
    }
}

/// What a publication to a bucket did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct S3Report {
    /// The objects uploaded.
    pub uploaded: usize,
    /// The objects the bucket already held.
    pub present: usize,
    /// Whether the pointer was written; it is not when it already pointed at the publication.
    pub pointer_written: bool,
}

impl S3 {
    /// A client for `bucket`.
    pub fn new(bucket: Bucket, credentials: Credentials) -> Result<Self, PublishError> {
        let invalid = |message: &str| {
            PublishError::new("config", format!("bucket {}: {message}", bucket.endpoint))
        };
        let (scheme, rest) = bucket
            .endpoint
            .split_once("://")
            .ok_or_else(|| invalid("the endpoint has no scheme"))?;
        let default_port = match scheme {
            "https" => ":443",
            "http" => ":80",
            _ => return Err(invalid("the scheme is not http or https")),
        };
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.is_empty() || authority.contains('@') || path.contains(['?', '#']) {
            return Err(invalid("the endpoint is not a bucket address"));
        }
        if !(bucket.prefix.is_empty() || bucket.prefix.ends_with('/')) {
            return Err(invalid("the prefix does not end in /"));
        }
        if bucket.connections == 0 {
            return Err(invalid("there are no connections"));
        }
        let path = path.trim_end_matches('/');
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(120)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .build()
            .into();
        Ok(Self {
            agent,
            origin: format!("{scheme}://{authority}"),
            host: authority
                .strip_suffix(default_port)
                .unwrap_or(authority)
                .to_owned(),
            path: if path.is_empty() {
                String::new()
            } else {
                format!("/{path}")
            },
            bucket,
            credentials,
        })
    }

    /// The name of `key` in the bucket.
    fn object(&self, key: &Key) -> String {
        format!("{}store/{}", self.bucket.prefix, key.relative_path())
    }

    /// The name of the pointer of `target` in the bucket.
    fn pointer(&self, target: &str) -> String {
        format!("{}published/{target}/current.json", self.bucket.prefix)
    }

    /// The signed headers of a request for `name`.
    fn headers(&self, method: &str, uri: &str, payload: &str) -> Vec<(&'static str, String)> {
        let time = timestamp(SystemTime::now());
        let signed = [
            ("host", self.host.as_str()),
            ("x-amz-content-sha256", payload),
            ("x-amz-date", time.as_str()),
        ];
        let signature = signature(
            &self.credentials.secret_access_key,
            &self.bucket.region,
            &time,
            method,
            uri,
            "",
            &signed,
            payload,
        );
        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}/{}/s3/aws4_request, \
             SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature={signature}",
            self.credentials.access_key_id,
            &time[..8],
            self.bucket.region,
        );
        vec![
            ("host", self.host.clone()),
            ("x-amz-content-sha256", payload.to_owned()),
            ("x-amz-date", time),
            ("authorization", authorization),
        ]
    }

    /// Sends a request for `name` until it gets a response that trying again cannot change.
    fn send(
        &self,
        method: &'static str,
        name: &str,
        payload: &str,
        request: impl Fn(&str, Vec<(&'static str, String)>) -> Result<Response<Body>, ureq::Error>,
    ) -> Result<Response<Body>, String> {
        let uri = format!("{}/{}", self.path, encode(name));
        let url = format!("{}{uri}", self.origin);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let failure = match request(&url, self.headers(method, &uri, payload)) {
                Ok(response) => {
                    let status = response.status().as_u16();
                    if !(status == 429 || status >= 500) || attempt >= ATTEMPTS {
                        return Ok(response);
                    }
                    format!("{method} {name}: HTTP {status}")
                }
                Err(e) => format!("{method} {name}: {e}"),
            };
            if attempt >= ATTEMPTS {
                return Err(failure);
            }
            std::thread::sleep(Duration::from_millis(500 << attempt));
        }
    }

    /// The size of `name`, or `None` when the bucket does not hold it.
    fn head(&self, name: &str) -> Result<Option<u64>, String> {
        let response = self.send("HEAD", name, EMPTY, |url, headers| {
            let mut request = self.agent.head(url);
            for (header, value) in headers {
                request = request.header(header, value);
            }
            request.call()
        })?;
        match response.status().as_u16() {
            200 => response
                .headers()
                .get("content-length")
                .and_then(|value| value.to_str().ok()?.parse().ok())
                .map(Some)
                .ok_or_else(|| format!("HEAD {name}: no length")),
            404 => Ok(None),
            status => Err(format!("HEAD {name}: HTTP {status}")),
        }
    }

    /// The bytes of `name`, or `None` when the bucket does not hold it.
    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        let mut response = self.send("GET", name, EMPTY, |url, headers| {
            let mut request = self.agent.get(url);
            for (header, value) in headers {
                request = request.header(header, value);
            }
            request.call()
        })?;
        match response.status().as_u16() {
            200 => response
                .body_mut()
                .read_to_vec()
                .map(Some)
                .map_err(|e| format!("GET {name}: {e}")),
            404 => Ok(None),
            status => Err(format!("GET {name}: HTTP {status}")),
        }
    }

    /// Writes the body that `body` makes to `name`. `sha256` is the SHA-256 of its bytes.
    fn put<B: AsSendBody>(
        &self,
        name: &str,
        sha256: &str,
        media_type: &str,
        cache_control: &'static str,
        body: impl Fn() -> Result<B, String>,
    ) -> Result<(), String> {
        let response = self.send("PUT", name, sha256, |url, headers| {
            let mut request = self
                .agent
                .put(url)
                .header("content-type", media_type)
                .header("cache-control", cache_control);
            for (header, value) in headers {
                request = request.header(header, value);
            }
            match body() {
                Ok(body) => request.send(body),
                Err(e) => Err(ureq::Error::Io(std::io::Error::other(e))),
            }
        })?;
        match response.status().as_u16() {
            200 | 201 | 204 => Ok(()),
            status => Err(format!("PUT {name}: HTTP {status}")),
        }
    }
}

/// Which publication layout an S3 upload writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S3Layout {
    /// Classic JSON-sharded layout (`IndexRoot`, `IndexShard`s, `Listing`s, `.asset.json`, members).
    Index,
    /// Three-tier browser-native layout (`index.sqlite`, `.tar` composite archives, standalone media).
    Sqlite(SqliteOptions),
}

impl Default for S3Layout {
    fn default() -> Self {
        Self::Sqlite(SqliteOptions::default())
    }
}

/// Publishes `plan` in the `index` layout to the bucket of `s3`: the objects first, then the
/// pointer of `target`. The shards, listings and root are written to `store` first, and every
/// object the bucket lacks is read from `store`. A failed stage leaves the previous pointer in
/// place, and running the publication again uploads only what is still missing.
pub fn publish(
    store: &LocalStore,
    s3: &S3,
    target: &str,
    plan: &Plan,
) -> Result<(Pointer, S3Report), PublishError> {
    publish_with(store, s3, target, plan, &S3Layout::Index)
}

/// Publishes `plan` in `layout` to the bucket of `s3`, skipping `HEAD` checks for objects already
/// referenced by the bucket's live pointer when that pointer's index is available in `store`.
pub fn publish_with(
    store: &LocalStore,
    s3: &S3,
    target: &str,
    plan: &Plan,
    layout: &S3Layout,
) -> Result<(Pointer, S3Report), PublishError> {
    let (pointer, all) = match layout {
        S3Layout::Index => {
            let Documents { pointer, objects } = documents(store, plan)?;
            let mut all: BTreeMap<Key, (u64, String)> = BTreeMap::new();
            for file in &plan.files {
                all.insert(file.key.clone(), (file.size, file.media_type.clone()));
            }
            for (key, size) in objects {
                all.insert(key, (size, "application/json".to_owned()));
            }
            (pointer, all)
        }
        S3Layout::Sqlite(options) => {
            let SqliteBuild {
                sqlite,
                sqlite_size,
                archives,
                direct_files,
            } = sqlite::build(store, plan, options)?;
            let root_doc = IndexRoot {
                format: "index-root".to_owned(),
                version: 1,
                published_version: plan.version.clone(),
                revision: plan.revision.clone(),
                shards: Vec::new(),
                listings: Vec::new(),
                failures: plan.failures.clone(),
            };
            let root_bytes =
                serde_json::to_vec(&root_doc).map_err(|e| PublishError::new("documents", e))?;
            let root_size = root_bytes.len() as u64;
            let root_key = store
                .put(&root_bytes, "json")
                .map_err(PublishError::wrap("documents"))?;
            let pointer = Pointer {
                format: "pointer".to_owned(),
                version: 1,
                published_version: plan.version.clone(),
                revision: plan.revision.clone(),
                root: hex(&root_key.sha256),
                sqlite: Some(hex(&sqlite.sha256)),
            };
            let mut all: BTreeMap<Key, (u64, String)> = BTreeMap::new();
            for (key, size, media_type) in direct_files {
                all.insert(key, (size, media_type));
            }
            for (key, size) in archives {
                all.insert(key, (size, "application/x-tar".to_owned()));
            }
            all.insert(root_key, (root_size, "application/json".to_owned()));
            all.insert(sqlite, (sqlite_size, "application/vnd.sqlite3".to_owned()));
            (pointer, all)
        }
    };

    let all: Vec<_> = all.into_iter().collect();
    let pointer_bytes =
        serde_json::to_vec(&pointer).map_err(|e| PublishError::new("pointer", e))?;
    let pointer_name = s3.pointer(target);

    // Pre-verify local store presence so missing store objects always surface as `verify` even
    // when the remote pointer already holds parts of the publication.
    for (key, (size, _)) in &all {
        let path = store.path(key);
        if !std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() == *size) {
            return Err(PublishError::new(
                "verify",
                format!("{key} is not in the store with {size} bytes"),
            ));
        }
    }

    // Consult the live remote pointer once: if identical, every object of `all` is already live;
    // otherwise resolve all keys of the live publication from `store` so unchanged objects skip
    // redundant `HEAD` checks.
    let remote_pointer = s3.get(&pointer_name).ok().flatten();
    if remote_pointer.as_deref() == Some(pointer_bytes.as_slice()) {
        return Ok((
            pointer,
            S3Report {
                uploaded: 0,
                present: all.len(),
                pointer_written: false,
            },
        ));
    }
    let known_live = remote_pointer
        .as_deref()
        .and_then(|raw| serde_json::from_slice::<Pointer>(raw).ok())
        .map(|prev| live_keys_from_pointer(store, &prev))
        .unwrap_or_default();

    let uploaded = AtomicUsize::new(0);
    each(all.len(), s3.bucket.connections, |i| {
        let (key, (size, media_type)) = &all[i];
        if known_live.contains(key) {
            return Ok(());
        }
        let name = s3.object(key);
        let failed = |message: String| PublishError::new("files", message);
        if s3.head(&name).map_err(failed)? == Some(*size) {
            return Ok(());
        }
        let path = store.path(key);
        s3.put(
            &name,
            &hex(&key.sha256),
            media_type,
            IMMUTABLE_CACHE_CONTROL,
            || File::open(&path).map_err(|e| format!("{key}: {e}")),
        )
        .map_err(failed)?;
        uploaded.fetch_add(1, Ordering::Relaxed);
        Ok(())
    })?;

    let failed = |message: String| PublishError::new("pointer", message);
    let pointer_written = remote_pointer.as_deref() != Some(pointer_bytes.as_slice())
        && s3.get(&pointer_name).map_err(failed)?.as_deref() != Some(pointer_bytes.as_slice());
    if pointer_written {
        let digest: [u8; 32] = Sha256::digest(&pointer_bytes).into();
        let sha256 = hex(&digest);
        s3.put(
            &pointer_name,
            &sha256,
            "application/json",
            POINTER_CACHE_CONTROL,
            || Ok(pointer_bytes.as_slice()),
        )
        .map_err(failed)?;
    }
    let uploaded = uploaded.into_inner();
    Ok((
        pointer,
        S3Report {
            uploaded,
            present: all.len() - uploaded,
            pointer_written,
        },
    ))
}

fn live_keys_from_pointer(store: &LocalStore, prev: &Pointer) -> BTreeSet<Key> {
    let mut keys = BTreeSet::new();
    let Some(root_sha) = parse_hex(&prev.root) else {
        return keys;
    };
    let root_key = Key {
        sha256: root_sha,
        extension: "json".to_owned(),
    };
    if store.contains(&root_key) {
        keys.insert(root_key.clone());
    }

    if let Some(sqlite_hex) = &prev.sqlite {
        if let Some(sqlite_sha) = parse_hex(sqlite_hex) {
            let sqlite_key = Key {
                sha256: sqlite_sha,
                extension: "sqlite".to_owned(),
            };
            if let Ok(sqlite_bytes) = store.read(&sqlite_key) {
                keys.insert(sqlite_key);
                if let Ok(referenced) = sqlite::referenced_keys(&sqlite_bytes) {
                    keys.extend(referenced);
                }
            }
        }
        return keys;
    }

    // Classic `index` layout: resolve shards, listings, and every file entry from local store.
    let Ok(root_bytes) = store.read(&root_key) else {
        return keys;
    };
    let Ok(root_doc) = serde_json::from_slice::<IndexRoot>(&root_bytes) else {
        return keys;
    };
    for listing in &root_doc.listings {
        if let Some(sha256) = parse_hex(&listing.sha256) {
            let lkey = Key {
                sha256,
                extension: "json".to_owned(),
            };
            if store.contains(&lkey) {
                keys.insert(lkey);
            }
        }
    }
    for shard_ref in &root_doc.shards {
        let Some(sha256) = parse_hex(&shard_ref.sha256) else {
            continue;
        };
        let skey = Key {
            sha256,
            extension: "json".to_owned(),
        };
        let Ok(shard_bytes) = store.read(&skey) else {
            continue;
        };
        let Ok(shard_doc) = serde_json::from_slice::<IndexShard>(&shard_bytes) else {
            continue;
        };
        keys.insert(skey);
        for entry in shard_doc.entries {
            if let Some(fsha) = parse_hex(&entry.sha256) {
                keys.insert(Key {
                    sha256: fsha,
                    extension: entry.extension,
                });
            }
        }
    }
    keys
}

/// Runs `job` for indices `0..n` on up to `threads` threads, stopping after the first failure.
/// Returns the failure of the lowest index among those that ran.
fn each(
    n: usize,
    threads: usize,
    job: impl Fn(usize) -> Result<(), PublishError> + Sync,
) -> Result<(), PublishError> {
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let failures = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..threads.min(n) {
            s.spawn(|| {
                while !stop.load(Ordering::Relaxed) {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= n {
                        break;
                    }
                    if let Err(e) = job(i) {
                        stop.store(true, Ordering::Relaxed);
                        failures
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push((i, e));
                    }
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap_or_else(|e| e.into_inner());
    match failures.into_iter().min_by_key(|(i, _)| *i) {
        Some((_, e)) => Err(e),
        None => Ok(()),
    }
}

/// Encodes a path as the canonical URI of AWS Signature Version 4: every byte except unreserved
/// characters and `/` as `%XX`.
fn encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The time of a request, `YYYYMMDDTHHMMSSZ` in UTC.
fn timestamp(now: SystemTime) -> String {
    let seconds = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (year, month, day) = civil(seconds / 86_400);
    let time = seconds % 86_400;
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
}

/// The date `days` days after 1970-01-01.
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

/// The AWS Signature Version 4 signature of a request to S3. `time` is `YYYYMMDDTHHMMSSZ`, `uri`
/// and `query` are canonical, and `headers` are the signed headers with lowercase names, in name
/// order.
#[allow(clippy::too_many_arguments)]
fn signature(
    secret: &str,
    region: &str,
    time: &str,
    method: &str,
    uri: &str,
    query: &str,
    headers: &[(&str, &str)],
    payload: &str,
) -> String {
    let date = &time[..8];
    let mut canonical = format!("{method}\n{uri}\n{query}\n");
    for (name, value) in headers {
        canonical.push_str(&format!("{name}:{}\n", value.trim()));
    }
    let names: Vec<&str> = headers.iter().map(|(name, _)| *name).collect();
    canonical.push_str(&format!("\n{}\n{payload}", names.join(";")));
    let canonical: [u8; 32] = Sha256::digest(canonical.as_bytes()).into();
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{time}\n{date}/{region}/s3/aws4_request\n{}",
        hex(&canonical)
    );
    let mut key = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    for part in [region, "s3", "aws4_request"] {
        key = hmac(&key, part.as_bytes());
    }
    hex(&hmac(&key, to_sign.as_bytes()))
}

/// HMAC-SHA256 of `message` under `key`.
fn hmac(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > block.len() {
        let digest: [u8; 32] = Sha256::digest(key).into();
        block[..32].copy_from_slice(&digest);
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|b| b ^ 0x36));
    inner.update(message);
    let mut outer = Sha256::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner.finalize());
    outer.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signs_the_published_example() {
        // The GET Object example of the AWS Signature Version 4 documentation for S3.
        let signature = signature(
            "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
            "us-east-1",
            "20130524T000000Z",
            "GET",
            "/test.txt",
            "",
            &[
                ("host", "examplebucket.s3.amazonaws.com"),
                ("range", "bytes=0-9"),
                ("x-amz-content-sha256", EMPTY),
                ("x-amz-date", "20130524T000000Z"),
            ],
            EMPTY,
        );
        assert_eq!(
            signature,
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
    }

    #[test]
    fn hmac_matches_rfc_4231() {
        // Test case 2 of RFC 4231.
        assert_eq!(
            hex(&hmac(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn formats_utc_time() {
        let at = |seconds| timestamp(UNIX_EPOCH + Duration::from_secs(seconds));
        assert_eq!(at(0), "19700101T000000Z");
        assert_eq!(at(1_369_353_600), "20130524T000000Z");
        assert_eq!(at(951_782_400 + 3661), "20000229T010101Z");
        assert_eq!(at(4_102_444_799), "20991231T235959Z");
    }

    #[test]
    fn encodes_names() {
        assert_eq!(encode("a/b-c_d.e~f"), "a/b-c_d.e~f");
        assert_eq!(encode("a b+c$"), "a%20b%2Bc%24");
    }

    #[test]
    fn reads_endpoints() {
        let bucket = |endpoint: &str, prefix: &str| Bucket {
            endpoint: endpoint.to_owned(),
            region: "us-east-1".to_owned(),
            prefix: prefix.to_owned(),
            connections: 1,
        };
        let credentials = Credentials {
            access_key_id: "a".to_owned(),
            secret_access_key: "b".to_owned(),
        };
        let s3 = S3::new(
            bucket("https://s3.example.com:443/bucket/", ""),
            credentials.clone(),
        )
        .unwrap();
        assert_eq!(
            (s3.origin.as_str(), s3.host.as_str(), s3.path.as_str()),
            ("https://s3.example.com:443", "s3.example.com", "/bucket")
        );
        let s3 = S3::new(bucket("http://127.0.0.1:9000", "p/"), credentials.clone()).unwrap();
        assert_eq!((s3.host.as_str(), s3.path.as_str()), ("127.0.0.1:9000", ""));
        let key = Key {
            sha256: [0xab; 32],
            extension: "webp".to_owned(),
        };
        assert_eq!(
            s3.object(&key),
            format!("p/store/ab/{}.webp", "ab".repeat(32))
        );
        assert_eq!(
            s3.pointer("jp/canonical"),
            "p/published/jp/canonical/current.json"
        );
        for (endpoint, prefix) in [
            ("s3.example.com", ""),
            ("ftp://s3.example.com", ""),
            ("https://user@s3.example.com", ""),
            ("https://s3.example.com", "p"),
        ] {
            let e = S3::new(bucket(endpoint, prefix), credentials.clone()).unwrap_err();
            assert_eq!(e.stage, "config");
        }
    }
}
