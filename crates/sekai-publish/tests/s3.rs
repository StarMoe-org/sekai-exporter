//! Publishing to an S3-compatible bucket, against a local server that checks signatures.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sekai_catalog::{Key, LocalStore};
use sekai_format::asset::{FORMAT, VERSION};
use sekai_format::{AssetDocument, MemberFile};
use sekai_publish::s3::{Bucket, Check, Credentials, S3, publish, publish_with};
use sekai_publish::{Binding, Layout, Plan, documents, plan};
use sha2::{Digest, Sha256};

const ACCESS_KEY: &str = "TESTACCESSKEY";
const SECRET_KEY: &str = "test-secret-key";
const REGION: &str = "test-region";

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

/// A request as the server received it.
struct Request {
    method: String,
    path: String,
    /// The query parameters, decoded, in name order.
    query: BTreeMap<String, String>,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

/// `text` with every `%XX` replaced by its byte.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            out.push(u8::from_str_radix(&text[i + 1..i + 3], 16).unwrap());
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
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

impl Request {
    /// Whether the request carries a valid Signature Version 4 signature for `secret`.
    fn signed(&self, secret: &str) -> bool {
        let Some(authorization) = self.headers.get("authorization") else {
            return false;
        };
        let fields: BTreeMap<&str, &str> = authorization
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
        let payload = self.headers["x-amz-content-sha256"].as_str();
        let query: Vec<String> = self
            .query
            .iter()
            .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
            .collect();
        let mut canonical = format!("{}\n{}\n{}\n", self.method, self.path, query.join("&"));
        for name in signed.split(';') {
            canonical.push_str(&format!("{name}:{}\n", self.headers[name].trim()));
        }
        canonical.push_str(&format!("\n{signed}\n{payload}"));
        let time = &self.headers["x-amz-date"];
        let to_sign = format!(
            "AWS4-HMAC-SHA256\n{time}\n{}\n{}",
            scope[1..].join("/"),
            sha256(canonical.as_bytes())
        );
        let mut key = hmac(format!("AWS4{secret}").as_bytes(), scope[1].as_bytes());
        for part in &scope[2..] {
            key = hmac(&key, part.as_bytes());
        }
        hex(&hmac(&key, to_sign.as_bytes())) == *signature
    }
}

/// A bucket served on a local port.
#[derive(Default)]
struct State {
    objects: Mutex<BTreeMap<String, Vec<u8>>>,
    cache_controls: Mutex<BTreeMap<String, String>>,
    heads: AtomicUsize,
    puts: AtomicUsize,
    lists: AtomicUsize,
    /// Uploads of objects other than pointers are refused once this many have been accepted.
    refuse_after: Mutex<Option<usize>>,
}

struct Server {
    state: Arc<State>,
    endpoint: String,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/bucket", listener.local_addr().unwrap());
        let state = Arc::new(State::default());
        let shared = Arc::clone(&state);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let state = Arc::clone(&shared);
                std::thread::spawn(move || serve(&state, stream.unwrap()));
            }
        });
        Self { state, endpoint }
    }

    fn client(&self, secret: &str) -> S3 {
        S3::new(
            Bucket {
                endpoint: self.endpoint.clone(),
                region: REGION.to_owned(),
                prefix: "out/".to_owned(),
                connections: 3,
            },
            Credentials {
                access_key_id: ACCESS_KEY.to_owned(),
                secret_access_key: secret.to_owned(),
            },
        )
        .unwrap()
    }

    fn object(&self, name: &str) -> Option<Vec<u8>> {
        self.state.objects.lock().unwrap().get(name).cloned()
    }

    fn refuse_after(&self, uploads: Option<usize>) {
        *self.state.refuse_after.lock().unwrap() = uploads;
    }

    fn remove(&self, name: &str) {
        self.state.objects.lock().unwrap().remove(name);
    }
}

/// The names in a listing page.
const PAGE: usize = 2;

/// A `ListObjectsV2` page of the names under `prefix` after the continuation token `after`.
fn list(state: &State, prefix: &str, after: Option<&String>) -> Vec<u8> {
    let objects = state.objects.lock().unwrap();
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

fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let path = path.to_owned();
    let query = query
        .split('&')
        .filter(|parameter| !parameter.is_empty())
        .filter_map(|parameter| parameter.split_once('='))
        .map(|(name, value)| (decode(name), decode(value)))
        .collect();
    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let length: usize = headers
        .get("content-length")
        .map_or(0, |value| value.parse().unwrap());
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        query,
        headers,
        body,
    })
}

