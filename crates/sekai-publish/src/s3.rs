//! S3-compatible stores.
//!
//! A publication to a bucket has the shape of the local output: every object of the store at
//! `<prefix>store/<aa>/<sha256>.<extension>`, and the pointer of a target at
//! `<prefix>published/<target>/current.json`. Requests are signed with AWS Signature Version 4.
//!
//! What the bucket holds is recorded in [`Holdings`](sekai_catalog::Holdings) as each upload is accepted, and a publication
//! uploads only the local objects that are not recorded, without asking the bucket about them.
//! Each upload is signed with the SHA-256 of its bytes, which is also its name, so the bucket
//! rejects bytes that do not match. An object larger than 16 MiB is written as a multipart upload,
//! so that each request stays short; its bytes are hashed as they are read, and the upload is
//! aborted when they do not match its name. The pointer is written last, and only when it changes.
//! When the record cannot be trusted, [`S3::holdings`] lists the whole store to replace it.
//!
//! The bucket also keeps the state of the runs that publish to it, as files under
//! `<prefix>state/` that no publication refers to: [`S3::read_state`] and [`S3::write_state`].
//! A state file larger than 16 MiB is written as a multipart upload as well.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use sekai_catalog::{Key, LocalStore, Objects, Remote, Store};
use sekai_format::publication::Pointer;
use sha2::{Digest, Sha256};
use ureq::http::Response;
use ureq::{Agent, AsSendBody, Body};

use crate::clock::Utc;
use crate::layout::{
    Layout, Listed, Object, Publication, check_version, documents, version_pointer,
};
use crate::plan::{Plan, PublishError, hex};

