//! The server over output directories written here: the bytes and headers it sends, what it
//! does with a store that is damaged, and the limits it keeps on connections.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sekai_format::paths::{Chunk, ChunkReference, PathFile, PathIndex};
use sekai_format::publication::Pointer;
use sekai_serve::server::{Options, Server};
use sekai_serve::{ObjectId, Routes};
use sha2::{Digest, Sha256};

const ROUTES: &str = r#"{ "routes": [
    { "pattern": "/{region}@{version}/{path}", "view": "storage" },
    { "pattern": "/{region}/{path}", "view": "storage" }
], "max_age": 0 }"#;

fn routes() -> Routes {
    Routes::from_json(ROUTES.as_bytes()).unwrap()
}

/// The default options, on a port that the system chooses.
fn options() -> Options {
    Options {
        listen: "127.0.0.1:0".parse().unwrap(),
        ..Options::default()
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// An output directory, as `export` and `update` leave it.
struct Output(PathBuf);

impl Output {
    /// An empty output directory for the test `name`.
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    /// The path of the object with the hash `sha256` and `extension` in the store.
    fn object(&self, sha256: [u8; 32], extension: &str) -> PathBuf {
        let object = ObjectId::new(sha256, extension).unwrap();
        self.0.join("store").join(object.path())
    }

    /// Stores `bytes` under their hash and returns it.
    fn put(&self, bytes: &[u8], extension: &str) -> [u8; 32] {
        let sha256 = sha256(bytes);
        let path = self.object(sha256, extension);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
        sha256
    }

    /// Publishes `files`, each a path and its content, as `version` of `<region>/storage` and
    /// as its current version. Returns the path of the chunk of the path index.
    fn publish(&self, region: &str, version: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let files = files.iter().map(|(path, bytes)| {
            let extension = path
                .rsplit_once('.')
                .map_or("bin", |(_, extension)| extension);
            PathFile {
                path: (*path).to_owned(),
                sha256: self.put(bytes, extension),
                extension: extension.to_owned(),
                size: bytes.len() as u64,
                media_type: format!("test/{extension}"),
            }
        });
        self.index(region, version, files.collect())
    }

    /// Publishes `files`, whose objects are stored already, as [`Output::publish`] does.
    fn index(&self, region: &str, version: &str, mut files: Vec<PathFile>) -> PathBuf {
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let chunk = self.put(&Chunk::encode(&files), "paths");
        let index = PathIndex {
            format: "path-index".to_owned(),
            version: sekai_format::paths::VERSION,
            view: "storage".to_owned(),
            published_version: version.to_owned(),
            revision: "0".repeat(8),
            files: files.len() as u64,
            chunks: vec![ChunkReference {
                first: files[0].path.clone(),
                sha256: hex(&chunk),
                files: files.len() as u64,
            }],
        };
        let root = self.put(&serde_json::to_vec(&index).unwrap(), "json");
        let pointer = Pointer {
            format: "pointer".to_owned(),
            version: sekai_format::publication::VERSION,
            published_version: version.to_owned(),
            revision: "0".repeat(8),
            root: None,
            sqlite: None,
            paths: Some(hex(&root)),
        };
        let pointer = serde_json::to_vec(&pointer).unwrap();
        let view = self.0.join("published").join(region).join("storage");
        std::fs::create_dir_all(view.join("versions")).unwrap();
        std::fs::write(view.join(format!("versions/{version}.json")), &pointer).unwrap();
        std::fs::write(view.join("current.json"), &pointer).unwrap();
        self.object(chunk, "paths")
    }

    /// Every file and directory of the output, by its path.
    fn listing(&self) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut directories = vec![self.0.clone()];
        while let Some(directory) = directories.pop() {
            for entry in std::fs::read_dir(&directory).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    directories.push(entry.path());
                }
                found.push(entry.path());
            }
        }
        found.sort();
        found
    }
}

/// A server that answers until it is dropped.
struct Running {
    address: SocketAddr,
    _runtime: tokio::runtime::Runtime,
}

