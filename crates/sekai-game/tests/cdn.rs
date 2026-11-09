//! The distribution sources against a local HTTP server.

#![cfg(feature = "cdn")]

use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use aes::Aes128;
use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockModeDecrypt, BlockModeEncrypt, KeyIvInit};
use sekai_format::identity::{AssetIdentity, ClientIdentity, VersionIdentity};
use sekai_game::cdn::{
    BundleEntry, Network, Region, Secrets, Source, TableSource, Tables, create_account, open,
};
use sekai_game::sink::Sink;
use serde_json::json;

const KEY: [u8; 16] = *b"0123456789abcdef";
const IV: [u8; 16] = *b"fedcba9876543210";

struct Request {
    method: String,
    path: String,
    cookie: String,
    body: Vec<u8>,
}

struct Reply {
    status: u16,
    set_cookie: Vec<String>,
    body: Vec<u8>,
}

fn reply(status: u16, body: impl Into<Vec<u8>>) -> Reply {
    Reply {
        status,
        set_cookie: Vec::new(),
        body: body.into(),
    }
}

fn answer(
    mut stream: std::net::TcpStream,
    handler: &(dyn Fn(&Request) -> Reply + Send + Sync),
) -> io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let method = first.next().unwrap_or("").to_owned();
    let path = first.next().unwrap_or("").to_owned();
    let headers: Vec<_> = lines.filter_map(|line| line.split_once(": ")).collect();
    let header = |wanted: &str| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map_or("", |(_, value)| *value)
    };
    let mut body = vec![0; header("content-length").parse().unwrap_or(0)];
    stream.read_exact(&mut body)?;
    let reply = handler(&Request {
        method,
        path,
        cookie: header("cookie").to_owned(),
        body,
    });
    let mut text = format!(
        "HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for value in &reply.set_cookie {
        text.push_str(&format!("Set-Cookie: {value}\r\n"));
    }
    text.push_str("\r\n");
    stream.write_all(text.as_bytes())?;
    stream.write_all(&reply.body)
}

/// Serves `handler` on a local port and returns `host:port`.
fn serve(handler: impl Fn(&Request) -> Reply + Send + Sync + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let handler: Arc<dyn Fn(&Request) -> Reply + Send + Sync> = Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = handler.clone();
            std::thread::spawn(move || {
                let _ = answer(stream, handler.as_ref());
            });
        }
    });
    address
}

fn encrypt(document: &serde_json::Value) -> Vec<u8> {
    let plain = rmp_serde::to_vec(document).unwrap();
    let length = plain.len();
    let mut buffer = plain;
    buffer.resize(length + 16, 0);
    let cipher = cbc::Encryptor::<Aes128>::new_from_slices(&KEY, &IV).unwrap();
    let written = cipher
        .encrypt_padded::<Pkcs7>(&mut buffer, length)
        .unwrap()
        .len();
    buffer.truncate(written);
    buffer
}

fn decrypt(body: &[u8]) -> serde_json::Value {
    let mut buffer = body.to_vec();
    let cipher = cbc::Decryptor::<Aes128>::new_from_slices(&KEY, &IV).unwrap();
    rmp_serde::from_slice(cipher.decrypt_padded::<Pkcs7>(&mut buffer).unwrap()).unwrap()
}

struct SecretFiles(PathBuf);