fn serve(state: &State, mut stream: TcpStream) {
    let Some(request) = read_request(&stream) else {
        return;
    };
    let name = request
        .path
        .strip_prefix("/bucket/")
        .unwrap_or_default()
        .to_owned();
    let (status, body) = if !request.signed(SECRET_KEY) {
        (403, Vec::new())
    } else {
        match request.method.as_str() {
            "HEAD" => {
                state.heads.fetch_add(1, Ordering::SeqCst);
                match state.objects.lock().unwrap().get(&name) {
                    Some(bytes) => (200, bytes.clone()),
                    None => (404, Vec::new()),
                }
            }
            "GET" if request.query.get("list-type").is_some_and(|t| t == "2") => {
                state.lists.fetch_add(1, Ordering::SeqCst);
                let prefix = request.query.get("prefix").cloned().unwrap_or_default();
                (
                    200,
                    list(state, &prefix, request.query.get("continuation-token")),
                )
            }
            "GET" => match state.objects.lock().unwrap().get(&name) {
                Some(bytes) => (200, bytes.clone()),
                None => (404, Vec::new()),
            },
            "PUT" => {
                let refused = !name.ends_with("current.json")
                    && state
                        .refuse_after
                        .lock()
                        .unwrap()
                        .is_some_and(|n| state.puts.load(Ordering::SeqCst) >= n);
                if refused {
                    (403, Vec::new())
                } else if request.headers["x-amz-content-sha256"] != sha256(&request.body) {
                    (400, Vec::new())
                } else {
                    state.puts.fetch_add(1, Ordering::SeqCst);
                    if let Some(cc) = request.headers.get("cache-control") {
                        state
                            .cache_controls
                            .lock()
                            .unwrap()
                            .insert(name.clone(), cc.clone());
                    }
                    state.objects.lock().unwrap().insert(name, request.body);
                    (200, Vec::new())
                }
            }
            _ => (405, Vec::new()),
        }
    };
    let mut response = format!(
        "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    if request.method != "HEAD" {
        response.extend_from_slice(&body);
    }
    let _ = stream.write_all(&response);
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sekai-s3-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn store(&self) -> LocalStore {
        LocalStore::open(self.0.join("store")).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn asset(store: &LocalStore, name: &str, members: &[(&str, &[u8])]) -> Binding {
    let members = members
        .iter()
        .map(|(role, bytes)| {
            let key = store.put(bytes, "webp").unwrap();
            MemberFile {
                role: (*role).to_owned(),
                sha256: hex(&key.sha256),
                size: bytes.len() as u64,
                extension: "webp".to_owned(),
                media_type: "image/webp".to_owned(),
            }
        })
        .collect();
    let document = AssetDocument {
        format: FORMAT.to_owned(),
        version: VERSION,
        name: name.to_owned(),
        form: "texture".to_owned(),
        form_version: 1,
        fingerprint: "ab".to_owned(),
        sources: Vec::new(),
        members,
        resources: Vec::new(),
        references: Vec::new(),
        error: None,
    };
    Binding {
        name: name.to_owned(),
        document: store.put(&document.to_bytes().unwrap(), "json").unwrap(),
    }
}

fn sample(store: &LocalStore, version: &str, extra: &[u8]) -> Plan {
    let bindings = [
        asset(store, "a/one", &[("image", b"one")]),
        asset(store, "a/two", &[("base", b"two"), ("mask", extra)]),
        asset(store, "b/three", &[("image", b"three")]),
    ];
    plan(version, &bindings, store, 2).unwrap()
}

fn object_name(key: &Key) -> String {
    format!("out/store/{}", key.relative_path())
}

const POINTER: &str = "out/published/jp/canonical/current.json";

#[test]
fn a_second_publication_uploads_nothing() {
    let s = Scratch::new("twice");
    let store = s.store();
    let server = Server::start();
    let s3 = server.client(SECRET_KEY);
    let plan = sample(&store, "v1", b"mask");

    let (pointer, report) = publish(&store, &s3, "jp/canonical", &plan, 2).unwrap();
    // Seven files, three shards, one listing and the root, and a path index of one chunk.
    assert_eq!((report.uploaded, report.present), (14, 0));
    assert!(report.pointer_written);
    for file in &plan.files {
        assert_eq!(
            server.object(&object_name(&file.key)),
            Some(store.read(&file.key).unwrap()),
            "{}",
            file.path
        );
    }
    assert_eq!(
        server.object(POINTER),
        Some(serde_json::to_vec(&pointer).unwrap())
    );
    let root = Key {
        sha256: sekai_catalog::store::parse_hex(pointer.root.as_deref().unwrap()).unwrap(),
        extension: "json".to_owned(),
    };
    assert!(server.object(&object_name(&root)).is_some());

    let puts = server.state.puts.load(Ordering::SeqCst);
    let (again, report) = publish(&store, &s3, "jp/canonical", &plan, 2).unwrap();
    assert_eq!(again, pointer);
    assert_eq!((report.uploaded, report.present), (0, 14));
    assert!(!report.pointer_written);
    assert_eq!(server.state.puts.load(Ordering::SeqCst), puts);
}

#[test]
fn a_wrong_key_is_refused_and_writes_no_pointer() {
    let s = Scratch::new("signature");
    let store = s.store();
    let server = Server::start();
    let plan = sample(&store, "v1", b"mask");
    let e = publish(&store, &server.client("wrong"), "jp/canonical", &plan, 2).unwrap_err();
    // Reading the live pointer is the first signed request, and its refusal stops the publication.
    assert_eq!(e.stage, "pointer");
    assert!(server.object(POINTER).is_none());
    assert_eq!(server.state.puts.load(Ordering::SeqCst), 0);
}

#[test]
fn an_interrupted_publication_converges() {
    let s = Scratch::new("interrupted");
    let store = s.store();
    let server = Server::start();
    let s3 = server.client(SECRET_KEY);
    let plan = sample(&store, "v1", b"mask");

    server.refuse_after(Some(4));
    let e = publish(&store, &s3, "jp/canonical", &plan, 2).unwrap_err();
    assert_eq!(e.stage, "files");
    assert!(server.object(POINTER).is_none());

    server.refuse_after(None);
    let (pointer, report) = publish(&store, &s3, "jp/canonical", &plan, 2).unwrap();
    assert_eq!(report.uploaded + report.present, 14);
    assert!(report.present >= 4);
    assert_eq!(
        server.object(POINTER),
        Some(serde_json::to_vec(&pointer).unwrap())
    );
}

#[test]
fn a_failed_publication_keeps_the_previous_pointer() {
    let s = Scratch::new("keeps");
    let store = s.store();
    let server = Server::start();
    let s3 = server.client(SECRET_KEY);
    publish(
        &store,
        &s3,
        "jp/canonical",
        &sample(&store, "v1", b"mask"),
        2,
    )
    .unwrap();
    let previous = server.object(POINTER).unwrap();

    server.refuse_after(Some(0));
    let e = publish(
        &store,
        &s3,
        "jp/canonical",
        &sample(&store, "v2", b"new"),
        2,
    )
    .unwrap_err();
    assert_eq!(e.stage, "files");
    assert_eq!(server.object(POINTER), Some(previous));
}

#[test]
fn an_object_missing_from_the_store_stops_before_the_pointer() {
    let s = Scratch::new("missing");
    let store = s.store();
    let server = Server::start();
    let plan = sample(&store, "v1", b"mask");
    let file = plan
        .files
        .iter()
        .find(|f| f.path == "a/two/mask.webp")
        .unwrap();
    std::fs::remove_file(store.path(&file.key)).unwrap();
    let e = publish(&store, &server.client(SECRET_KEY), "jp/canonical", &plan, 2).unwrap_err();
    assert_eq!(e.stage, "verify");
    assert!(server.object(POINTER).is_none());
}

#[test]
fn incremental_publication_skips_head_for_live_objects() {
    let s = Scratch::new("incremental");
    let store = s.store();
    let server = Server::start();
    let s3 = server.client(SECRET_KEY);

    publish(
        &store,
        &s3,
        "jp/canonical",
        &sample(&store, "v1", b"mask"),
        2,
    )
    .unwrap();
    server.state.heads.store(0, Ordering::SeqCst);

    // Identical publication sends 0 HEADs and 0 PUTs.
    publish(
        &store,
        &s3,
        "jp/canonical",
        &sample(&store, "v1", b"mask"),
        2,
    )
    .unwrap();
    assert_eq!(server.state.heads.load(Ordering::SeqCst), 0);

    // Changing one member checks only the 8 changed objects (1 member, 1 document, the shards "a"
    // and "a/two", 1 listing, the root, and the chunk and root of the path index), skipping HEAD
    // on the 6 unchanged objects of v1.
    let (_, report) = publish(
        &store,
        &s3,
        "jp/canonical",
        &sample(&store, "v2", b"mask2"),
        2,
    )
    .unwrap();
    assert_eq!((report.uploaded, report.present), (8, 6));
    assert_eq!(server.state.heads.load(Ordering::SeqCst), 8);
}

#[test]
fn verifying_lists_the_store_and_restores_lost_objects() {
    let s = Scratch::new("verify");
    let store = s.store();
    let server = Server::start();
    let s3 = server.client(SECRET_KEY);
    let plan = sample(&store, "v1", b"mask");
    publish(&store, &s3, "jp/canonical", &plan, 2).unwrap();
    let lost = object_name(&plan.files[0].key);
    server.remove(&lost);

    // The pointer names the publication, so a plain publication trusts it.
    let (_, report) = publish(&store, &s3, "jp/canonical", &plan, 2).unwrap();
    assert_eq!(report.uploaded, 0);
    assert!(server.object(&lost).is_none());

    server.state.heads.store(0, Ordering::SeqCst);
    let (_, report) = publish_with(
        &store,
        &s3,
        "jp/canonical",
        &plan,
        Layout::Index,
        Check::Listing,
        2,
    )
    .unwrap();
    assert_eq!((report.uploaded, report.present), (1, 13));
    assert!(!report.pointer_written);
    assert_eq!(
        server.object(&lost),
        Some(store.read(&plan.files[0].key).unwrap())
    );
    assert_eq!(server.state.heads.load(Ordering::SeqCst), 0);
    // One request for each empty store directory, and more where a directory fills pages.
    assert!(server.state.lists.load(Ordering::SeqCst) >= 256);
}

#[test]
fn sqlite_layout_sets_cache_control_and_asks_only_about_new_objects() {
    let s = Scratch::new("sqlite-s3");
    let store = s.store();
    let server = Server::start();
    let s3 = server.client(SECRET_KEY);

    let l2d_members: Vec<MemberFile> = [
        (
            "model",
            "json",
            "application/json",
            br#"{"v":3}"#.as_slice(),
        ),
        (
            "moc3",
            "moc3",
            "application/octet-stream",
            b"MOC3".as_slice(),
        ),
    ]
    .into_iter()
    .map(|(role, ext, mime, bytes)| {
        let key = store.put(bytes, ext).unwrap();
        MemberFile {
            role: role.to_owned(),
            sha256: hex(&key.sha256),
            size: bytes.len() as u64,
            extension: ext.to_owned(),
            media_type: mime.to_owned(),
        }
    })
    .collect();
    let l2d_doc = AssetDocument {
        format: FORMAT.to_owned(),
        version: VERSION,
        name: "chara/miku".to_owned(),
        form: "live2d-model".to_owned(),
        form_version: 2,
        fingerprint: "aa".to_owned(),
        sources: Vec::new(),
        members: l2d_members,
        resources: Vec::new(),
        references: Vec::new(),
        error: None,
    };
    let l2d_binding = Binding {
        name: "chara/miku".to_owned(),
        document: store.put(&l2d_doc.to_bytes().unwrap(), "json").unwrap(),
    };
    let tex_binding = asset(&store, "tex/bg", &[("image", b"bg-webp")]);

    let objects = |plan: &Plan| -> BTreeSet<Key> {
        let publication = documents(&store, "jp/canonical", plan, Layout::Sqlite, 2).unwrap();
        publication.objects.into_iter().map(|o| o.key).collect()
    };
    let v1 = plan("v1", &[l2d_binding.clone(), tex_binding.clone()], &store, 2).unwrap();
    let (pointer, report) = publish_with(
        &store,
        &s3,
        "jp/canonical",
        &v1,
        Layout::Sqlite,
        Check::Pointer,
        2,
    )
    .unwrap();
    assert_eq!((report.uploaded, report.present), (objects(&v1).len(), 0));
    assert!(pointer.sqlite.is_some() && pointer.paths.is_some());

    let sqlite_key = Key {
        sha256: sekai_catalog::store::parse_hex(pointer.sqlite.as_deref().unwrap()).unwrap(),
        extension: "sqlite".to_owned(),
    };
    let sqlite_name = object_name(&sqlite_key);
    assert!(server.object(&sqlite_name).is_some());
    let ccs = server.state.cache_controls.lock().unwrap().clone();
    assert_eq!(
        ccs.get(&sqlite_name).map(String::as_str),
        Some("public, max-age=31536000, immutable")
    );
    assert_eq!(ccs.get(POINTER).map(String::as_str), Some("no-cache"));

    // A second version asks only about the objects the first did not have.
    server.state.heads.store(0, Ordering::SeqCst);
    let extra = asset(&store, "tex/fg", &[("image", b"fg-webp")]);
    let v2 = plan("v2", &[l2d_binding, tex_binding, extra], &store, 2).unwrap();
    let (_, report) = publish_with(
        &store,
        &s3,
        "jp/canonical",
        &v2,
        Layout::Sqlite,
        Check::Pointer,
        2,
    )
    .unwrap();
    let all = objects(&v2);
    let new = all.difference(&objects(&v1)).count();
    assert_eq!((report.uploaded, report.present), (new, all.len() - new));
    assert_eq!(server.state.heads.load(Ordering::SeqCst), new);
}