impl Running {
    fn start(output: &Output, options: Options) -> Self {
        let server = Server::bind(&output.0, routes(), options).unwrap();
        let address = server.local_addr().unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_io()
            .enable_time()
            .build()
            .unwrap();
        runtime.spawn(server.serve());
        Self {
            address,
            _runtime: runtime,
        }
    }

    /// A connection that gives up on a read after `patience`.
    fn connect(&self, patience: Duration) -> TcpStream {
        let stream = TcpStream::connect(self.address).unwrap();
        stream.set_read_timeout(Some(patience)).unwrap();
        stream
    }

    /// Sends a request on a connection of its own and reads the whole response.
    fn request(&self, method: &str, path: &str, headers: &[(&str, &str)]) -> Reply {
        let mut stream = self.connect(Duration::from_secs(30));
        let mut headers = headers.to_vec();
        headers.push(("Connection", "close"));
        send(&mut stream, method, path, &headers);
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        Reply::parse(&bytes)
    }

    fn get(&self, path: &str) -> Reply {
        self.request("GET", path, &[])
    }
}

fn send(stream: &mut TcpStream, method: &str, path: &str, headers: &[(&str, &str)]) {
    let mut request = format!("{method} {path} HTTP/1.1\r\nHost: test\r\n");
    for (name, value) in headers {
        request.push_str(&format!("{name}: {value}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).unwrap();
}

struct Reply {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl Reply {
    fn parse(bytes: &[u8]) -> Self {
        let end = bytes
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .unwrap_or_else(|| panic!("no response in {} bytes", bytes.len()));
        let head = std::str::from_utf8(&bytes[..end]).unwrap();
        let mut lines = head.split("\r\n");
        let status = lines.next().unwrap().split(' ').nth(1).unwrap();
        let headers = lines.map(|line| {
            let (name, value) = line.split_once(':').unwrap();
            (name.to_ascii_lowercase(), value.trim().to_owned())
        });
        Self {
            status: status.parse().unwrap(),
            headers: headers.collect(),
            body: bytes[end + 4..].to_vec(),
        }
    }

    /// Reads one response from a connection that stays open after it.
    fn read(stream: &mut TcpStream) -> Self {
        let mut bytes = Vec::new();
        let mut byte = [0];
        while !bytes.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            bytes.push(byte[0]);
        }
        let mut reply = Self::parse(&bytes);
        reply.body = vec![0; reply.header("content-length").parse().unwrap()];
        stream.read_exact(&mut reply.body).unwrap();
        reply
    }

    fn header(&self, name: &str) -> &str {
        self.headers.get(name).map_or("", String::as_str)
    }
}

/// Reads `stream` until the server ends or resets it, and returns how many bytes that gave.
/// Fails when the stream is still open once the patience of its connection is over.
fn closed(stream: &mut TcpStream) -> u64 {
    let mut received = 0;
    let mut buffer = vec![0; 1 << 16];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => return received,
            Ok(read) => received += read as u64,
            Err(error) => match error.kind() {
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
                    panic!("the connection is still open after {received} bytes")
                }
                _ => return received,
            },
        }
    }
}

/// A file that is larger than the pieces it is read in.
fn large() -> Vec<u8> {
    (0..300_000u32).map(|at| (at % 251) as u8).collect()
}

