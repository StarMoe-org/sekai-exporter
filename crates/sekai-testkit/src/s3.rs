//! An S3-compatible bucket in memory, which checks the Signature Version 4 of every request and
//! the SHA-256 of every upload, and logs what it answered.
//!
//! The bucket is served at `http://<host>:<port>/bucket`, path-style, for the access key
//! [`ACCESS_KEY`] with the secret [`SECRET_KEY`], signed for [`REGION`]. It knows `HEAD`, `GET`,
//! `ListObjectsV2` in pages of two names, `PUT` and multipart uploads.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use sha2::{Digest, Sha256};

use crate::http::{self, Request, Response};

/// The access key ID the bucket accepts.
pub const ACCESS_KEY: &str = "TESTACCESSKEY";
/// The secret access key the bucket accepts.
pub const SECRET_KEY: &str = "test-secret-key";
/// The region requests are signed for.
pub const REGION: &str = "test-region";

/// The names of the secrets that hold [`ACCESS_KEY`] and [`SECRET_KEY`].
const SECRETS: [&str; 2] = ["SEKAI_TESTKIT_S3_ACCESS_KEY", "SEKAI_TESTKIT_S3_SECRET_KEY"];

/// The names in a listing page.
const PAGE: usize = 2;

/// What a request asked of the bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// `HEAD` of an object.
    Head,
    /// `GET` of an object.
    Get,
    /// A page of `ListObjectsV2`.
    List,
    /// `PUT` of an object.
    Put,
    /// The start of a multipart upload.
    StartUpload,
    /// A part of a multipart upload.
    PutPart,
    /// The completion of a multipart upload.
    CompleteUpload,
    /// The abort of a multipart upload.
    AbortUpload,
}

impl Operation {
    fn of(request: &Request) -> Option<Self> {
        let upload = request.query.contains_key("uploadId");
        let listing = request.query.get("list-type").is_some_and(|t| t == "2");
        Some(match (request.method.as_str(), upload) {
            ("PUT", true) => Self::PutPart,
            ("POST", true) => Self::CompleteUpload,
            ("DELETE", true) => Self::AbortUpload,
            ("POST", false) if request.query.contains_key("uploads") => Self::StartUpload,
            ("HEAD", false) => Self::Head,
            ("GET", false) if listing => Self::List,
            ("GET", false) => Self::Get,
            ("PUT", false) => Self::Put,
            _ => return None,
        })
    }
}

/// A request the bucket answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    /// What the request asked.
    pub operation: Operation,
    /// The name of the object, without the bucket.
    pub name: String,
    /// The status of the response.
    pub status: u16,
    /// The bytes of the request body.
    pub received: usize,
    /// The bytes of the response body.
    pub sent: usize,
}

/// A multipart upload in progress.
struct Upload {
    name: String,
    cache_control: Option<String>,
    /// The bytes of each part by number, with the entity tag the server gave it.
    parts: BTreeMap<usize, (String, Vec<u8>)>,
}

#[derive(Default)]
struct State {
    objects: Mutex<BTreeMap<String, Vec<u8>>>,
    cache_controls: Mutex<BTreeMap<String, String>>,
    /// The uploads and parts accepted.
    accepted: AtomicUsize,
    /// Uploads of objects other than pointers are refused once this many have been accepted.
    refuse_after: Mutex<Option<usize>>,
    /// How many of the next answers to `GET` of an object end halfway through the object.
    cut_gets: AtomicUsize,
    /// The multipart uploads in progress, by upload ID.
    uploads: Mutex<BTreeMap<String, Upload>>,
    log: Mutex<Vec<Logged>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A bucket served on a local port.
pub struct Server {
    state: Arc<State>,
    endpoint: String,
}

impl Server {
    /// Starts an empty bucket.
    pub fn start() -> Self {
        let state = Arc::new(State::default());
        let shared = Arc::clone(&state);
        let address = http::listen(move |request| shared.answer(request));
        Self {
            state,
            endpoint: format!("http://{address}/bucket"),
        }
    }