/// The SHA-256 of no bytes.
const EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// How many times a request is sent before its failure is reported. Between attempts the client
/// waits 1, 2, 4 and then 8 seconds, so that a busy service has time to recover.
const ATTEMPTS: u32 = 5;
/// Cache policy for immutable content-addressed objects under `store/`.
const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";
/// Cache policy for mutable target pointers (`published/<target>/current.json`).
const POINTER_CACHE_CONTROL: &str = "no-cache";
/// Cache policy for the state files under `state/`, which only the exporter reads.
const STATE_CACHE_CONTROL: &str = "private, no-store";
/// The media type of the state files.
const STATE_MEDIA_TYPE: &str = "application/octet-stream";
/// The size of the parts of a large object or state file. Each part is a request of its own,
/// short enough that no service between the client and the bucket cuts it off and that its
/// signature is still current when it arrives; S3 asks for at least 5 MiB.
const PART: u64 = 16 << 20;
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
    /// The number of listing requests in flight at once, which a service may answer more slowly
    /// than others. At most [`Bucket::connections`] are in flight all the same.
    pub listings: usize,
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
    /// One slot for each of the connections of the bucket.
    slots: Slots,
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
        if bucket.connections == 0 || bucket.listings == 0 {
            return Err(invalid("there are no connections"));
        }
        let path = path.trim_end_matches('/');
        // A listing of a large store can take minutes before the service sends anything. Every
        // connection in flight stays open for the next request, so that none is opened again.
        let agent = Agent::config_builder()
            .http_status_as_error(false)
            .max_idle_connections(bucket.connections)
            .max_idle_connections_per_host(bucket.connections)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(300)))
            .timeout_per_read(Some(Duration::from_secs(300)))
            .build()
            .into();
        Ok(Self {
            agent,
            slots: Slots::new(bucket.connections),
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

    /// The number of requests in flight at once.
    pub fn connections(&self) -> usize {
        self.bucket.connections
    }

    /// Every object of the store in the bucket with its size, from a listing of the whole store,
    /// one store directory per connection.
    pub fn holdings(&self) -> Result<BTreeMap<Key, u64>, PublishError> {
        let listed = self.list_store()?;
        let objects = listed.into_iter().filter_map(|file| {
            let key = file.name.rsplit('/').next()?.parse().ok()?;
            Some((key, file.size))
        });
        Ok(objects.collect())
    }

    /// Every file under `directory` of the bucket, named without the prefix of the bucket, from
    /// one listing.
    pub fn files(&self, directory: &str) -> Result<Vec<Listed>, PublishError> {
        let listed = self.list(&format!("{}{directory}/", self.bucket.prefix));
        Ok(self.unprefixed(listed.map_err(|e| PublishError::new("listing", e))?))
    }

    /// [`S3::files`] of a directory whose files lie in subdirectories named by two hexadecimal
    /// digits, listed one subdirectory per connection.
    pub fn sharded_files(&self, directory: &str) -> Result<Vec<Listed>, PublishError> {
        let directory = format!("{}{directory}", self.bucket.prefix);
        Ok(self.unprefixed(self.list_sharded(&directory, "listing")?))
    }

    /// The files of `listed`, named without the prefix of the bucket.
    fn unprefixed(&self, listed: Vec<Listed>) -> Vec<Listed> {
        let unprefixed = listed.into_iter().filter_map(|file| {
            let name = file.name.strip_prefix(&self.bucket.prefix)?.to_owned();
            Some(Listed { name, ..file })
        });
        unprefixed.collect()
    }

    /// Deletes the file `name` of the bucket, named without the prefix of the bucket. A file the
    /// bucket does not hold is deleted already.
    pub fn delete(&self, name: &str) -> Result<(), PublishError> {
        let name = format!("{}{name}", self.bucket.prefix);
        self.send_delete(&name, "")
            .map_err(|e| PublishError::new("delete", e))
    }

    /// The name of `key` in the bucket.
    fn object(&self, key: &Key) -> String {
        format!("{}store/{}", self.bucket.prefix, key.relative_path())
    }

    /// The name of the pointer of `target` in the bucket.
    fn pointer(&self, target: &str) -> String {
        format!("{}published/{target}/current.json", self.bucket.prefix)
    }

    /// The name of the pointer of the published version `version` of `target` in the bucket.
    fn version_pointer(&self, target: &str, version: &str) -> String {
        format!(
            "{}published/{}",
            self.bucket.prefix,
            version_pointer(target, version)
        )
    }

    /// The current pointer of `target`, or `None` when the bucket has none.
    pub fn current(&self, target: &str) -> Result<Option<Pointer>, PublishError> {
        self.read_pointer(&self.pointer(target))
    }

    /// The pointer of the published version `version` of `target`, or `None` when the bucket has
    /// none.
    pub fn version(&self, target: &str, version: &str) -> Result<Option<Pointer>, PublishError> {
        check_version(version)?;
        self.read_pointer(&self.version_pointer(target, version))
    }

    /// The pointer named `name` in the bucket, or `None` when the bucket has none.
    fn read_pointer(&self, name: &str) -> Result<Option<Pointer>, PublishError> {
        let bytes = self
            .get(name)
            .map_err(|e| PublishError::new("pointer", e))?;
        bytes
            .map(|bytes| serde_json::from_slice(&bytes))
            .transpose()
            .map_err(|e| PublishError::new("pointer", e))
    }

    /// The name of the state file `file` in the bucket, under `state/`, which no publication
    /// refers to.
    fn state(&self, file: &str) -> String {
        format!("{}state/{file}", self.bucket.prefix)
    }

    /// The bytes of the state file `file` of the bucket, or `None` when it keeps none.
    pub fn read_state(&self, file: &str) -> Result<Option<Vec<u8>>, PublishError> {
        self.get(&self.state(file))
            .map_err(|e| PublishError::new("state", e))
    }

    /// Writes `bytes` to the bucket as its state file `file`, in parts when there are more than
    /// 16 MiB.
    pub fn write_state(&self, file: &str, bytes: &[u8]) -> Result<(), PublishError> {
        let name = self.state(file);
        let written = if bytes.len() as u64 > PART {
            self.put_in_parts(&name, STATE_MEDIA_TYPE, STATE_CACHE_CONTROL, bytes)
        } else {
            let digest = Sha256::digest(bytes);
            self.put(
                &name,
                &hex(&digest),
                STATE_MEDIA_TYPE,
                STATE_CACHE_CONTROL,
                || Ok(bytes),
            )
        };
        written.map_err(|e| PublishError::new("state", e))
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
    /// response that trying again cannot change, and returns it with its body read. An attempt
    /// fails when the connection fails before the last byte of the body, so a response cut off
    /// midway is asked for again.
    fn send(
        &self,
        method: &'static str,
        name: &str,
        query: &str,
        payload: &str,
        request: impl Fn(&str, Vec<(&'static str, String)>) -> Result<Response<Body>, ureq::Error>,
    ) -> Result<Reply, String> {
        let uri = format!("{}/{}", self.path, encode(name));
        let url = match query {
            "" => format!("{}{uri}", self.origin),
            query => format!("{}{uri}?{query}", self.origin),
        };
        let mut attempt = 0;
        loop {
            attempt += 1;
            let reply = self.exchange(|| request(&url, self.headers(method, &uri, query, payload)));
            let failure = match reply {
                Ok(reply) => {
                    let status = reply.status().as_u16();
                    if !transient(status) || attempt >= ATTEMPTS {
                        return Ok(reply);
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

    /// One attempt of a request: waits for a free slot, so that no more requests are in flight
    /// than the bucket has connections, sends the request that `request` makes and reads the
    /// whole response.
    fn exchange(
        &self,
        request: impl FnOnce() -> Result<Response<Body>, ureq::Error>,
    ) -> Result<Reply, ureq::Error> {
        let _slot = self.slots.take();
        let (head, mut body) = request()?.into_parts();
        let mut bytes = Vec::new();
        body.as_reader().read_to_end(&mut bytes)?;
        Ok(Response::from_parts(head, bytes))
    }

    /// The bytes of `name`, or `None` when the bucket does not hold it.
    fn get(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        let reply = self.send("GET", name, "", EMPTY, |url, headers| {
            signed(self.agent.get(url), headers).call()
        })?;
        match reply.status().as_u16() {
            200 => Ok(Some(reply.into_body())),
            404 => Ok(None),
            status => Err(format!("GET {name}: HTTP {status}")),
        }
    }

    /// The files under `prefix`, from as many listing requests as it takes.
    fn list(&self, prefix: &str) -> Result<Vec<Listed>, String> {
        let mut files = Vec::new();
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
            let reply = self.send("GET", "", &query, EMPTY, |url, headers| {
                signed(self.agent.get(url), headers).call()
            })?;
            let body = success_text("LIST", prefix, reply)?;
            let page = Page::parse(&body).map_err(|e| format!("LIST {prefix}: {e}"))?;
            files.extend(page.files);
            match page.next {
                Some(next) => token = Some(next),
                None => return Ok(files),
            }
        }
    }

    /// Every object of the store in the bucket.
    fn list_store(&self) -> Result<Vec<Listed>, PublishError> {
        let store = format!("{}store", self.bucket.prefix);
        self.list_sharded(&store, "files")
    }

    /// The names of the state files in the state directory `directory` of the bucket, which holds
    /// no directories, as [`S3::read_state`] names them.
    pub fn state_names(&self, directory: &str) -> Result<Vec<String>, PublishError> {
        let listed = self.list(&self.state(&format!("{directory}/")));
        let listed = listed.map_err(|e| PublishError::new("state", e))?;
        let state = self.state("");
        let names = listed
            .into_iter()
            .filter_map(|file| file.name.strip_prefix(&state).map(str::to_owned));
        Ok(names.collect())
    }

    /// Every object under `directory`, where the objects lie in subdirectories named by two
    /// hexadecimal digits, listed one subdirectory per connection. A failure is an error of
    /// `stage`.
    fn list_sharded(
        &self,
        directory: &str,
        stage: &'static str,
    ) -> Result<Vec<Listed>, PublishError> {
        let names = Mutex::new(Vec::new());
        each(SHARDS, self.bucket.listings, |shard| {
            let prefix = format!("{directory}/{shard:02x}/");
            let listed = self
                .list(&prefix)
                .map_err(|e| PublishError::new(stage, e))?;
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
            let request = signed(self.agent.put(url), headers)
                .header("content-type", media_type)
                .header("cache-control", cache_control);
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

    /// Writes the local file `path` as the object `key` of `size` bytes, as a multipart upload when
    /// it is larger than one part. A part is checked by the bucket against its own SHA-256, so the
    /// bytes of a multipart upload are checked against `key` as they are read instead.
    fn put_object(
        &self,
        key: &Key,
        size: u64,
        media_type: &str,
        path: &Path,
    ) -> Result<(), String> {
        let name = self.object(key);
        let open = || File::open(path).map_err(|e| format!("{key}: {e}"));
        if size > PART {
            let source = Named::new(open()?, key.sha256);
            self.put_in_parts(&name, media_type, IMMUTABLE_CACHE_CONTROL, source)
        } else {
            self.put(
                &name,
                &hex(&key.sha256),
                media_type,
                IMMUTABLE_CACHE_CONTROL,
                open,
            )
        }
    }

    /// Writes the bytes of `source` to `name` as a multipart upload of [`PART`]-byte parts. An
    /// upload that fails is aborted, so that the bucket does not keep its parts.
    fn put_in_parts(
        &self,
        name: &str,
        media_type: &str,
        cache_control: &'static str,
        source: impl Read,
    ) -> Result<(), String> {
        let upload = self.start_upload(name, media_type, cache_control)?;
        self.put_parts(name, &upload, source)
            .and_then(|tags| self.complete_upload(name, &upload, &tags))
            .map_err(|e| match self.abort_upload(name, &upload) {
                Ok(()) => e,
                Err(abort) => format!("{e}; {abort}"),
            })
    }

    /// Starts a multipart upload to `name` and returns its upload ID.
    fn start_upload(
        &self,
        name: &str,
        media_type: &str,
        cache_control: &'static str,
    ) -> Result<String, String> {
        let response = self.send("POST", name, "uploads=", EMPTY, |url, headers| {
            signed(self.agent.post(url), headers)
                .header("content-type", media_type)
                .header("cache-control", cache_control)
                .send_empty()
        })?;
        let body = success_text("POST", name, response)?;
        element(&body, "UploadId")
            .map(unescape)
            .ok_or_else(|| format!("POST {name}: the response has no UploadId"))
    }

    /// Uploads `source` as the parts of the multipart upload `upload` of `name`, one after
    /// another, and returns the entity tags of the parts in order.
    fn put_parts(
        &self,
        name: &str,
        upload: &str,
        mut source: impl Read,
    ) -> Result<Vec<String>, String> {
        let mut tags = Vec::new();
        let mut part = Vec::new();
        loop {
            part.clear();
            source
                .by_ref()
                .take(PART)
                .read_to_end(&mut part)
                .map_err(|e| format!("{name}: {e}"))?;
            if part.is_empty() {
                return Ok(tags);
            }
            tags.push(self.put_part(name, upload, tags.len() + 1, &part)?);
        }
    }

    /// Uploads `bytes` as part `number` of the multipart upload `upload` of `name`, and returns
    /// the entity tag of the part.
    fn put_part(
        &self,
        name: &str,
        upload: &str,
        number: usize,
        bytes: &[u8],
    ) -> Result<String, String> {
        let query = format!("partNumber={number}&uploadId={}", encode_component(upload));
        let digest = Sha256::digest(bytes);
        let response = self.send("PUT", name, &query, &hex(&digest), |url, headers| {
            signed(self.agent.put(url), headers).send(bytes)
        })?;
        match response.status().as_u16() {
            200 => response
                .headers()
                .get("etag")
                .and_then(|tag| tag.to_str().ok())
                .map(str::to_owned)
                .ok_or_else(|| format!("PUT {name} part {number}: the response has no ETag")),
            status => Err(format!("PUT {name} part {number}: HTTP {status}")),
        }
    }

    /// Completes the multipart upload `upload` of `name` from the parts with the entity tags
    /// `tags`, in order.
    fn complete_upload(&self, name: &str, upload: &str, tags: &[String]) -> Result<(), String> {
        let parts: String = tags
            .iter()
            .zip(1..)
            .map(|(tag, number)| {
                format!(
                    "<Part><PartNumber>{number}</PartNumber><ETag>{}</ETag></Part>",
                    escape(tag)
                )
            })
            .collect();
        let xml = format!("<CompleteMultipartUpload>{parts}</CompleteMultipartUpload>");
        let digest = Sha256::digest(xml.as_bytes());
        let query = format!("uploadId={}", encode_component(upload));
        let response = self.send("POST", name, &query, &hex(&digest), |url, headers| {
            signed(self.agent.post(url), headers).send(xml.as_bytes())
        })?;
        // The service may answer 200 and report a failure in the body.
        let body = success_text("POST", name, response)?;
        if body.contains("<CompleteMultipartUploadResult") {
            Ok(())
        } else {
            Err(format!("POST {name}: the upload was not completed"))
        }
    }

    /// Aborts the multipart upload `upload` of `name`, so that the bucket drops its parts.
    fn abort_upload(&self, name: &str, upload: &str) -> Result<(), String> {
        let query = format!("uploadId={}", encode_component(upload));
        self.send_delete(name, &query)
    }

    /// Sends `DELETE` of `name` with the canonical query string `query`.
    fn send_delete(&self, name: &str, query: &str) -> Result<(), String> {
        let response = self.send("DELETE", name, query, EMPTY, |url, headers| {
            signed(self.agent.delete(url), headers).call()
        })?;
        match response.status().as_u16() {
            200 | 204 => Ok(()),
            status => Err(format!("DELETE {name}: HTTP {status}")),
        }
    }
}

/// The requests a client may have in flight: a request waits for a free slot and holds it until
/// its response is dropped.
struct Slots {
    free: Mutex<usize>,
    freed: Condvar,
}

/// A slot taken from [`Slots`], given back when dropped.
struct Slot<'s>(&'s Slots);

impl Slots {
    fn new(slots: usize) -> Self {
        Self {
            free: Mutex::new(slots),
            freed: Condvar::new(),
        }
    }

    fn free(&self) -> MutexGuard<'_, usize> {
        self.free.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A free slot, once there is one.
    fn take(&self) -> Slot<'_> {
        let mut free = self.free();
        while *free == 0 {
            free = self.freed.wait(free).unwrap_or_else(|e| e.into_inner());
        }
        *free -= 1;
        Slot(self)
    }
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        *self.0.free() += 1;
        self.0.freed.notify_one();
    }
}

/// A response with its whole body.
type Reply = Response<Vec<u8>>;

/// `request` with the signed `headers`.
fn signed<B>(
    request: ureq::RequestBuilder<B>,
    headers: Vec<(&'static str, String)>,
) -> ureq::RequestBuilder<B> {
    headers.into_iter().fold(request, |request, (name, value)| {
        request.header(name, value)
    })
}

/// Whether a response with `status` may be different when the request is sent again: the server
/// timed out waiting for the request (408), is busy (429), saw the connection close before it
/// answered (499), or failed (5xx).
fn transient(status: u16) -> bool {
    matches!(status, 408 | 429 | 499) || status >= 500
}

/// The body of `reply` to `method` on `name` as text, when the reply is a success.
fn success_text(method: &str, name: &str, reply: Reply) -> Result<String, String> {
    match reply.status().as_u16() {
        200 => String::from_utf8(reply.into_body()).map_err(|e| format!("{method} {name}: {e}")),
        status => Err(format!("{method} {name}: HTTP {status}")),
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
) -> Result<(Pointer, S3Report), PublishError> {
    publish_with(objects, s3, target, plan, Layout::Index)
}

/// Publishes `plan` as `target` in `layout` to the bucket of `s3`, writing its documents to the
/// local store of `objects` first. See [`upload`].
pub fn publish_with(
    objects: &Objects<'_>,
    s3: &S3,
    target: &str,
    plan: &Plan,
    layout: Layout,
) -> Result<(Pointer, S3Report), PublishError> {
    check_version(&plan.version)?;
    let publication = documents(objects.local, target, plan, layout)?;
    upload(objects, s3, target, publication)
}

/// Uploads `publication`, whose documents are in the local store of `objects`, as `target` to the
/// bucket of `s3`: the objects with [`store_objects`], then the pointers with [`make_current`].
pub fn upload(
    objects: &Objects<'_>,
    s3: &S3,
    target: &str,
    publication: Publication,
) -> Result<(Pointer, S3Report), PublishError> {
    let Publication {
        pointer,
        objects: referred,
    } = publication;
    check_version(&pointer.published_version)?;
    let Stored { uploaded, present } = store_objects(objects, s3, referred)?;
    let pointer_written = make_current(s3, target, &pointer)?;
    Ok((
        pointer,
        S3Report {
            uploaded,
            present,
            pointer_written,
        },
    ))
}

/// What [`store_objects`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stored {
    /// The objects uploaded.
    pub uploaded: usize,
    /// The objects taken to be in the bucket already, which were not uploaded.
    pub present: usize,
}

/// Uploads the objects of `referred` to the bucket of `s3`.
///
/// The holdings of `objects` record what the bucket holds. Every object that they do not record
/// and that the local store of `objects` has is uploaded, and recorded once the bucket accepts it,
/// also when a later upload fails. An object the local store does not have was named by an
/// earlier publication or record, and those are written only once their objects are in the
/// bucket. Running the upload again sends only what is still unrecorded.
pub fn store_objects(
    objects: &Objects<'_>,
    s3: &S3,
    referred: Vec<Object>,
) -> Result<Stored, PublishError> {
    let referred: BTreeMap<Key, (u64, String)> = referred
        .into_iter()
        .map(|object| (object.key, (object.size, object.media_type)))
        .collect();
    let referred: Vec<_> = referred.into_iter().collect();
    let unrecorded = unrecorded(objects, &referred)?;
    let confirmed = Mutex::new(Vec::with_capacity(unrecorded.len()));
    let uploading = put_objects(objects.local, s3, &unrecorded, &confirmed);
    let confirmed = confirmed.into_inner().unwrap_or_else(|e| e.into_inner());
    objects
        .held
        .hold(&confirmed)
        .map_err(PublishError::wrap("holdings"))?;
    uploading?;
    Ok(Stored {
        uploaded: unrecorded.len(),
        present: referred.len() - unrecorded.len(),
    })
}

/// Makes `pointer` the current publication of `target` in the bucket of `s3`: writes the pointer
/// of its version, then the current pointer of `target`, unless that is `pointer` already. Call it
/// once every object of the publication is in the bucket. Returns whether the pointers were
/// written.
pub fn make_current(s3: &S3, target: &str, pointer: &Pointer) -> Result<bool, PublishError> {
    check_version(&pointer.published_version)?;
    let bytes = serde_json::to_vec(pointer).map_err(|e| PublishError::new("pointer", e))?;
    let current = s3.pointer(target);
    let remote = s3
        .get(&current)
        .map_err(|e| PublishError::new("pointer", e))?;
    if remote.as_deref() == Some(bytes.as_slice()) {
        return Ok(false);
    }
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    let sha256 = hex(&digest);
    let version = s3.version_pointer(target, &pointer.published_version);
    for name in [&version, &current] {
        s3.put(
            name,
            &sha256,
            "application/json",
            POINTER_CACHE_CONTROL,
            || Ok(bytes.as_slice()),
        )
        .map_err(|e| PublishError::new("pointer", e))?;
    }
    Ok(true)
}

/// An object a publication refers to: its key, its size and its media type.
type Referred = (Key, (u64, String));

/// The objects of `referred`, in key order, that the holdings of `objects` do not record with
/// their size and that its local store has. A local file of another size is refused.
fn unrecorded<'a>(
    objects: &Objects<'_>,
    referred: &'a [Referred],
) -> Result<Vec<&'a Referred>, PublishError> {
    let mut unknown = Vec::new();
    for directory in referred.chunk_by(|(a, _), (b, _)| a.sha256[0] == b.sha256[0]) {
        let held = objects.held.sizes(directory[0].0.sha256[0]);
        let held = held.map_err(PublishError::wrap("holdings"))?;
        for referred @ (key, (size, _)) in directory {
            if held.get(key) == Some(size) {
                continue;
            }
            let local = objects.local.size(key);
            match local.map_err(PublishError::wrap("files"))? {
                Some(local) if local == *size => unknown.push(referred),
                Some(local) => {
                    return Err(PublishError::new(
                        "verify",
                        format!("{key} has {local} bytes in the store, not {size}"),
                    ));
                }
                None if objects.held.complete() => {
                    return Err(PublishError::new(
                        "verify",
                        format!("{key} is in neither the bucket nor the local store"),
                    ));
                }
                None => {}
            }
        }
    }
    Ok(unknown)
}

/// Uploads each of `unrecorded` from `local`, adding every object the bucket then holds to
/// `confirmed`. The bucket is not asked first: an object is named by its bytes, so writing it
/// again changes nothing.
fn put_objects(
    local: &LocalStore,
    s3: &S3,
    unrecorded: &[&Referred],
    confirmed: &Mutex<Vec<(Key, u64)>>,
) -> Result<(), PublishError> {
    each(unrecorded.len(), s3.bucket.connections, |i| {
        let (key, (size, media_type)) = unrecorded[i];
        s3.put_object(key, *size, media_type, &local.path(key))
            .map_err(|message| PublishError::new("files", message))?;
        confirmed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((key.clone(), *size));
        Ok(())
    })
}

/// A reader of the bytes of an object, which fails at their end when they do not have the SHA-256
/// the object is named by.
struct Named<R> {
    source: R,
    /// The hash of the bytes read so far, until the end has been checked.
    hash: Option<Sha256>,
    sha256: [u8; 32],
}

impl<R> Named<R> {
    fn new(source: R, sha256: [u8; 32]) -> Self {
        Self {
            source,
            hash: Some(Sha256::new()),
            sha256,
        }
    }
}

impl<R: Read> Read for Named<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.source.read(buffer)?;
        if read > 0 {
            if let Some(hash) = &mut self.hash {
                hash.update(&buffer[..read]);
            }
        } else if !buffer.is_empty()
            && let Some(hash) = self.hash.take()
            && hash.finalize()[..] != self.sha256
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the bytes do not have the SHA-256 {}", hex(&self.sha256)),
            ));
        }
        Ok(read)
    }
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
    /// The files on the page.
    files: Vec<Listed>,
    /// The continuation token of the next page, when the listing goes on.
    next: Option<String>,
}

impl Page {
    /// Reads the XML of a `ListObjectsV2` response. Any other response, such as the list of
    /// buckets that a request outside every bucket gets, or one cut short, is an error rather
    /// than a page with fewer names.
    fn parse(xml: &str) -> Result<Self, String> {
        let complete =
            xml.contains("<ListBucketResult") && xml.trim_end().ends_with("</ListBucketResult>");
        if !complete {
            return Err("the response is not a whole listing of objects".to_owned());
        }
        let files = elements(xml, "Contents")
            .map(|contents| {
                let name = element(contents, "Key").ok_or("an object has no Key")?;
                let size = element(contents, "Size")
                    .and_then(|size| size.parse().ok())
                    .ok_or("an object has no Size")?;
                let modified = element(contents, "LastModified")
                    .and_then(Utc::parse)
                    .ok_or("an object has no LastModified")?;
                Ok(Listed {
                    name: unescape(name),
                    size,
                    modified: modified.time(),
                })
            })
            .collect::<Result<_, &str>>()?;
        let next = match element(xml, "IsTruncated") {
            Some("true") => Some(unescape(
                element(xml, "NextContinuationToken").ok_or("a truncated page has no token")?,
            )),
            _ => None,
        };
        Ok(Self { files, next })
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

/// `text` with `&`, `<` and `>` as XML entities.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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
    let Utc {
        year,
        month,
        day,
        hour,
        minute,
        second,
    } = Utc::of(now);
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
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
    use std::time::UNIX_EPOCH;

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
            <Contents><Key>p/store/ab/a&amp;b.json</Key>\
            <LastModified>1970-01-01T00:00:01.000Z</LastModified><Size>12</Size></Contents>\
            <Contents><Key>p/store/ab/c.png</Key>\
            <LastModified>1970-01-01T00:01:00Z</LastModified><Size>3</Size></Contents>\
            <NextContinuationToken>t+1/=</NextContinuationToken></ListBucketResult>";
        let file = |name: &str, size, seconds| Listed {
            name: name.to_owned(),
            size,
            modified: UNIX_EPOCH + Duration::from_secs(seconds),
        };
        assert_eq!(
            Page::parse(xml),
            Ok(Page {
                files: vec![
                    file("p/store/ab/a&b.json", 12, 1),
                    file("p/store/ab/c.png", 3, 60)
                ],
                next: Some("t+1/=".to_owned()),
            })
        );
        let last = "<ListBucketResult><IsTruncated>false</IsTruncated></ListBucketResult>";
        assert_eq!(
            Page::parse(last),
            Ok(Page {
                files: Vec::new(),
                next: None
            })
        );
        assert!(Page::parse("<Contents><Key>x</Key></Contents>").is_err());
        let undated = "<ListBucketResult><Contents><Key>x</Key><Size>1</Size></Contents>\
            </ListBucketResult>";
        assert!(Page::parse(undated).is_err());
        let buckets = "<ListAllMyBucketsResult><Buckets></Buckets></ListAllMyBucketsResult>";
        assert!(Page::parse(buckets).is_err());
        assert!(Page::parse(&xml[..xml.len() - 20]).is_err());
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
            listings: 1,
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