#[test]
fn published_files_are_served_with_their_bytes_and_headers() {
    let output = Output::new("served");
    let large = large();
    let files: [(&str, &[u8]); 4] = [
        ("a.txt", b"first"),
        ("empty.bin", b""),
        ("dir/large.bin", &large),
        ("音/星 1.txt", b"named"),
    ];
    output.publish("jp", "1", &files);
    let server = Running::start(&output, options());

    for (path, bytes) in files {
        let url = match path {
            "音/星 1.txt" => "/jp/%E9%9F%B3/%E6%98%9F%201.txt".to_owned(),
            path => format!("/jp/{path}"),
        };
        let reply = server.get(&url);
        assert_eq!(reply.status, 200, "{path}");
        assert!(reply.body == bytes, "{path}");
        let extension = path.rsplit_once('.').unwrap().1;
        let etag = format!("\"{}\"", hex(&sha256(bytes)));
        for (name, value) in [
            ("content-type", format!("test/{extension}")),
            ("content-length", bytes.len().to_string()),
            ("etag", etag.clone()),
            ("cache-control", "public, max-age=0".to_owned()),
            ("accept-ranges", "bytes".to_owned()),
            ("x-content-type-options", "nosniff".to_owned()),
        ] {
            assert_eq!(reply.header(name), value, "{path}: {name}");
        }

        let pinned = server.get(&url.replacen("/jp/", "/jp@1/", 1));
        assert_eq!(pinned.status, 200, "{path}");
        assert!(pinned.body == bytes, "{path}");
        assert_eq!(
            pinned.header("cache-control"),
            "public, max-age=31536000, immutable"
        );

        let cached = server.request("GET", &url, &[("If-None-Match", &etag)]);
        assert_eq!((cached.status, cached.body.len()), (304, 0), "{path}");
        let head = server.request("HEAD", &url, &[]);
        assert_eq!((head.status, head.body.len()), (200, 0), "{path}");
        assert_eq!(head.header("content-length"), bytes.len().to_string());
    }

    for (range, first, last) in [("bytes=1-3", 1, 3), ("bytes=-5", 299_995, 299_999)] {
        let part = server.request("GET", "/jp/dir/large.bin", &[("Range", range)]);
        assert_eq!(part.status, 206, "{range}");
        assert!(part.body == large[first..=last], "{range}");
        assert_eq!(
            part.header("content-range"),
            format!("bytes {first}-{last}/300000")
        );
    }
    let past = server.request("GET", "/jp/a.txt", &[("Range", "bytes=5-")]);
    assert_eq!(
        (past.status, past.header("content-range")),
        (416, "bytes */5")
    );

    for missing in ["/jp/none.txt", "/en/a.txt", "/jp@2/a.txt", "/jp", "/"] {
        let reply = server.get(missing);
        assert_eq!(
            (reply.status, reply.body.as_slice()),
            (404, &b"not found\n"[..])
        );
    }
    assert_eq!(server.request("DELETE", "/jp/a.txt", &[]).status, 405);
    assert_eq!(server.request("OPTIONS", "/jp/a.txt", &[]).status, 204);
    let long = format!("/jp/{}", "a".repeat(9000));
    assert_eq!(server.get(&long).status, 414);
}

#[test]
fn a_connection_answers_one_request_after_another() {
    let output = Output::new("kept");
    output.publish("jp", "1", &[("a.txt", b"first"), ("b.txt", b"second")]);
    let server = Running::start(&output, options());
    let mut stream = server.connect(Duration::from_secs(30));
    for (path, bytes) in [("/jp/a.txt", &b"first"[..]), ("/jp/b.txt", b"second")] {
        send(&mut stream, "GET", path, &[]);
        let reply = Reply::read(&mut stream);
        assert_eq!((reply.status, reply.body.as_slice()), (200, bytes));
    }
    send(&mut stream, "GET", "/jp/none.txt", &[]);
    assert_eq!(Reply::read(&mut stream).status, 404);
}

#[test]
fn a_new_publication_answers_and_the_old_one_keeps_its_name() {
    let output = Output::new("republished");
    output.publish("jp", "1", &[("a.txt", b"first")]);
    let server = Running::start(&output, options());
    assert_eq!(server.get("/jp/a.txt").body, b"first");
    assert_eq!(server.get("/jp/b.txt").status, 404);

    output.publish("jp", "2", &[("a.txt", b"changed"), ("b.txt", b"added")]);
    assert_eq!(server.get("/jp/a.txt").body, b"changed");
    assert_eq!(server.get("/jp/b.txt").body, b"added");
    assert_eq!(server.get("/jp@1/a.txt").body, b"first");
    assert_eq!(server.get("/jp@1/b.txt").status, 404);
}