    /// The address of the bucket, path-style.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The `publish.s3` section of an exporter configuration that names the bucket under
    /// `prefix`, with the secrets that [`Server::write_secrets`] writes.
    pub fn section(&self, prefix: &str) -> serde_json::Value {
        serde_json::json!({
            "endpoint": self.endpoint,
            "region": REGION,
            "prefix": prefix,
            "connections": 2,
            "secrets": {"access_key_id": SECRETS[0], "secret_access_key": SECRETS[1]},
        })
    }

    /// Writes the secrets of [`Server::section`] as files in `directory`.
    pub fn write_secrets(directory: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(directory)?;
        std::fs::write(directory.join(SECRETS[0]), ACCESS_KEY)?;
        std::fs::write(directory.join(SECRETS[1]), SECRET_KEY)
    }

    /// The bytes of the object `name`, if the bucket holds it.
    pub fn object(&self, name: &str) -> Option<Vec<u8>> {
        lock(&self.state.objects).get(name).cloned()
    }

    /// The names of the objects that start with `prefix`, in order.
    pub fn names(&self, prefix: &str) -> Vec<String> {
        let objects = lock(&self.state.objects);
        let names = objects.keys().filter(|name| name.starts_with(prefix));
        names.cloned().collect()
    }

    /// Holds `bytes` as the object `name`, as if another client had uploaded them.
    pub fn insert(&self, name: &str, bytes: Vec<u8>) {
        lock(&self.state.objects).insert(name.to_owned(), bytes);
    }

    /// Loses the object `name`.
    pub fn remove(&self, name: &str) {
        lock(&self.state.objects).remove(name);
    }

    /// Refuses uploads of objects other than pointers once `uploads` have been accepted, or
    /// accepts all of them again.
    pub fn refuse_after(&self, uploads: Option<usize>) {
        *lock(&self.state.refuse_after) = uploads;
    }

    /// Ends the next `gets` answers to `GET` of an object halfway through the object, as when the
    /// connection drops.
    pub fn cut_gets(&self, gets: usize) {
        self.state.cut_gets.store(gets, Ordering::SeqCst);
    }

    /// The `Cache-Control` the object `name` was uploaded with.
    pub fn cache_control(&self, name: &str) -> Option<String> {
        lock(&self.state.cache_controls).get(name).cloned()
    }

    /// How many multipart uploads were started and neither completed nor aborted.
    pub fn uploads_in_progress(&self) -> usize {
        lock(&self.state.uploads).len()
    }

    /// Every request the bucket answered, in order.
    pub fn requests(&self) -> Vec<Logged> {
        lock(&self.state.log).clone()
    }

    /// How many requests of `operation` the bucket answered with success.
    pub fn count(&self, operation: Operation) -> usize {
        let log = lock(&self.state.log);
        let succeeded = log.iter().filter(|logged| logged.status < 300);
        succeeded
            .filter(|logged| logged.operation == operation)
            .count()
    }
}

impl State {
    fn answer(&self, request: &Request) -> Response {
        let name = request.path.strip_prefix("/bucket/").unwrap_or_default();
        let operation = Operation::of(request);
        let response = match operation {
            _ if !signed(request, SECRET_KEY) => Response::new(403, ""),
            None => Response::new(405, ""),
            Some(operation) => self.perform(operation, name, request),
        };
        if let Some(operation) = operation {
            lock(&self.log).push(Logged {
                operation,
                name: name.to_owned(),
                status: response.status,
                received: request.body.len(),
                sent: response.body.len(),
            });
        }
        response
    }

