//! `sekai-exporter update` of a `nuverse` region that a local HTTP server serves, whose current
//! version changes between runs.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::{Arc, Mutex};

use aes::Aes128;
use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockModeEncrypt, KeyIvInit};
use serde_json::json;

const KEY: [u8; 16] = *b"0123456789abcdef";
const IV: [u8; 16] = *b"fedcba9876543210";
const APPLICATION: &str = "1.0.0";
/// The marker that a downloaded bundle starts with when it is otherwise as built.
const PREFIXED: [u8; 4] = [0x20, 0, 0, 0];

fn fixture(compression: &str, name: &str) -> Vec<u8> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sekai-unity/tests/fixtures");
    std::fs::read(fixtures.join(compression).join(name)).unwrap()
}

/// The uncompressed data fixture with its file and the file it refers to renamed, so that it
/// depends on no other bundle of the versions.
fn lone() -> Vec<u8> {
    let mut bytes = fixture("uncompressed", "fixture_data");
    for (from, to) in [
        (
            "2e6074037da702b660ee3dbd01c9ea62",
            "00000000000000000000000000000001",
        ),
        (
            "9076adc37246e4c243c45fbc50ed10e1",
            "00000000000000000000000000000002",
        ),
    ] {
        let (from, to) = (from.as_bytes(), to.as_bytes());
        while let Some(at) = bytes.windows(from.len()).position(|window| window == from) {
            bytes[at..at + to.len()].copy_from_slice(to);
        }
    }
    bytes
}

fn encrypt(document: &serde_json::Value) -> Vec<u8> {
    let plain = rmp_serde::to_vec(document).unwrap();
    let length = plain.len();
    let mut buffer = plain;
    buffer.resize(length + 16, 0);
    let cipher = cbc::Encryptor::<Aes128>::new_from_slices(&KEY, &IV).unwrap();
    let written = cipher.encrypt_padded::<Pkcs7>(&mut buffer, length).unwrap();
    written.to_vec()
}

/// A bundle of a version: its name, the CRC its manifest declares and its bytes, which are
/// downloaded after [`PREFIXED`].
struct Bundle {
    name: &'static str,
    crc: u64,
    bytes: Vec<u8>,
}

/// The versions a CDN serves, by the value of the version document, and the current one.
struct Cdn {
    versions: BTreeMap<&'static str, Vec<Bundle>>,
    current: Mutex<&'static str>,
}

impl Cdn {
    /// The body at `path`, or `None` when there is nothing there.
    fn answer(&self, path: &str) -> Option<Vec<u8>> {
        let current = *self.current.lock().unwrap();
        if path == format!("/rt/{APPLICATION}/ios/version") {
            return Some(format!("{current}\n").into_bytes());
        }
        let rest = path.strip_prefix(&format!("/sf/{APPLICATION}/ios"))?;
        let (value, rest) = rest.split_once('/')?;
        let bundles = self.versions.get(value)?;
        if rest == "info" {
            return Some(encrypt(&manifest(value, bundles)));
        }
        let bundle = bundles.iter().find(|bundle| bundle.name == rest)?;
        Some([&PREFIXED, bundle.bytes.as_slice()].concat())
    }
}

/// The manifest of the version `value`, which holds `bundles`.
fn manifest(value: &str, bundles: &[Bundle]) -> serde_json::Value {
    let entries: serde_json::Map<String, serde_json::Value> = bundles
        .iter()
        .map(|bundle| {
            let entry = json!({
                "crc": bundle.crc,
                "hash": null,
                "fileSize": bundle.bytes.len(),
                "downloadPath": format!("ios{value}"),
            });
            (bundle.name.to_owned(), entry)
        })
        .collect();
    json!({ "bundles": entries })
}

fn answer(mut stream: TcpStream, cdn: &Cdn) -> io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
    }
    let head = String::from_utf8_lossy(&head);
    let path = head.split(' ').nth(1).unwrap_or("");
    let (status, body) = match cdn.answer(path) {
        Some(body) => (200, body),
        None => (404, Vec::new()),
    };
    let head = format!(
        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(&body)
}