#[test]
fn a_version_published_again_under_its_name_answers_with_its_new_files() {
    let output = Output::new("same-name");
    output.publish("jp", "local", &[("a.txt", b"first")]);
    let server = Running::start(&output, options());
    assert_eq!(server.get("/jp@local/a.txt").body, b"first");
    assert_eq!(server.get("/jp@local/b.txt").status, 404);

    // The name stays and the files change, as on every run of `export`.
    output.publish("jp", "local", &[("a.txt", b"changed"), ("b.txt", b"added")]);
    assert_eq!(server.get("/jp@local/a.txt").body, b"changed");
    assert_eq!(server.get("/jp@local/b.txt").body, b"added");
    assert_eq!(server.get("/jp/a.txt").body, b"changed");
}

#[test]
fn a_client_that_ends_its_side_after_its_request_is_answered() {
    let output = Output::new("half-closed");
    let large = large();
    output.publish("jp", "1", &[("a.txt", b"first"), ("large.bin", &large)]);
    let server = Running::start(&output, options());

    // The client sends its request and shuts down its sending side, as simple clients do. The
    // server sees the end of the stream while it looks the file up, and still answers.
    let cases: [(&str, u16, &[u8]); 3] = [
        ("/jp/a.txt", 200, b"first"),
        ("/jp/large.bin", 200, &large),
        ("/jp/none.txt", 404, b"not found\n"),
    ];
    for (path, status, bytes) in cases {
        for _ in 0..8 {
            let mut stream = server.connect(Duration::from_secs(30));
            send(&mut stream, "GET", path, &[]);
            stream.shutdown(Shutdown::Write).unwrap();
            let mut reply = Vec::new();
            stream.read_to_end(&mut reply).unwrap();
            let reply = Reply::parse(&reply);
            assert_eq!(reply.status, status, "{path}");
            assert!(reply.body == bytes, "{path}");
        }
    }
}

#[test]
fn what_the_store_does_not_hold_whole_is_not_sent() {
    let output = Output::new("damaged");
    let files: [(&str, &[u8]); 3] = [
        ("cut.txt", b"0123456789"),
        ("gone.txt", b"gone"),
        ("whole.txt", b"whole"),
    ];
    output.publish("jp", "1", &files);
    std::fs::write(output.object(sha256(b"0123456789"), "txt"), b"01234").unwrap();
    std::fs::remove_file(output.object(sha256(b"gone"), "txt")).unwrap();
    let server = Running::start(&output, options());

    for path in ["/jp/cut.txt", "/jp/gone.txt"] {
        let reply = server.get(path);
        assert_eq!(reply.status, 500, "{path}");
        assert_eq!(reply.body, b"the file could not be read\n");
        assert_eq!(reply.header("cache-control"), "no-store");
    }
    // Their headers come from the index, so they need no object.
    assert_eq!(server.request("HEAD", "/jp/cut.txt", &[]).status, 200);
    assert_eq!(server.get("/jp/whole.txt").body, b"whole");

    // A chunk with a byte changed is not the chunk its name says, whatever it decodes to.
    let chunk = output.publish("en", "1", &[("a.txt", b"first")]);
    let mut bytes = std::fs::read(&chunk).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&chunk, &bytes).unwrap();
    assert_eq!(server.get("/en/a.txt").status, 500);
    *bytes.last_mut().unwrap() ^= 1;
    std::fs::write(&chunk, &bytes).unwrap();
    assert_eq!(server.get("/en/a.txt").body, b"first");
}

#[test]
fn the_server_writes_nothing_to_the_output() {
    let output = Output::new("read-only");
    output.publish("jp", "1", &[("a.txt", b"first")]);
    let before = output.listing();
    let server = Running::start(&output, options());
    for path in ["/jp/a.txt", "/jp/none.txt", "/en/a.txt", "/jp@9/a.txt"] {
        server.get(path);
    }
    assert_eq!(output.listing(), before);

    // A directory that does not exist is not served, and is not made either.
    let missing = output.0.join("none");
    let error = Server::bind(&missing, routes(), options()).unwrap_err();
    assert!(error.to_string().contains("is not a directory"), "{error}");
    assert!(!missing.exists());
}