    fn perform(&self, operation: Operation, name: &str, request: &Request) -> Response {
        let digest = request.headers.get("x-amz-content-sha256");
        let uploading = matches!(operation, Operation::Put | Operation::PutPart);
        if uploading && self.refuses(name) {
            return Response::new(403, "");
        }
        if uploading && digest.is_none_or(|digest| *digest != sha256(&request.body)) {
            return Response::new(400, "");
        }
        let id = request.query.get("uploadId").map(String::as_str);
        match (operation, id) {
            (Operation::Head | Operation::Get, _) => match lock(&self.objects).get(name) {
                Some(bytes) => Response {
                    cut: operation == Operation::Get && self.cuts(),
                    ..Response::new(200, bytes.clone())
                },
                None => Response::new(404, ""),
            },
            (Operation::List, _) => {
                let prefix = request.query.get("prefix").map_or("", String::as_str);
                let after = request.query.get("continuation-token");
                Response::new(200, self.list(prefix, after))
            }
            (Operation::Put, _) => {
                self.accepted.fetch_add(1, Ordering::SeqCst);
                self.keep(
                    name.to_owned(),
                    request.body.clone(),
                    cache_control(request),
                );
                Response::new(200, "")
            }
            (Operation::StartUpload, _) => Response::new(200, self.start_upload(name, request)),
            (Operation::PutPart, Some(id)) => match self.put_part(id, request) {
                Some(tag) => Response {
                    headers: format!("etag: {tag}\r\n"),
                    ..Response::new(200, "")
                },
                None => Response::new(404, ""),
            },
            (Operation::CompleteUpload, Some(id)) => match self.complete_upload(id, request) {
                true => Response::new(
                    200,
                    "<CompleteMultipartUploadResult></CompleteMultipartUploadResult>",
                ),
                false => Response::new(200, "<Error><Code>InvalidPart</Code></Error>"),
            },
            (Operation::AbortUpload, Some(id)) => {
                lock(&self.uploads).remove(id);
                Response::new(204, "")
            }
            _ => Response::new(400, ""),
        }
    }

    /// Whether this answer to `GET` of an object is cut off, which uses up one of the cuts.
    fn cuts(&self) -> bool {
        self.cut_gets
            .try_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                left.checked_sub(1)
            })
            .is_ok()
    }

    /// Whether an upload to `name` is refused.
    fn refuses(&self, name: &str) -> bool {
        let limit = *lock(&self.refuse_after);
        !name.ends_with("current.json")
            && limit.is_some_and(|n| self.accepted.load(Ordering::SeqCst) >= n)
    }

    fn keep(&self, name: String, bytes: Vec<u8>, cache_control: Option<String>) {
        if let Some(cache_control) = cache_control {
            lock(&self.cache_controls).insert(name.clone(), cache_control);
        }
        lock(&self.objects).insert(name, bytes);
    }

    /// A `ListObjectsV2` page of the names under `prefix` after the continuation token `after`.
    fn list(&self, prefix: &str, after: Option<&String>) -> Vec<u8> {
        let objects = lock(&self.objects);
        let mut names = objects
            .range(prefix.to_owned()..)
            .take_while(|(name, _)| name.starts_with(prefix))
            .filter(|(name, _)| after.is_none_or(|after| *name > after));
        let page: Vec<_> = names.by_ref().take(PAGE).collect();
        let mut xml = String::from("<ListBucketResult>");
        for (name, bytes) in &page {
            xml.push_str(&format!(
                "<Contents><Key>{}</Key><Size>{}</Size></Contents>",
                name.replace('&', "&amp;"),
                bytes.len()
            ));
        }
        match (names.next(), page.last()) {
            (Some(_), Some((last, _))) => xml.push_str(&format!(
                "<IsTruncated>true</IsTruncated><NextContinuationToken>{last}</NextContinuationToken>"
            )),
            _ => xml.push_str("<IsTruncated>false</IsTruncated>"),
        }
        xml.push_str("</ListBucketResult>");
        xml.into_bytes()
    }

    /// Starts a multipart upload of `request` to `name` and answers with its ID.
    fn start_upload(&self, name: &str, request: &Request) -> Vec<u8> {
        let mut uploads = lock(&self.uploads);
        let id = format!("upload+{}", uploads.len());
        let upload = Upload {
            name: name.to_owned(),
            cache_control: cache_control(request),
            parts: BTreeMap::new(),
        };
        uploads.insert(id.clone(), upload);
        format!("<InitiateMultipartUploadResult><UploadId>{id}</UploadId></InitiateMultipartUploadResult>")
            .into_bytes()
    }

    /// Takes the body of `request` as a part of the upload `id`, and returns its entity tag.
    fn put_part(&self, id: &str, request: &Request) -> Option<String> {
        let number = request.query.get("partNumber")?.parse().ok()?;
        let tag = format!("\"{}\"", &sha256(&request.body)[..32]);
        let mut uploads = lock(&self.uploads);
        let upload = uploads.get_mut(id)?;
        upload
            .parts
            .insert(number, (tag.clone(), request.body.clone()));
        self.accepted.fetch_add(1, Ordering::SeqCst);
        Some(tag)
    }

    /// Joins the parts of the upload `id` that `request` lists into its object, when every part
    /// is listed in order with its entity tag.
    fn complete_upload(&self, id: &str, request: &Request) -> bool {
        let listed = String::from_utf8_lossy(&request.body).into_owned();
        let Some(upload) = lock(&self.uploads).remove(id) else {
            return false;
        };
        let tags: Vec<&str> = listed
            .split("<ETag>")
            .skip(1)
            .filter_map(|rest| rest.split("</ETag>").next())
            .collect();
        let held: Vec<&str> = upload.parts.values().map(|(tag, _)| tag.as_str()).collect();
        if tags != held {
            return false;
        }
        let bytes = upload
            .parts
            .into_values()
            .flat_map(|(_, bytes)| bytes)
            .collect();
        self.keep(upload.name, bytes, upload.cache_control);
        true
    }
}

