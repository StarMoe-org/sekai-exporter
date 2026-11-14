//! A CDN of the `nuverse` protocol for the application version [`APPLICATION`] on `ios`, whose
//! current asset version a test sets.
//!
//! The version document names the current value, the manifest of each value lists its bundles,
//! encrypted with [`KEY`] and [`IV`], and each bundle is downloaded after the marker that leaves
//! it otherwise as built.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use aes::Aes128;
use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockModeEncrypt, KeyIvInit};
use serde_json::json;

use crate::http::{self, Response};

/// The application version the CDN serves.
pub const APPLICATION: &str = "1.0.0";
/// The AES-128 key of the documents.
pub const KEY: [u8; 16] = *b"0123456789abcdef";
/// The AES-128 initialization vector of the documents.
pub const IV: [u8; 16] = *b"fedcba9876543210";
/// The names of the secrets that hold [`KEY`] and [`IV`].
const SECRETS: [&str; 2] = ["SEKAI_TESTKIT_KEY", "SEKAI_TESTKIT_IV"];
/// The marker a downloaded bundle starts with when it is otherwise as built.
const PREFIXED: [u8; 4] = [0x20, 0, 0, 0];

/// A bundle of an asset version.
pub struct Bundle {
    /// The name of the bundle in the manifest.
    pub name: String,
    /// The CRC its manifest declares.
    pub crc: u64,
    /// The bundle as built.
    pub bytes: Vec<u8>,
}

struct State {
    /// The bundles of each asset version, by the value of the version document.
    versions: BTreeMap<String, Vec<Bundle>>,
    current: Mutex<String>,
}

/// A CDN served on a local port.
pub struct Cdn {
    state: Arc<State>,
    host: String,
}

impl Cdn {
    /// Serves `versions`, the bundles of each asset version by its value, with `current` as the
    /// current one.
    pub fn start(versions: BTreeMap<String, Vec<Bundle>>, current: &str) -> Self {
        let state = Arc::new(State {
            versions,
            current: Mutex::new(current.to_owned()),
        });
        let shared = Arc::clone(&state);
        let host = http::listen(move |request| match shared.answer(&request.path) {
            Some(body) => Response::new(200, body),
            None => Response::new(404, ""),
        });
        Self { state, host }
    }

    /// Makes `value` the current asset version.
    pub fn serve_version(&self, value: &str) {
        let mut current = self
            .state
            .current
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *current = value.to_owned();
    }

    /// The configuration of a region of the CDN, whose secrets [`Cdn::write_secrets`] writes.
    pub fn region(&self) -> serde_json::Value {
        json!({
            "kind": "nuverse",
            "platform": "ios",
            "hosts": [self.host],
            "urls": {
                "version": "http://{host}/rt/{application_version}/{platform}/version",
                "manifest": "http://{host}/sf/{application_version}/{platform}{value}/info",
                "bundle": "http://{host}/sf/{application_version}/{download_path}/{bundle}",
            },
            "client": {"application_version": APPLICATION},
            "secrets": {"key": SECRETS[0], "iv": SECRETS[1]},
            "network": {"retries": 1, "back_off_ms": 1},
        })
    }

    /// Writes the secrets of [`Cdn::region`] as files in `directory`.
    pub fn write_secrets(directory: &Path) -> std::io::Result<()> {
        let hex = |bytes: &[u8]| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        std::fs::create_dir_all(directory)?;
        std::fs::write(directory.join(SECRETS[0]), hex(&KEY))?;
        std::fs::write(directory.join(SECRETS[1]), hex(&IV))
    }
}

impl State {
    /// The body at `path`, or `None` when there is nothing there.
    fn answer(&self, path: &str) -> Option<Vec<u8>> {
        if path == format!("/rt/{APPLICATION}/ios/version") {
            let current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
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

/// The manifest of the asset version `value`, which holds `bundles`.
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
            (bundle.name.clone(), entry)
        })
        .collect();
    json!({ "bundles": entries })
}

fn encrypt(document: &serde_json::Value) -> Vec<u8> {
    let mut buffer = rmp_serde::to_vec(document).expect("a JSON value is also MessagePack");
    let length = buffer.len();
    buffer.resize(length + 16, 0);
    let cipher = cbc::Encryptor::<Aes128>::new_from_slices(&KEY, &IV)
        .expect("the key and the IV have 16 bytes");
    let written = cipher
        .encrypt_padded::<Pkcs7>(&mut buffer, length)
        .expect("the buffer has room for a block of padding");
    written.to_vec()
}