#[test]
fn connections_beyond_the_limit_wait_for_room() {
    let output = Output::new("limited");
    output.publish("jp", "1", &[("a.txt", b"first")]);
    let options = Options {
        connections: 1,
        ..options()
    };
    let server = Running::start(&output, options);

    // The first connection stays open after its response and holds the only place.
    let mut first = server.connect(Duration::from_secs(30));
    send(&mut first, "GET", "/jp/a.txt", &[]);
    assert_eq!(Reply::read(&mut first).status, 200);

    // The second gets no response while the first holds the only place.
    let mut second = server.connect(Duration::from_secs(30));
    send(&mut second, "GET", "/jp/a.txt", &[("Connection", "close")]);
    std::thread::sleep(Duration::from_millis(500));
    second.set_nonblocking(true).unwrap();
    let waiting = second.read(&mut [0]).unwrap_err();
    assert_eq!(waiting.kind(), io::ErrorKind::WouldBlock);
    second.set_nonblocking(false).unwrap();

    drop(first);
    let mut bytes = Vec::new();
    second.read_to_end(&mut bytes).unwrap();
    assert_eq!(Reply::parse(&bytes).body, b"first");
}

#[test]
fn a_connection_without_a_request_is_closed() {
    let output = Output::new("silent");
    output.publish("jp", "1", &[("a.txt", b"first")]);
    let options = Options {
        connections: 1,
        head_timeout: Duration::from_millis(300),
        ..options()
    };
    let server = Running::start(&output, options);

    // One that sends nothing, one that stops in the middle of its request, and one that
    // sends no second request: each is closed, and the next gets the only place.
    let mut silent = server.connect(Duration::from_secs(30));
    closed(&mut silent);

    let mut partial = server.connect(Duration::from_secs(30));
    partial
        .write_all(b"GET /jp/a.txt HTTP/1.1\r\nHost: t")
        .unwrap();
    closed(&mut partial);

    let mut kept = server.connect(Duration::from_secs(30));
    send(&mut kept, "GET", "/jp/a.txt", &[]);
    assert_eq!(Reply::read(&mut kept).status, 200);
    closed(&mut kept);

    assert_eq!(server.get("/jp/a.txt").body, b"first");
}

#[test]
fn a_client_that_stops_reading_is_dropped() {
    const SIZE: u64 = 128 << 20;
    let output = Output::new("stalled");
    output.publish("jp", "1", &[("a.txt", b"first")]);
    // A file too large for the buffers of a connection. The server checks the size of an
    // object and not its hash, so the file is named by the hash of something else.
    let sha256 = sha256(b"large");
    let object = output.object(sha256, "bin");
    std::fs::create_dir_all(object.parent().unwrap()).unwrap();
    std::fs::File::create(&object)
        .unwrap()
        .set_len(SIZE)
        .unwrap();
    let large = PathFile {
        path: "large.bin".to_owned(),
        sha256,
        extension: "bin".to_owned(),
        size: SIZE,
        media_type: "test/bin".to_owned(),
    };
    output.index("en", "1", vec![large]);
    let options = Options {
        connections: 1,
        idle_timeout: Duration::from_millis(300),
        ..options()
    };
    let server = Running::start(&output, options);

    // The response is never read, so it stops moving once the buffers of the connection are
    // full. The connection holds the only place: the next request is answered only once the
    // server has dropped it.
    let mut stalled = server.connect(Duration::from_secs(30));
    send(
        &mut stalled,
        "GET",
        "/en/large.bin",
        &[("Connection", "close")],
    );
    assert_eq!(server.get("/jp/a.txt").body, b"first");
    let received = closed(&mut stalled);
    assert!(received < SIZE, "{received}");
}