fn cache_control(request: &Request) -> Option<String> {
    request.headers.get("cache-control").cloned()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn sha256(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    hex(&digest)
}

fn hmac(key: &[u8], message: &[u8]) -> Vec<u8> {
    let mut block = [0u8; 64];
    block[..key.len()].copy_from_slice(key);
    let inner: [u8; 32] = Sha256::new()
        .chain_update(block.map(|b| b ^ 0x36))
        .chain_update(message)
        .finalize()
        .into();
    let outer: [u8; 32] = Sha256::new()
        .chain_update(block.map(|b| b ^ 0x5c))
        .chain_update(inner)
        .finalize()
        .into();
    outer.to_vec()
}

/// Every byte of `text` except unreserved characters as `%XX`.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Whether `request` carries a valid Signature Version 4 signature for `secret`.
fn signed(request: &Request, secret: &str) -> bool {
    let header = |name: &str| request.headers.get(name).map_or("", String::as_str);
    let fields: BTreeMap<&str, &str> = header("authorization")
        .trim_start_matches("AWS4-HMAC-SHA256 ")
        .split(", ")
        .filter_map(|field| field.split_once('='))
        .collect();
    let (Some(credential), Some(signed), Some(signature)) = (
        fields.get("Credential"),
        fields.get("SignedHeaders"),
        fields.get("Signature"),
    ) else {
        return false;
    };
    let scope: Vec<&str> = credential.split('/').collect();
    if scope.len() != 5 || scope[0] != ACCESS_KEY || scope[2] != REGION {
        return false;
    }
    let query: Vec<String> = request
        .query
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect();
    let mut canonical = format!(
        "{}\n{}\n{}\n",
        request.method,
        request.path,
        query.join("&")
    );
    for name in signed.split(';') {
        canonical.push_str(&format!("{name}:{}\n", header(name).trim()));
    }
    let payload = header("x-amz-content-sha256");
    canonical.push_str(&format!("\n{signed}\n{payload}"));
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{}\n{}",
        header("x-amz-date"),
        scope[1..].join("/"),
        sha256(canonical.as_bytes())
    );
    let mut key = hmac(format!("AWS4{secret}").as_bytes(), scope[1].as_bytes());
    for part in &scope[2..] {
        key = hmac(&key, part.as_bytes());
    }
    hex(&hmac(&key, to_sign.as_bytes())) == *signature
}