/// Serves `cdn` on a local port and returns `host:port`.
fn serve(cdn: Arc<Cdn>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let cdn = cdn.clone();
            std::thread::spawn(move || {
                let _ = answer(stream, &cdn);
            });
        }
    });
    address
}

/// A scratch directory with the configuration and the secrets of the region `r`, whose CDN
/// serves three versions: in the second a bundle has moved, and in the third the other one
/// declares other content.
struct Scratch {
    root: PathBuf,
    cdn: Arc<Cdn>,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        let secrets = root.join("secrets");
        std::fs::create_dir_all(&secrets).unwrap();
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        std::fs::write(secrets.join("TEST_KEY"), hex(&KEY)).unwrap();
        std::fs::write(secrets.join("TEST_IV"), hex(&IV)).unwrap();

        let bundle = |name, crc, file| Bundle {
            name,
            crc,
            bytes: fixture("lzma", file),
        };
        let lone = || Bundle {
            name: "e/lone",
            crc: 41,
            bytes: lone(),
        };
        let versions = BTreeMap::from([
            (
                "1",
                vec![
                    bundle("a/data", 11, "fixture_data"),
                    bundle("b/scene", 21, "fixture_scene"),
                    lone(),
                ],
            ),
            (
                "2",
                vec![
                    bundle("a/data", 11, "fixture_data"),
                    bundle("c/scene", 21, "fixture_scene"),
                    lone(),
                ],
            ),
            (
                "3",
                vec![
                    bundle("a/data", 12, "fixture_data"),
                    bundle("c/scene", 21, "fixture_scene"),
                    lone(),
                ],
            ),
        ]);
        let cdn = Arc::new(Cdn {
            versions,
            current: Mutex::new("1"),
        });
        let host = serve(cdn.clone());
        let region = json!({
            "kind": "nuverse",
            "platform": "ios",
            "hosts": [host],
            "urls": {
                "version": "http://{host}/rt/{application_version}/{platform}/version",
                "manifest": "http://{host}/sf/{application_version}/{platform}{value}/info",
                "bundle": "http://{host}/sf/{application_version}/{download_path}/{bundle}",
            },
            "client": {"application_version": APPLICATION},
            "secrets": {"key": "TEST_KEY", "iv": "TEST_IV"},
            "network": {"retries": 1, "back_off_ms": 1},
        });
        let config = json!({ "regions": { "r": region } });
        std::fs::write(root.join("config.json"), config.to_string()).unwrap();
        Self { root, cdn }
    }

    fn serve_version(&self, value: &'static str) {
        *self.cdn.current.lock().unwrap() = value;
    }

    /// Runs `update` of the region into the output directory `output`.
    fn update(&self, output: &str) -> String {
        let run = Command::new(env!("CARGO_BIN_EXE_sekai-exporter"))
            .arg("update")
            .arg("r")
            .arg(self.root.join(output))
            .arg("--config")
            .arg(self.root.join("config.json"))
            .arg("--secrets")
            .arg(self.root.join("secrets"))
            .args(["--threads", "2"])
            .output()
            .unwrap();
        succeeded(&run)
    }

    /// The current pointer of `view` in the output directory `output`.
    fn pointer(&self, output: &str, view: &str) -> serde_json::Value {
        let path = self
            .root
            .join(output)
            .join(format!("published/r/{view}/current.json"));
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }
}

fn succeeded(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    stdout
}

#[test]
fn each_update_publishes_what_an_update_from_nothing_publishes() {
    let scratch = Scratch::new("increment-same");
    scratch.update("earlier");
    for value in ["2", "3"] {
        scratch.serve_version(value);
        let increment = scratch.update("earlier");
        // The lone bundle is the same, so its unit keeps what the last run gave.
        assert!(increment.contains("units 3 reused 1"), "{increment}");
        assert!(increment.contains(" kept 1 "), "{increment}");
        let fresh = format!("fresh-{value}");
        scratch.update(&fresh);
        for view in ["canonical", "storage"] {
            assert_eq!(
                scratch.pointer("earlier", view),
                scratch.pointer(&fresh, view),
                "{value} {view}"
            );
        }
    }
}
