//! S3-compatible stores.
//!
//! A publication to a bucket has the shape of the local output: every object of the store at
//! `<prefix>store/<aa>/<sha256>.<extension>`, and the pointer of a target at
//! `<prefix>published/<target>/current.json`. Requests are signed with AWS Signature Version 4.
//!
//! What the bucket holds is recorded in [`Holdings`] as each upload is accepted, and a publication
//! uploads only the objects that are not recorded, without asking the bucket about them. Each
//! upload is signed with the SHA-256 of its bytes, which is also its name, so the bucket rejects
//! bytes that do not match. The pointer is written last, and only when it changes. When the record
//! cannot be trusted, [`S3::holdings`] lists the whole store to replace it.
//!
//! The bucket also keeps the state of the runs that publish to it, as files under
//! `<prefix>state/` that no publication refers to: [`S3::fetch_state`] and [`S3::store_state`].

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sekai_catalog::{Holdings, Key, LocalStore, Objects, Remote};
use sekai_format::publication::Pointer;
use sha2::{Digest, Sha256};
use ureq::http::Response;
use ureq::{Agent, AsSendBody, Body};

use crate::layout::{Layout, Publication, check_version, documents, version_pointer};
use crate::plan::{Plan, PublishError, hex};

/// The SHA-256 of no bytes.
const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// How many times a request is sent before its failure is reported.
const ATTEMPTS: u32 = 3;
/// Cache policy for immutable content-addressed objects under `store/`.
const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";
/// Cache policy for mutable target pointers (`published/<target>/current.json`).
const POINTER_CACHE_CONTROL: &str = "no-cache";
/// Cache policy for the state files under `state/`, which only the exporter reads.
const STATE_CACHE_CONTROL: &str = "private, no-store";
/// The store directories: the first two hexadecimal digits of the names.
const SHARDS: usize = 256;

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
    /// The objects recorded as held, which were not uploaded again.
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

    /// Every object of the store in the bucket with its size, from a listing of the whole store,
    /// one store directory per connection.
    pub fn holdings(&self) -> Result<BTreeMap<Key, u64>, PublishError> {
        let listed = self.list_store()?;
        let objects = listed.into_iter().filter_map(|(name, size)| {
            let key = name.rsplit('/').next()?.parse().ok()?;
            Some((key, size))
        });
        Ok(objects.collect())
    }

    /// The name of `key` in the bucket.
    fn object(&self, key: &Key) -> String {
        format!("{}store/{}", self.bucket.prefix, key.relative_path())
    }

    /// The name of the pointer of `target` in the bucket.
    fn pointer(&self, target: &str) -> String {
        format!("{}published/{target}/current.json", self.bucket.prefix)
    }

    /// The name of the state file `file` in the bucket, under `state/`, which no publication
    /// refers to.
    fn state(&self, file: &str) -> String {
        format!("{}state/{file}", self.bucket.prefix)
    }

    /// Copies the state file `file` of the bucket to `path`, and returns whether the bucket keeps
    /// one. The bytes go to a file beside `path` first, which replaces it once it is complete.
    pub fn fetch_state(&self, file: &str, path: &Path) -> Result<bool, PublishError> {
        let mut partial = path.as_os_str().to_owned();
        partial.push(".part");
        let fetched = self
            .get_with(&self.state(file), |body| {
                let mut out = File::create(&partial)?;
                io::copy(body, &mut out)?;
                out.sync_all()
            })
            .map_err(|e| PublishError::new("state", e))?;
        if fetched.is_some() {
            std::fs::rename(&partial, path).map_err(PublishError::wrap("state"))?;
        }
        Ok(fetched.is_some())
    }

    /// Writes the file at `path` to the bucket as its state file `file`.
    pub fn store_state(&self, file: &str, path: &Path) -> Result<(), PublishError> {
        let digest = file_sha256(path).map_err(PublishError::wrap("state"))?;
        self.put(
            &self.state(file),
            &hex(&digest),
            "application/octet-stream",
            STATE_CACHE_CONTROL,
            || File::open(path).map_err(|e| format!("{}: {e}", path.display())),
        )
        .map_err(|e| PublishError::new("state", e))
    }

    /// The signed headers of a request for `uri` with the canonical query string `query`.
    fn headers(
        &self,
        method: &str,
        uri: &str,
        query: &str,
        payload: &str,
    ) -> Vec<(&'static str, String)> {
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
            query,
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

    /// Sends a request for `name` with the canonical query string `query` until it gets a
    /// response that trying again cannot change.
    fn send(
        &self,
        method: &'static str,
        name: &str,
        query: &str,
        payload: &str,
        request: impl Fn(&str, Vec<(&'static str, String)>) -> Result<Response<Body>, ureq::Error>,
    ) -> Result<Response<Body>, String> {
        let uri = format!("{}/{}", self.path, encode(name));
        let url = match query {
            "" => format!("{}{uri}", self.origin),
            query => format!("{}{uri}?{query}", self.origin),
        };
        let mut attempt = 0;
        loop {
            attempt += 1;
            let failure = match request(&url, self.headers(method, &uri, query, payload)) {
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

    /// The bytes of `name`, or `None` when the bucket does not hold it.
    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        self.get_with(name, |body| {
            let mut bytes = Vec::new();
            body.read_to_end(&mut bytes)?;
            Ok(bytes)
        })
    }

    /// What `read` makes of the bytes of `name`, or `None` when the bucket does not hold it.
    fn get_with<T>(
        &self,
        name: &str,
        read: impl FnOnce(&mut dyn Read) -> io::Result<T>,
    ) -> Result<Option<T>, String> {
        let mut response = self.send("GET", name, "", EMPTY, |url, headers| {
            let mut request = self.agent.get(url);
            for (header, value) in headers {
                request = request.header(header, value);
            }
            request.call()
        })?;
        match response.status().as_u16() {
            200 => read(&mut response.body_mut().as_reader())
                .map(Some)
                .map_err(|e| format!("GET {name}: {e}")),
            404 => Ok(None),
            status => Err(format!("GET {name}: HTTP {status}")),
        }
    }

    /// The names under `prefix` with their sizes, from as many listing requests as it takes.
    fn list(&self, prefix: &str) -> Result<BTreeMap<String, u64>, String> {
        let mut names = BTreeMap::new();
        let mut token: Option<String> = None;
        loop {
            // The parameters in name order, as the canonical query string has them.
            let continuation = token.as_deref().map(|token| ("continuation-token", token));
            let query = continuation
                .into_iter()
                .chain([("list-type", "2"), ("prefix", prefix)])
                .map(|(name, value)| format!("{name}={}", encode_component(value)))
                .collect::<Vec<_>>()
                .join("&");
            let mut response = self.send("GET", "", &query, EMPTY, |url, headers| {
                let mut request = self.agent.get(url);
                for (header, value) in headers {
                    request = request.header(header, value);
                }
                request.call()
            })?;
            let status = response.status().as_u16();
            if status != 200 {
                return Err(format!("LIST {prefix}: HTTP {status}"));
            }
            let body = response
                .body_mut()
                .read_to_string()
                .map_err(|e| format!("LIST {prefix}: {e}"))?;
            let page = Page::parse(&body).map_err(|e| format!("LIST {prefix}: {e}"))?;
            names.extend(page.names);
            match page.next {
                Some(next) => token = Some(next),
                None => return Ok(names),
            }
        }
    }

    /// Every object of the store in the bucket with its size, listed one store directory per
    /// connection.
    fn list_store(&self) -> Result<BTreeMap<String, u64>, PublishError> {
        let names = Mutex::new(BTreeMap::new());
        each(SHARDS, self.bucket.connections, |shard| {
            let prefix = format!("{}store/{shard:02x}/", self.bucket.prefix);
            let listed = self
                .list(&prefix)
                .map_err(|e| PublishError::new("files", e))?;
            names
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend(listed);
            Ok(())
        })?;
        Ok(names.into_inner().unwrap_or_else(|e| e.into_inner()))
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
        let response = self.send("PUT", name, "", sha256, |url, headers| {
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
                Err(e) => Err(ureq::Error::Io(io::Error::other(e))),
            }
        })?;
        match response.status().as_u16() {
            200 | 201 | 204 => Ok(()),
            status => Err(format!("PUT {name}: HTTP {status}")),
        }
    }
}

impl Remote for S3 {
    fn fetch(&self, key: &Key) -> io::Result<Option<Vec<u8>>> {
        self.get(&self.object(key)).map_err(io::Error::other)
    }
}

/// Publishes `plan` as `target` in the `index` layout to the bucket of `s3`. See [`publish_with`].
pub fn publish(
    objects: &Objects<'_>,
    s3: &S3,
    target: &str,
    plan: &Plan,
    threads: usize,
) -> Result<(Pointer, S3Report), PublishError> {
    publish_with(objects, s3, target, plan, Layout::Index, threads)
}

/// Publishes `plan` as `target` in `layout` to the bucket of `s3`: the objects first, then the
/// pointer of its version, then the current pointer of `target`. The documents are written to the
/// local store of `objects`, whose holdings record what the bucket holds. Every object they do not
/// record is uploaded from the local store, and recorded once the bucket accepts it, also when the
/// publication fails later. A failed stage leaves the previous pointer in place, and running the
/// publication again uploads only what is still unrecorded. The documents are written on up to
/// `threads` threads.
pub fn publish_with(
    objects: &Objects<'_>,
    s3: &S3,
    target: &str,
    plan: &Plan,
    layout: Layout,
    threads: usize,
) -> Result<(Pointer, S3Report), PublishError> {
    check_version(&plan.version)?;
    let Publication {
        pointer,
        objects: referred,
    } = documents(objects.local, target, plan, layout, threads)?;
    let referred: BTreeMap<Key, (u64, String)> = referred
        .into_iter()
        .map(|object| (object.key, (object.size, object.media_type)))
        .collect();
    let referred: Vec<_> = referred.into_iter().collect();
    let pointer_bytes =
        serde_json::to_vec(&pointer).map_err(|e| PublishError::new("pointer", e))?;
    let pointer_name = s3.pointer(target);
    let remote_pointer = s3
        .get(&pointer_name)
        .map_err(|e| PublishError::new("pointer", e))?;

    let unrecorded = unrecorded(objects.held, &referred)?;
    let confirmed = Mutex::new(Vec::with_capacity(unrecorded.len()));
    let uploading = upload(objects.local, s3, &unrecorded, &confirmed);
    let confirmed = confirmed.into_inner().unwrap_or_else(|e| e.into_inner());
    objects
        .held
        .hold(&confirmed)
        .map_err(PublishError::wrap("holdings"))?;
    uploading?;
    let uploaded = unrecorded.len();

    let pointer_written = remote_pointer.as_deref() != Some(pointer_bytes.as_slice());
    if pointer_written {
        let digest: [u8; 32] = Sha256::digest(&pointer_bytes).into();
        let sha256 = hex(&digest);
        let version_name = format!(
            "{}published/{}",
            s3.bucket.prefix,
            version_pointer(target, &plan.version)
        );
        for name in [&version_name, &pointer_name] {
            s3.put(
                name,
                &sha256,
                "application/json",
                POINTER_CACHE_CONTROL,
                || Ok(pointer_bytes.as_slice()),
            )
            .map_err(|e| PublishError::new("pointer", e))?;
        }
    }
    Ok((
        pointer,
        S3Report {
            uploaded,
            present: referred.len() - uploaded,
            pointer_written,
        },
    ))
}

/// An object a publication refers to: its key, its size and its media type.
type Referred = (Key, (u64, String));

/// The objects of `referred`, in key order, that `held` does not record with their size.
fn unrecorded<'a>(
    held: &dyn Holdings,
    referred: &'a [Referred],
) -> Result<Vec<&'a Referred>, PublishError> {
    let mut unknown = Vec::new();
    for directory in referred.chunk_by(|(a, _), (b, _)| a.sha256[0] == b.sha256[0]) {
        let sizes = held
            .sizes(directory[0].0.sha256[0])
            .map_err(PublishError::wrap("holdings"))?;
        unknown.extend(
            directory
                .iter()
                .filter(|(key, (size, _))| sizes.get(key) != Some(size)),
        );
    }
    Ok(unknown)
}

/// Uploads each of `unrecorded` from `local`, adding every object the bucket then holds to
/// `confirmed`. The bucket is not asked first: an object is named by its bytes, so writing it
/// again changes nothing.
fn upload(
    local: &LocalStore,
    s3: &S3,
    unrecorded: &[&Referred],
    confirmed: &Mutex<Vec<(Key, u64)>>,
) -> Result<(), PublishError> {
    each(unrecorded.len(), s3.bucket.connections, |i| {
        let (key, (size, media_type)) = unrecorded[i];
        let path = local.path(key);
        if !std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() == *size) {
            return Err(PublishError::new(
                "verify",
                format!("{key} is neither in the store with {size} bytes nor recorded as held"),
            ));
        }
        s3.put(
            &s3.object(key),
            &hex(&key.sha256),
            media_type,
            IMMUTABLE_CACHE_CONTROL,
            || File::open(&path).map_err(|e| format!("{key}: {e}")),
        )
        .map_err(|message| PublishError::new("files", message))?;
        confirmed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((key.clone(), *size));
        Ok(())
    })
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

/// The SHA-256 of the bytes of the file at `path`.
fn file_sha256(path: &Path) -> io::Result<[u8; 32]> {
    let mut file = BufReader::with_capacity(1 << 20, File::open(path)?);
    let mut hasher = Sha256::new();
    loop {
        let chunk = file.fill_buf()?;
        if chunk.is_empty() {
            return Ok(hasher.finalize().into());
        }
        hasher.update(chunk);
        let read = chunk.len();
        file.consume(read);
    }
}

/// Encodes a path as the canonical URI of AWS Signature Version 4: every byte except unreserved
/// characters and `/` as `%XX`.
fn encode(path: &str) -> String {
    percent_encode(path, |byte| byte == b'/')
}

/// Encodes a query parameter as the canonical query string of AWS Signature Version 4: every
/// byte except unreserved characters as `%XX`.
fn encode_component(value: &str) -> String {
    percent_encode(value, |_| false)
}

/// `text` with every byte as `%XX` except unreserved characters and those `keep` accepts.
fn percent_encode(text: &str, keep: impl Fn(u8) -> bool) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') || keep(byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// One page of a listing.
#[derive(Debug, PartialEq, Eq)]
struct Page {
    /// The names on the page with their sizes.
    names: Vec<(String, u64)>,
    /// The continuation token of the next page, when the listing goes on.
    next: Option<String>,
}

impl Page {
    /// Reads the XML of a `ListObjectsV2` response.
    fn parse(xml: &str) -> Result<Self, String> {
        let names = elements(xml, "Contents")
            .map(|contents| {
                let name = element(contents, "Key").ok_or("an object has no Key")?;
                let size = element(contents, "Size")
                    .and_then(|size| size.parse().ok())
                    .ok_or("an object has no Size")?;
                Ok((unescape(name), size))
            })
            .collect::<Result<_, &str>>()?;
        let next = match element(xml, "IsTruncated") {
            Some("true") => Some(unescape(
                element(xml, "NextContinuationToken").ok_or("a truncated page has no token")?,
            )),
            _ => None,
        };
        Ok(Self { names, next })
    }
}

/// The text of every element `tag` in `xml`, in order.
fn elements<'a>(xml: &'a str, tag: &str) -> impl Iterator<Item = &'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut rest = xml;
    std::iter::from_fn(move || {
        let start = rest.find(&open)? + open.len();
        let end = start + rest[start..].find(&close)?;
        let text = &rest[start..end];
        rest = &rest[end + close.len()..];
        Some(text)
    })
}

/// The text of the first element `tag` in `xml`.
fn element<'a>(xml: &'a str, tag: &str) -> Option<&'a str> {
    elements(xml, tag).next()
}

/// `text` with the five predefined XML entities replaced by their characters.
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
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
    fn reads_listing_pages() {
        let xml = "<ListBucketResult><IsTruncated>true</IsTruncated>\
            <Contents><Key>p/store/ab/a&amp;b.json</Key><Size>12</Size></Contents>\
            <Contents><Key>p/store/ab/c.png</Key><Size>3</Size></Contents>\
            <NextContinuationToken>t+1/=</NextContinuationToken></ListBucketResult>";
        assert_eq!(
            Page::parse(xml),
            Ok(Page {
                names: vec![
                    ("p/store/ab/a&b.json".to_owned(), 12),
                    ("p/store/ab/c.png".to_owned(), 3)
                ],
                next: Some("t+1/=".to_owned()),
            })
        );
        let last = "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>";
        assert_eq!(
            Page::parse(last),
            Ok(Page {
                names: Vec::new(),
                next: None
            })
        );
        assert!(Page::parse("<Contents><Key>x</Key></Contents>").is_err());
        assert_eq!(encode_component("t+1/="), "t%2B1%2F%3D");
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
