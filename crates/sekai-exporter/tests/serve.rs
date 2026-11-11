//! `sekai-exporter serve` over the export of the fixture bundles of `sekai-unity`: every path of
//! the published tree is served with its bytes, by the current and by the named version.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../sekai-unity/tests/fixtures")
}

/// The files of the tree at `root`, by the paths its listing names.
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let listing = std::fs::read(root.join("_listing.json")).unwrap();
    let listing: serde_json::Value = serde_json::from_slice(&listing).unwrap();
    listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            let path = entry["path"].as_str().unwrap();
            (path.to_owned(), std::fs::read(root.join(path)).unwrap())
        })
        .collect()
}

/// A running server, stopped when dropped.
struct Server {
    child: Child,
    address: String,
}

impl Server {
    fn start(output: &Path, routes: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_sekai-exporter"))
            .arg("serve")
            .arg(output)
            .arg("--routes")
            .arg(routes)
            .args(["--listen", "127.0.0.1:0", "--threads", "2"])
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stderr.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let address = line
            .trim()
            .strip_prefix("serving on http://")
            .unwrap_or_else(|| panic!("{line}"))
            .to_owned();
        Self { child, address }
    }

    /// Sends a request and reads the whole response.
    fn request(&self, method: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
        let mut stream = TcpStream::connect(&self.address).unwrap();
        let mut request =
            format!("{method} {path} HTTP/1.1\r\nHost: test\r\nConnection: close\r\n");
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        Reply::parse(&bytes)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Reply {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl Reply {
    fn parse(bytes: &[u8]) -> Self {
        let end = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = std::str::from_utf8(&bytes[..end]).unwrap();
        let mut lines = head.split("\r\n");
        let status = lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers = lines
            .map(|line| {
                let (name, value) = line.split_once(':').unwrap();
                (name.to_ascii_lowercase(), value.trim().to_owned())
            })
            .collect();
        Self {
            status,
            headers,
            body: bytes[end + 4..].to_vec(),
        }
    }

    fn is(&self, status: u16, body: &[u8]) -> bool {
        self.status == status && self.body == body
    }

    fn header(&self, name: &str) -> &str {
        self.headers.get(name).map_or("", String::as_str)
    }
}

/// `path` with every byte but unreserved ones and `/` percent-encoded.
fn encode(path: &str) -> String {
    path.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                char::from(byte).to_string()
            }
            byte => format!("%{byte:02X}"),
        })
        .collect()
}

#[test]
fn published_paths_are_served_with_their_bytes() {
    let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join("serve");
    let _ = std::fs::remove_dir_all(&output);
    let status = Command::new(env!("CARGO_BIN_EXE_sekai-exporter"))
        .arg("export")
        .arg(fixtures().join("lzma"))
        .arg(&output)
        .arg("--tree")
        .arg(output.join("tree"))
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    let routes = output.join("routes.json");
    std::fs::write(
        &routes,
        r#"{ "routes": [
            { "pattern": "/{region}@{version}/{path}", "view": "canonical" },
            { "pattern": "/{region}/{path}", "view": "canonical" }
        ] }"#,
    )
    .unwrap();
    let pointer =
        std::fs::read_to_string(output.join("published/local/canonical/current.json")).unwrap();
    let pointer: serde_json::Value = serde_json::from_str(&pointer).unwrap();
    let version = pointer["published_version"].as_str().unwrap();

    let files = tree(&output.join("tree"));
    assert!(!files.is_empty());
    let server = Server::start(&output, &routes);
    for (path, bytes) in &files {
        let url = format!("/local/{}", encode(path));
        let reply = server.request("GET", &url, &[]);
        assert!(reply.is(200, bytes), "{path}: {}", reply.status);
        assert_eq!(reply.header("cache-control"), "public, max-age=60");
        let pinned = server.request("GET", &format!("/local@{version}/{}", encode(path)), &[]);
        assert!(pinned.is(200, bytes), "{path}: {}", pinned.status);
        assert_eq!(
            pinned.header("cache-control"),
            "public, max-age=31536000, immutable"
        );
        let etag = reply.header("etag");
        let cached = server.request("GET", &url, &[("If-None-Match", etag)]);
        assert_eq!((cached.status, cached.body.len()), (304, 0), "{path}");
        let head = server.request("HEAD", &url, &[]);
        assert_eq!(head.header("content-length"), bytes.len().to_string());
        assert!(head.body.is_empty());
    }

    let (path, bytes) = files.iter().max_by_key(|(_, bytes)| bytes.len()).unwrap();
    let part = server.request(
        "GET",
        &format!("/local/{}", encode(path)),
        &[("Range", "bytes=1-3")],
    );
    assert!(part.is(206, &bytes[1..4]), "{}", part.status);
    assert_eq!(
        part.header("content-range"),
        format!("bytes 1-3/{}", bytes.len())
    );

    for missing in [
        "/local/none.png",
        "/other/a.png",
        "/local@0.0.0-none/a.png",
        "/",
    ] {
        assert_eq!(server.request("GET", missing, &[]).status, 404, "{missing}");
    }
    assert_eq!(server.request("DELETE", "/local/a.png", &[]).status, 405);
}