#[test]
fn a_request_head_that_is_too_long_is_refused() {
    let output = Output::new("long-head");
    output.publish("jp", "1", &[("a.txt", b"first")]);
    let server = Running::start(&output, options());

    // The server stops reading in the middle of the request, so the rest of the request may
    // not be taken, and a system may then drop the response on its way.
    let long = "x".repeat(20 << 10);
    let request = format!("GET /jp/a.txt HTTP/1.1\r\nHost: test\r\nX-Long: {long}\r\n\r\n");
    let mut stream = server.connect(Duration::from_secs(30));
    let _ = stream.write_all(request.as_bytes());
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply);
    let refused = reply.is_empty() || reply.starts_with(b"HTTP/1.1 431 ");
    assert!(refused, "{}", String::from_utf8_lossy(&reply));

    assert_eq!(server.get("/jp/a.txt").body, b"first");
}

#[test]
fn an_index_does_not_lead_out_of_the_store() {
    let output = Output::new("escape");
    // A file beside the store, and an index entry whose extension is a path to it. The
    // directory the path goes through exists, so the path does lead to the file.
    std::fs::write(output.0.join("secret.txt"), b"secret").unwrap();
    let secret = sha256(b"secret");
    let extension = "/../../../secret.txt";
    let through = output.0.join("store").join(&hex(&secret)[..2]);
    let through = through.join(format!("{}.", hex(&secret)));
    std::fs::create_dir_all(&through).unwrap();
    let leaked = std::fs::read(format!("{}{extension}", through.display())).unwrap();
    assert_eq!(leaked, b"secret");

    let file = |path: &str, extension: &str, media_type: &str| PathFile {
        path: path.to_owned(),
        sha256: secret,
        extension: extension.to_owned(),
        size: 6,
        media_type: media_type.to_owned(),
    };
    output.put(b"secret", "txt");
    let files = vec![
        file("leak.txt", extension, "text/plain"),
        file("split.txt", "txt", "text/plain\r\nx-added: 1"),
        file("stored.txt", "txt", "text/plain"),
    ];
    output.index("jp", "1", files);
    let server = Running::start(&output, options());

    for path in ["/jp/leak.txt", "/jp/split.txt"] {
        for method in ["GET", "HEAD"] {
            let reply = server.request(method, path, &[]);
            assert_eq!(reply.status, 500, "{method} {path}");
            assert_ne!(reply.body, b"secret", "{method} {path}");
            assert_eq!(reply.header("x-added"), "", "{method} {path}");
        }
    }
    assert_eq!(server.get("/jp/stored.txt").body, b"secret");

    // Nor does a request: a path is looked up in the index and never opened.
    for path in [
        "/jp/../secret.txt",
        "/jp/%2e%2e/secret.txt",
        "/../secret.txt",
    ] {
        let reply = server.get(path);
        assert_eq!(reply.status, 404, "{path}");
    }
}

#[test]
fn a_pointer_that_is_too_long_is_not_read() {
    let output = Output::new("long-pointer");
    output.publish("jp", "1", &[("a.txt", b"first")]);
    let pointer = std::fs::read(output.0.join("published/jp/storage/current.json")).unwrap();
    // The same pointer with a megabyte of white space after it, which a reader of JSON skips.
    let mut long = pointer.clone();
    long.resize(pointer.len() + (1 << 20), b' ');
    let view = output.0.join("published/en/storage");
    std::fs::create_dir_all(&view).unwrap();
    std::fs::write(view.join("current.json"), &long).unwrap();
    let server = Running::start(&output, options());

    assert_eq!(server.get("/en/a.txt").status, 500);
    assert_eq!(server.get("/jp/a.txt").body, b"first");

    // Nothing of the failure is remembered: the pointer answers once it is short.
    std::fs::write(view.join("current.json"), &pointer).unwrap();
    assert_eq!(server.get("/en/a.txt").body, b"first");
}