impl SecretFiles {
    fn new(name: &str) -> Self {
        let directory =
            std::env::temp_dir().join(format!("sekai-cdn-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        std::fs::write(directory.join("TEST_KEY"), hex(&KEY)).unwrap();
        std::fs::write(directory.join("TEST_IV"), hex(&IV)).unwrap();
        Self(directory)
    }

    fn secrets(&self) -> Secrets {
        Secrets::new(Some(self.0.clone()))
    }
}

impl Drop for SecretFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn source(name: &str, region: serde_json::Value) -> (Box<dyn Source>, SecretFiles) {
    let files = SecretFiles::new(name);
    let config = json!({ "regions": { "r": region } }).to_string();
    let region = Region::load("r", &config).unwrap();
    (open(region, &files.secrets()).unwrap(), files)
}

#[derive(Default)]
struct Buffer {
    bytes: Vec<u8>,
    resets: usize,
}

impl Write for Buffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Sink for Buffer {
    fn reset(&mut self) -> io::Result<()> {
        self.bytes.clear();
        self.resets += 1;
        Ok(())
    }
}

fn cp_region(base: &str, client: serde_json::Value, extra: serde_json::Value) -> serde_json::Value {
    let mut region = json!({
        "kind": "cp",
        "platform": "android",
        "urls": {
            "signature": format!("http://{base}/signature"),
            "version": format!("http://{base}/version/{{application_version}}/{{application_hash}}"),
            "manifest": format!("http://{base}/info/{{profile}}/{{host_hash}}/{{asset_version}}/{{asset_hash}}/{{platform}}"),
            "bundle": format!("http://{base}/bundle/{{asset_version}}/{{platform}}/{{bundle}}"),
        },
        "client": client,
        "secrets": {"key": "TEST_KEY", "iv": "TEST_IV"},
        "network": {"retries": 2, "back_off_ms": 1},
    });
    for (name, value) in extra.as_object().unwrap() {
        region[name] = value.clone();
    }
    region
}

fn cp_client(version: &str) -> serde_json::Value {
    json!({
        "application_version": version,
        "application_hash": "app",
        "asset_version": "10",
        "asset_hash": "abc",
        "unity_version": "u",
        "user_agent": "agent",
        "device_model": "d",
        "operating_system": "Android",
    })
}

/// The answer of a `cp` service that knows the application versions in `known`.
fn cp_answer(known: &[&str], request: &Request) -> Reply {
    let path = request.path.as_str();
    if path == "/signature" && request.method == "POST" {
        return Reply {
            status: 200,
            set_cookie: vec!["CloudFront-Policy=p; Path=/".to_owned()],
            body: Vec::new(),
        };
    }
    if request.cookie != "CloudFront-Policy=p" {
        return reply(403, "");
    }
    if let Some(rest) = path.strip_prefix("/version/") {
        let version = rest.split('/').next().unwrap_or("");
        if known.contains(&version) {
            let document = json!({"profile": "prod", "assetbundleHostHash": "hh", "domain": "x"});
            return reply(200, encrypt(&document));
        }
        return reply(404, "");
    }
    if path == "/info/prod/hh/10/abc/android" {
        let document = json!({"version": "10", "bundles": {
            "b/two": {"bundleName": "b/two", "hash": "h2", "fileSize": 5, "extra": [1, 2]},
            "a/one": {"bundleName": "a/one", "hash": "h1", "fileSize": 9, "dependencies": ["b/two"]},
        }});
        return reply(200, encrypt(&document));
    }
    reply(404, "")
}

/// A `cp` service that knows the application versions in `known`.
fn cp_service(known: &'static [&'static str]) -> String {
    serve(move |request| cp_answer(known, request))
}

struct Collected(Vec<u8>);

impl Write for Collected {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Sink for Collected {
    fn reset(&mut self) -> io::Result<()> {
        self.0.clear();
        Ok(())
    }
}

/// A credential whose middle part holds the user id 123.
const CREDENTIAL: &str = "head.eyJ1c2VySWQiOjEyM30.signature";

#[test]
fn an_account_is_created_once_and_tells_the_asset_version() {
    let created = Arc::new(AtomicUsize::new(0));
    let counter = created.clone();
    let base = serve(
        move |request| match (request.method.as_str(), request.path.as_str()) {
            ("POST", "/prod/account") => {
                counter.fetch_add(1, Ordering::SeqCst);
                assert_eq!(decrypt(&request.body)["deviceModel"], "d");
                reply(200, encrypt(&json!({"credential": CREDENTIAL})))
            }
            ("PUT", "/prod/login/123") => {
                assert_eq!(decrypt(&request.body)["credential"], CREDENTIAL);
                reply(
                    200,
                    encrypt(&json!({"assetVersion": "11", "assetHash": "new"})),
                )
            }
            _ => cp_answer(&["1.0.0"], request),
        },
    );
    let mut region = cp_region(&base, cp_client("1.0.0"), json!({}));
    region["urls"]["account"] = json!(format!("http://{base}/{{profile}}/account"));
    region["urls"]["login"] = json!(format!("http://{base}/{{profile}}/login/{{user_id}}"));
    region["secrets"]["account"] = json!("TEST_ACCOUNT");
    let config = json!({"regions": {"r": region}}).to_string();
    let files = SecretFiles::new("account");
    let load = || Region::load("r", &config).unwrap();

    assert_eq!(
        create_account(load(), &files.secrets()).unwrap(),
        "TEST_ACCOUNT"
    );
    assert_eq!(
        std::fs::read_to_string(files.0.join("TEST_ACCOUNT")).unwrap(),
        CREDENTIAL
    );
    let error = create_account(load(), &files.secrets()).unwrap_err();
    assert_eq!(error.code, "secret");
    assert_eq!(created.load(Ordering::SeqCst), 1);

    let identity = open(load(), &files.secrets())
        .map_err(|e| e.to_string())
        .unwrap();
    let AssetIdentity::Cp {
        asset_version,
        asset_hash,
        ..
    } = identity.identity(None).unwrap().asset
    else {
        panic!("not a cp identity");
    };
    assert_eq!((asset_version.as_str(), asset_hash.as_str()), ("11", "new"));
}

#[test]
fn a_directory_is_a_source_of_its_files() {
    let root = std::env::temp_dir().join(format!("sekai-dir-source-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("music/long")).unwrap();
    std::fs::write(root.join("music/long/a"), b"first").unwrap();
    std::fs::write(root.join("b"), b"second!").unwrap();
    let region = json!({"regions": {"local": {"kind": "dir", "directory": root}}}).to_string();
    let open_source = || {
        let region = Region::load("local", &region).unwrap();
        open(region, &Secrets::new(None))
            .map_err(|e| e.to_string())
            .unwrap()
    };
    let source = open_source();
    let identity = source.identity(None).unwrap();
    assert_eq!(identity, open_source().identity(None).unwrap());
    let entries = source.manifest(&identity).unwrap();
    let names: Vec<_> = entries.iter().map(|e| (e.name.as_str(), e.size)).collect();
    assert_eq!(names, [("b", 7), ("music/long/a", 5)]);
    let mut sink = Collected(Vec::new());
    assert_eq!(source.fetch(&identity, &entries[1], &mut sink).unwrap(), 5);
    assert_eq!(sink.0, b"first");
    std::fs::write(root.join("b"), b"changed").unwrap();
    assert_ne!(identity, source.identity(None).unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cp_reads_the_identity_and_manifest_with_signed_cookies() {
    let base = cp_service(&["2.0.0"]);
    let (source, _files) = source("cp", cp_region(&base, cp_client("2.0.0"), json!({})));
    let identity = source.identity(None).unwrap();
    assert_eq!(
        identity.asset,
        AssetIdentity::Cp {
            asset_version: "10".to_owned(),
            asset_hash: "abc".to_owned(),
            profile: "prod".to_owned(),
            host_hash: "hh".to_owned(),
        }
    );
    assert_eq!(identity.asset.name(), "10-abc");
    let manifest = source.manifest(&identity).unwrap();
    assert_eq!(manifest[0].name, "a/one");
    assert_eq!(manifest[0].declared, "h1");
    assert_eq!(manifest[0].dependencies, ["b/two"]);
    assert_eq!(manifest[1].size, 5);
}

#[test]
fn the_newest_confirmed_version_wins() {
    let base = cp_service(&["2.0.0"]);
    let feed_base = serve(|_| {
        reply(
            200,
            json!({"appVersion": "3.0.0", "appHash": "app", "assetVersion": "10", "assetHash": "abc"})
                .to_string(),
        )
    });
    let (source, _files) = source(
        "newest",
        cp_region(
            &base,
            cp_client("1.0.0"),
            json!({"feed": format!("http://{feed_base}/feed")}),
        ),
    );
    let previous = VersionIdentity {
        asset: AssetIdentity::Cp {
            asset_version: "9".to_owned(),
            asset_hash: "old".to_owned(),
            profile: "prod".to_owned(),
            host_hash: "hh".to_owned(),
        },
        client: ClientIdentity {
            application_version: "2.0.0".to_owned(),
            application_hash: Some("app".to_owned()),
            multiplayer_version: None,
        },
        tables: None,
    };
    let identity = source.identity(Some(&previous)).unwrap();
    assert_eq!(identity.client.application_version, "2.0.0");
}

#[test]
fn a_forced_version_is_not_replaced_by_discovery() {
    let base = cp_service(&["1.0.0", "3.0.0"]);
    let feed_base = serve(|_| {
        reply(
            200,
            json!({"appVersion": "3.0.0", "appHash": "app", "assetVersion": "10", "assetHash": "abc"})
                .to_string(),
        )
    });
    let extra = json!({"feed": format!("http://{feed_base}/feed"), "force": true});
    let (source, _files) = source("force", cp_region(&base, cp_client("1.0.0"), extra));
    let identity = source.identity(None).unwrap();
    assert_eq!(identity.client.application_version, "1.0.0");
}

#[test]
fn no_confirmed_version_is_an_error() {
    let base = cp_service(&[]);
    let (source, _files) = source("none", cp_region(&base, cp_client("1.0.0"), json!({})));
    assert_eq!(source.identity(None).unwrap_err().code, "no_version");
}

fn cp_identity() -> VersionIdentity {
    VersionIdentity {
        asset: AssetIdentity::Cp {
            asset_version: "10".to_owned(),
            asset_hash: "abc".to_owned(),
            profile: "prod".to_owned(),
            host_hash: "hh".to_owned(),
        },
        client: ClientIdentity {
            application_version: "2.0.0".to_owned(),
            application_hash: Some("app".to_owned()),
            multiplayer_version: None,
        },
        tables: None,
    }
}

fn bundle(name: &str, size: u64) -> BundleEntry {
    BundleEntry {
        name: name.to_owned(),
        declared: "h".to_owned(),
        download_path: Some("ios7".to_owned()),
        size,
        dependencies: Vec::new(),
    }
}

#[test]
fn a_failed_transfer_starts_again_from_the_first_byte() {
    let calls = Arc::new(AtomicUsize::new(0));
    let seen = calls.clone();
    let base = serve(move |request| {
        if request.path == "/signature" {
            return Reply {
                status: 200,
                set_cookie: vec!["CloudFront-Policy=p".to_owned()],
                body: Vec::new(),
            };
        }
        match seen.fetch_add(1, Ordering::SeqCst) {
            0 => reply(500, "partial"),
            _ => reply(200, "whole"),
        }
    });
    let (source, _files) = source("retry", cp_region(&base, cp_client("2.0.0"), json!({})));
    let mut sink = Buffer::default();
    let read = source
        .fetch(&cp_identity(), &bundle("a/one", 0), &mut sink)
        .unwrap();
    assert_eq!(read, 5);
    assert_eq!(sink.bytes, b"whole");
    assert_eq!(sink.resets, 1);
}

#[test]
fn an_expired_signature_is_renewed() {
    let signatures = Arc::new(AtomicUsize::new(0));
    let issued = signatures.clone();
    let base = serve(move |request| {
        if request.path == "/signature" {
            let number = issued.fetch_add(1, Ordering::SeqCst) + 1;
            return Reply {
                status: 200,
                set_cookie: vec![format!("CloudFront-Policy={number}")],
                body: Vec::new(),
            };
        }
        if request.cookie == "CloudFront-Policy=1" {
            reply(200, "bytes")
        } else {
            reply(403, "")
        }
    });
    let (source, _files) = source("expired", cp_region(&base, cp_client("2.0.0"), json!({})));
    let mut sink = Buffer::default();
    source
        .fetch(&cp_identity(), &bundle("a/one", 0), &mut sink)
        .unwrap();
    assert_eq!(sink.bytes, b"bytes");
    assert_eq!(signatures.load(Ordering::SeqCst), 1);
}

fn nuverse_region(hosts: &[String], version: &str) -> serde_json::Value {
    json!({
        "kind": "nuverse",
        "platform": "ios",
        "hosts": hosts,
        "urls": {
            "version": "http://{host}/rt/{application_version}/{platform}/version",
            "manifest": "http://{host}/sf/{application_version}/{platform}{value}/info",
            "bundle": "http://{host}/sf/{application_version}/{download_path}/{bundle}",
        },
        "client": {"application_version": version},
        "secrets": {"key": "TEST_KEY", "iv": "TEST_IV"},
        "network": {"retries": 2, "back_off_ms": 1},
    })
}

fn nuverse_identity() -> VersionIdentity {
    VersionIdentity {
        asset: AssetIdentity::Nuverse {
            application_version: "1.2.0".to_owned(),
            value: "7".to_owned(),
        },
        client: ClientIdentity {
            application_version: "1.2.3".to_owned(),
            application_hash: None,
            multiplayer_version: None,
        },
        tables: None,
    }
}

#[test]
fn tables_are_read_at_the_current_revision() {
    let base = serve(|request| match request.path.as_str() {
        "/commits/HEAD" => reply(200, json!({"sha": "abc123", "other": 1}).to_string()),
        "/raw/abc123/cards.json" => reply(200, "[{\"id\":1}]"),
        _ => reply(404, ""),
    });
    let source = TableSource::Git {
        revision: format!("http://{base}/commits/HEAD"),
        revision_field: "sha".to_owned(),
        table: format!("http://{base}/raw/{{revision}}/{{name}}.json"),
    };
    let tables = Tables::open(&source, &Network::default()).unwrap();
    assert_eq!(tables.revision(), "abc123");
    assert_eq!(tables.read("cards").unwrap(), b"[{\"id\":1}]");
    assert_eq!(tables.read("events").unwrap_err().code, "status");
}

#[test]
fn a_revision_document_without_the_field_is_an_error() {
    let base = serve(|_| reply(200, "{}"));
    let source = TableSource::Git {
        revision: format!("http://{base}/commits/HEAD"),
        revision_field: "sha".to_owned(),
        table: format!("http://{base}/{{name}}"),
    };
    let error = Tables::open(&source, &Network::default()).err().unwrap();
    assert_eq!(error.code, "decode");
}

#[test]
fn tables_in_a_directory_are_named_by_their_contents() {
    let directory = std::env::temp_dir().join(format!("sekai-tables-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("cards.json"), "[1]").unwrap();
    let source = TableSource::Dir {
        directory: directory.clone(),
    };
    let first = Tables::open(&source, &Network::default()).unwrap();
    assert_eq!(first.read("cards").unwrap(), b"[1]");
    std::fs::write(directory.join("cards.json"), "[2]").unwrap();
    let second = Tables::open(&source, &Network::default()).unwrap();
    assert_ne!(first.revision(), second.revision());
    std::fs::remove_dir_all(&directory).unwrap();
}

#[test]
fn nuverse_falls_back_to_the_minor_version() {
    let base = serve(|request| match request.path.as_str() {
        "/rt/1.2.0/ios/version" => reply(200, "7\n"),
        "/sf/1.2.0/ios7/info" => {
            let document = json!({"bundles": {
                "a": {"crc": 1234u64, "hash": null, "fileSize": 3, "downloadPath": "ios7"},
            }});
            reply(200, encrypt(&document))
        }
        _ => reply(404, ""),
    });
    let (source, _files) = source("nuverse", nuverse_region(&[base], "1.2.3"));
    let identity = source.identity(None).unwrap();
    assert_eq!(identity, nuverse_identity());
    let manifest = source.manifest(&identity).unwrap();
    assert_eq!(
        manifest,
        [BundleEntry {
            name: "a".to_owned(),
            declared: "1234".to_owned(),
            download_path: Some("ios7".to_owned()),
            size: 3,
            dependencies: Vec::new(),
        }]
    );
}

#[test]
fn a_failing_host_is_rotated_away() {
    let bad = Arc::new(AtomicUsize::new(0));
    let hits = bad.clone();
    let base = serve(move |request| {
        if request.path.starts_with("/bad") {
            hits.fetch_add(1, Ordering::SeqCst);
            reply(500, "")
        } else {
            reply(200, "1234567")
        }
    });
    let hosts = [format!("{base}/bad"), format!("{base}/good")];
    let (source, _files) = source("rotate", nuverse_region(&hosts, "1.2.0"));
    let mut sink = Buffer::default();
    // The byte values of "aa" add up to an even number, so the first host is tried first.
    source
        .fetch(&nuverse_identity(), &bundle("aa", 3), &mut sink)
        .unwrap();
    assert_eq!(sink.bytes, b"1234567");
    assert_eq!(bad.load(Ordering::SeqCst), 1);
}

#[test]
fn a_bundle_of_the_wrong_length_fails() {
    let base = serve(|_| reply(200, "short"));
    let (source, _files) = source("length", nuverse_region(&[base], "1.2.0"));
    let mut sink = Buffer::default();
    let error = source
        .fetch(&nuverse_identity(), &bundle("a", 100), &mut sink)
        .unwrap_err();
    assert_eq!(error.code, "length");
    assert_eq!(sink.resets, 2);
}

#[test]
fn a_missing_secret_stops_the_source_before_any_request() {
    let config = json!({"regions": {"r": nuverse_region(&["h".to_owned()], "1.2.0")}}).to_string();
    let region = Region::load("r", &config).unwrap();
    let error = open(region, &Secrets::default()).err().unwrap();
    assert_eq!(error.code, "secret");
    assert!(error.message.contains("TEST_KEY"));
}
