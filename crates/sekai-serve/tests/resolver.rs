//! The resolver over publications held in memory: the responses to requests, the reads its
//! lookups make, and what it remembers between them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use sekai_format::paths::{Chunk, ChunkReference, PathFile, PathIndex};
use sekai_format::publication::{
    IndexChunk, IndexRoot, ListingEntry, ListingTable, ObjectReference, Pointer, TableChunk,
    VERSION as PUBLICATION_VERSION,
};
use sekai_serve::{
    Body, Fault, Fetched, Limits, Lookup, ObjectId, Pending, Read, Request, Resolver, Response,
    Routes, Slice,
};
use sha2::{Digest, Sha256};

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn object(sha256: [u8; 32], extension: &str) -> ObjectId {
    ObjectId::new(sha256, extension).unwrap()
}

/// The file at `path` of a view. Its content is its path, and its extension is its type.
fn file(path: &str) -> PathFile {
    let extension = path
        .rsplit_once('.')
        .map_or("bin", |(_, extension)| extension);
    PathFile {
        path: path.to_owned(),
        sha256: sha256(path.as_bytes()),
        extension: extension.to_owned(),
        size: path.len() as u64,
        media_type: format!("test/{extension}"),
    }
}

fn files(paths: &[&str]) -> Vec<PathFile> {
    paths.iter().map(|path| file(path)).collect()
}

/// What is published, as a host reads it: pointers by their path and objects by theirs.
#[derive(Default)]
struct Published {
    pointers: HashMap<String, Vec<u8>>,
    objects: HashMap<String, Vec<u8>>,
    /// The names whose reads fail.
    failing: Vec<String>,
    /// The reads made, in order.
    reads: RefCell<Vec<Read>>,
}

impl Published {
    /// Publishes `files`, which are in path order, as `version` of `target` (`<region>/<view>`)
    /// and as its current version, with `per_chunk` files in a chunk. Returns the path of the
    /// root and of each chunk.
    fn publish(
        &mut self,
        target: &str,
        version: &str,
        files: &[PathFile],
        per_chunk: usize,
    ) -> (String, Vec<String>) {
        let mut chunks = Vec::new();
        let mut chunk_paths = Vec::new();
        for part in files.chunks(per_chunk) {
            let bytes = Chunk::encode(part);
            let sha256 = sha256(&bytes);
            chunks.push(ChunkReference {
                first: part[0].path.clone(),
                sha256: hex(&sha256),
                files: part.len() as u64,
            });
            chunk_paths.push(object(sha256, "paths").path());
            self.objects.insert(object(sha256, "paths").path(), bytes);
        }
        let index = PathIndex {
            format: "path-index".to_owned(),
            version: sekai_format::paths::VERSION,
            view: target.rsplit('/').next().unwrap().to_owned(),
            published_version: version.to_owned(),
            revision: "0".repeat(8),
            files: files.len() as u64,
            chunks,
        };
        let bytes = serde_json::to_vec(&index).unwrap();
        let root = sha256(&bytes);
        self.objects.insert(object(root, "json").path(), bytes);
        self.point(target, version, Some(hex(&root)));
        (object(root, "json").path(), chunk_paths)
    }

    /// Writes the pointer of `version` of `target`, which names the path index `paths`, and makes
    /// it current.
    fn point(&mut self, target: &str, version: &str, paths: Option<String>) {
        let pointer = Pointer {
            format: "pointer".to_owned(),
            version: sekai_format::publication::POINTER_VERSION,
            published_version: version.to_owned(),
            revision: "0".repeat(8),
            root: None,
            paths,
            tree: None,
            history: None,
            changes: None,
        };
        self.make_current(target, &pointer);
    }

    /// Writes `pointer` as the pointer of its version of `target`, and makes it current.
    fn make_current(&mut self, target: &str, pointer: &Pointer) {
        let version = &pointer.published_version;
        let bytes = serde_json::to_vec(pointer).unwrap();
        for name in ["current".to_owned(), format!("versions/{version}")] {
            self.pointers
                .insert(format!("{target}/{name}.json"), bytes.clone());
        }
    }

    fn fetch(&self, read: &Read) -> Fetched<'_> {
        self.reads.borrow_mut().push(read.clone());
        let (name, stored) = match read {
            Read::Pointer(name) => (name.clone(), self.pointers.get(name)),
            Read::Root(object)
            | Read::Chunk(object)
            | Read::IndexRoot(object)
            | Read::ListingChunk(object) => (object.path(), self.objects.get(&object.path())),
        };
        match stored {
            _ if self.failing.contains(&name) => Fetched::Failed("the disk is on fire"),
            Some(bytes) => Fetched::Found(bytes),
            None => Fetched::Absent,
        }
    }

    /// The kinds of the reads made since the last call.
    fn reads(&self) -> Vec<&'static str> {
        let kind = |read: Read| match read {
            Read::Pointer(_) => "pointer",
            Read::Root(_) => "root",
            Read::Chunk(_) => "chunk",
            Read::IndexRoot(_) => "index_root",
            Read::ListingChunk(_) => "listing_chunk",
        };
        self.reads.take().into_iter().map(kind).collect()
    }
}

fn routes(settings: &str) -> Routes {
    let json = format!(
        r#"{{ "routes": [
            {{ "pattern": "/{{region}}@{{version}}/{{view}}/{{path}}" }},
            {{ "pattern": "/{{region}}/{{view}}/{{path}}" }}
        ]{settings} }}"#
    );
    Routes::from_json(json.as_bytes()).unwrap()
}

fn resolver(settings: &str) -> Resolver {
    Resolver::new(routes(settings), Limits::default())
}

fn get(path: &str) -> Request<'_> {
    Request {
        method: "GET",
        path,
        range: None,
        if_none_match: None,
        if_range: None,
    }
}

/// Answers `request` at `now` seconds from what `published` holds, as a host does.
fn lookup(
    resolver: &Resolver,
    published: &Published,
    request: &Request<'_>,
    now: u64,
) -> (Response, Option<Fault>) {
    let now = Duration::from_secs(now);
    let mut lookup = resolver.lookup(request, now);
    loop {
        lookup = match lookup {
            Lookup::Done(response) => return (response, None),
            Lookup::Failed(response, fault) => return (response, Some(fault)),
            Lookup::Read(pending) => {
                let fetched = published.fetch(pending.read());
                pending.supply(fetched, now)
            }
        };
    }
}

/// The response to `request`, which no fault prevents.
fn answer(resolver: &Resolver, published: &Published, request: &Request<'_>) -> Response {
    let (response, fault) = lookup(resolver, published, request, 0);
    assert_eq!(fault, None);
    response
}

fn header<'r>(response: &'r Response, name: &str) -> Option<&'r str> {
    response
        .headers
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, value)| value.as_str())
}

mod responses {
    use super::*;

    /// The storage view of `jp` with one file of ten bytes, `music/a.mp3`, and one whose name
    /// is not ASCII.
    fn published() -> Published {
        let mut published = Published::default();
        let files = files(&["a/b c.png", "music/a.mp3", "music/long/星.mp3"]);
        published.publish("jp/storage", "6.1.0", &files, 8192);
        published
    }

    const PATH: &str = "/jp/storage/music/a.mp3";

    fn etag() -> String {
        format!("\"{}\"", hex(&sha256(b"music/a.mp3")))
    }

    fn slice(start: u64, length: u64) -> Body {
        Body::Object(Slice {
            object: object(sha256(b"music/a.mp3"), "mp3"),
            start,
            length,
            size: 11,
        })
    }

    #[test]
    fn a_file_is_sent_with_its_name_type_and_hash() {
        let (resolver, published) = (resolver(""), published());
        let response = answer(&resolver, &published, &get(PATH));
        assert_eq!(response.status, 200);
        assert_eq!(response.body, slice(0, 11));
        for (name, value) in [
            ("content-type", "test/mp3"),
            ("content-length", "11"),
            ("accept-ranges", "bytes"),
            ("etag", etag().as_str()),
            ("cache-control", "public, max-age=60"),
            ("content-disposition", "inline; filename=\"a.mp3\""),
            ("x-content-type-options", "nosniff"),
        ] {
            assert_eq!(header(&response, name), Some(value), "{name}");
        }
        if let Body::Object(slice) = &response.body {
            let hash = hex(&sha256(b"music/a.mp3"));
            assert_eq!(slice.object.path(), format!("{}/{hash}.mp3", &hash[..2]));
            assert_eq!(slice.object.to_string(), slice.object.path());
        }

        let path = "/jp/storage/music/long/%E6%98%9F.mp3";
        let named = answer(&resolver, &published, &get(path));
        assert_eq!(
            header(&named, "content-disposition"),
            Some("inline; filename=\"_.mp3\"; filename*=UTF-8''%E6%98%9F.mp3")
        );
        let spaced = answer(&resolver, &published, &get("/jp/storage/a/b%20c.png"));
        assert_eq!(
            header(&spaced, "content-disposition"),
            Some("inline; filename=\"b c.png\"")
        );
    }

    #[test]
    fn a_head_request_gets_the_headers_without_the_body() {
        let (resolver, published) = (resolver(""), published());
        let head = |path| Request {
            method: "HEAD",
            ..get(path)
        };
        let response = answer(&resolver, &published, &head(PATH));
        assert_eq!((response.status, &response.body), (200, &Body::Empty));
        assert_eq!(header(&response, "content-length"), Some("11"));
        let missing = answer(&resolver, &published, &head("/jp/storage/none"));
        assert_eq!((missing.status, &missing.body), (404, &Body::Empty));

        // The headers are those of `GET`, the length of the body among them, whatever the
        // status: a file, a path the view does not have, a path of no route, a range the file
        // does not have, and a file that cannot be sent.
        let unsent = PathFile {
            extension: "../x".to_owned(),
            ..file("unsent")
        };
        let mut damaged = Published::default();
        damaged.publish("jp/storage", "1", &[unsent], 8192);
        let cases = [
            (&published, PATH, None, 200, "11"),
            (&published, "/jp/storage/none", None, 404, "10"),
            (&published, "/none", None, 404, "10"),
            (&published, PATH, Some("bytes=11-"), 416, "22"),
            (&damaged, "/jp/storage/unsent", None, 500, "27"),
        ];
        for (published, path, range, status, length) in cases {
            let resolver = super::resolver("");
            let request = |method| Request {
                method,
                range,
                ..get(path)
            };
            let (got, _) = lookup(&resolver, published, &request("GET"), 0);
            let (headed, _) = lookup(&resolver, published, &request("HEAD"), 0);
            assert_eq!((got.status, headed.status), (status, status), "{path}");
            assert_eq!(header(&got, "content-length"), Some(length), "{path}");
            assert_eq!(headed.headers, got.headers, "{path}");
            assert_eq!(headed.body, Body::Empty, "{path}");
            if let Body::Text(text) = got.body {
                assert_eq!(text.len().to_string(), length, "{path}");
            }
        }
    }

    #[test]
    fn a_named_version_is_cached_as_long_as_the_current_one() {
        let (resolver, published) = (resolver(""), published());
        let pinned = answer(&resolver, &published, &get("/jp@6.1.0/storage/music/a.mp3"));
        assert_eq!(pinned.status, 200);
        assert_eq!(header(&pinned, "cache-control"), Some("public, max-age=60"));
    }

    /// The response to a request for `range` of the file of eleven bytes.
    fn ranged(range: &str) -> Response {
        let request = Request {
            range: Some(range),
            ..get(PATH)
        };
        answer(&resolver(""), &published(), &request)
    }

    #[test]
    fn byte_ranges_are_served() {
        for (range, first, length) in [
            ("bytes=0-3", 0, 4),
            ("bytes=4-", 4, 7),
            ("bytes=8-100", 8, 3),
            ("bytes=-3", 8, 3),
            ("bytes=-30", 0, 11),
        ] {
            let response = ranged(range);
            assert_eq!(response.status, 206, "{range}");
            assert_eq!(response.body, slice(first, length), "{range}");
            let content_range = format!("bytes {first}-{}/11", first + length - 1);
            assert_eq!(
                header(&response, "content-range"),
                Some(content_range.as_str())
            );
            assert_eq!(
                header(&response, "content-length"),
                Some(length.to_string().as_str())
            );
        }
    }

    #[test]
    fn a_range_past_the_end_is_not_satisfiable() {
        for range in ["bytes=11-", "bytes=-0", "bytes=100-200"] {
            let response = ranged(range);
            assert_eq!(response.status, 416, "{range}");
            assert_eq!(header(&response, "content-range"), Some("bytes */11"));
        }
    }

    #[test]
    fn an_empty_file_has_no_range() {
        let empty = PathFile {
            size: 0,
            ..file("empty.bin")
        };
        let mut published = Published::default();
        published.publish("jp/storage", "1", &[empty], 8192);
        let resolver = resolver("");
        let whole = answer(&resolver, &published, &get("/jp/storage/empty.bin"));
        assert_eq!(
            (whole.status, header(&whole, "content-length")),
            (200, Some("0"))
        );
        let request = Request {
            range: Some("bytes=0-"),
            ..get("/jp/storage/empty.bin")
        };
        let response = answer(&resolver, &published, &request);
        assert_eq!(response.status, 416);
        assert_eq!(header(&response, "content-range"), Some("bytes */0"));
    }

    #[test]
    fn other_ranges_get_the_whole_file() {
        for range in [
            "bytes=0-1,4-5",
            "bytes=5-2",
            "bytes=+1-2",
            "items=0-1",
            "bytes=-",
        ] {
            let response = ranged(range);
            assert_eq!(
                (response.status, &response.body),
                (200, &slice(0, 11)),
                "{range}"
            );
            assert_eq!(header(&response, "content-range"), None, "{range}");
        }
    }

    #[test]
    fn a_range_for_another_version_gets_the_whole_file() {
        let (resolver, published) = (resolver(""), published());
        let tag = etag();
        for (if_range, status) in [(tag.as_str(), 206), ("\"00\"", 200), ("W/\"ab\"", 200)] {
            let request = Request {
                range: Some("bytes=0-0"),
                if_range: Some(if_range),
                ..get(PATH)
            };
            assert_eq!(
                answer(&resolver, &published, &request).status,
                status,
                "{if_range}"
            );
        }
    }

    #[test]
    fn a_known_file_is_not_modified() {
        let (resolver, published) = (resolver(""), published());
        let tag = etag();
        let weak = format!("W/{tag}");
        let listed = format!("\"00\", {tag}");
        for (if_none_match, status) in [
            (tag.as_str(), 304),
            (weak.as_str(), 304),
            (listed.as_str(), 304),
            ("*", 304),
            ("\"00\"", 200),
        ] {
            let request = Request {
                if_none_match: Some(if_none_match),
                ..get(PATH)
            };
            let response = answer(&resolver, &published, &request);
            assert_eq!(response.status, status, "{if_none_match}");
            if status == 304 {
                assert_eq!(response.body, Body::Empty);
                assert_eq!(header(&response, "etag"), Some(tag.as_str()));
            }
        }
    }

    #[test]
    fn redirects_point_at_the_object() {
        let settings = r#", "delivery": "redirect", "store_url": "https://cdn.example/store/""#;
        let response = answer(&resolver(settings), &published(), &get(PATH));
        assert_eq!((response.status, &response.body), (302, &Body::Empty));
        let hash = hex(&sha256(b"music/a.mp3"));
        let location = format!("https://cdn.example/store/{}/{hash}.mp3", &hash[..2]);
        assert_eq!(header(&response, "location"), Some(location.as_str()));
        assert_eq!(
            header(&response, "cache-control"),
            Some("public, max-age=60")
        );
    }

    #[test]
    fn requests_without_a_file_are_answered_at_once() {
        let (resolver, published) = (resolver(r#", "cors": "*""#), published());
        let missing = answer(&resolver, &published, &get("/jp/storage/none.mp3"));
        assert_eq!(
            (missing.status, &missing.body),
            (404, &Body::Text("not found\n"))
        );
        assert_eq!(
            header(&missing, "cache-control"),
            Some("public, max-age=60")
        );
        assert_eq!(header(&missing, "access-control-allow-origin"), Some("*"));
        published.reads();

        let with = |method, path| Request {
            method,
            ..get(path)
        };
        let unrouted = answer(&resolver, &published, &get("/jp/storage"));
        assert_eq!(unrouted.status, 404);

        let refused = answer(&resolver, &published, &with("POST", PATH));
        assert_eq!(refused.status, 405);
        assert_eq!(header(&refused, "allow"), Some("GET, HEAD, OPTIONS"));

        let preflight = answer(&resolver, &published, &with("OPTIONS", PATH));
        assert_eq!((preflight.status, &preflight.body), (204, &Body::Empty));
        assert_eq!(
            header(&preflight, "access-control-allow-headers"),
            Some("Range, If-None-Match, If-Range")
        );

        let long = format!("/jp/storage/{}", "a".repeat(8 << 10));
        assert_eq!(answer(&resolver, &published, &get(&long)).status, 414);
        assert_eq!(published.reads(), [""; 0]);
    }

    #[test]
    fn cors_headers_are_only_sent_when_configured() {
        let response = answer(&resolver(""), &published(), &get(PATH));
        assert_eq!(header(&response, "access-control-allow-origin"), None);
    }

    #[test]
    fn a_file_that_its_index_describes_wrongly_is_not_sent() {
        let escaping = PathFile {
            extension: "../../x".to_owned(),
            ..file("a")
        };
        let typed = PathFile {
            media_type: "text/plain\r\nx-injected: 1".to_owned(),
            ..file("b")
        };
        let mut published = Published::default();
        published.publish("jp/storage", "1", &[escaping, typed], 8192);
        let resolver = resolver("");
        for path in ["/jp/storage/a", "/jp/storage/b"] {
            let (response, fault) = lookup(&resolver, &published, &get(path), 0);
            assert_eq!(response.status, 500, "{path}");
            assert_eq!(header(&response, "cache-control"), Some("no-store"));
            let fault = fault.unwrap();
            assert!(matches!(fault.read, Read::Chunk(_)), "{fault}");
        }
    }
}

mod lookups {
    use super::*;

    /// Two chunks of two files each.
    const PATHS: [&str; 4] = ["b/1.png", "b/2.png", "d/1.png", "d/2.png"];

    fn published() -> Published {
        let mut published = Published::default();
        published.publish("jp/storage", "1", &files(&PATHS), 2);
        published
    }

    fn status(resolver: &Resolver, published: &Published, path: &str, now: u64) -> u16 {
        lookup(resolver, published, &get(path), now).0.status
    }

    #[test]
    fn a_lookup_reads_the_pointer_the_root_and_the_chunk_once() {
        let (resolver, published) = (resolver(""), published());
        assert_eq!(status(&resolver, &published, "/jp/storage/b/1.png", 0), 200);
        assert_eq!(published.reads(), ["pointer", "root", "chunk"]);
        // The other file of the chunk, and a path the chunk does not have.
        assert_eq!(status(&resolver, &published, "/jp/storage/b/2.png", 1), 200);
        assert_eq!(status(&resolver, &published, "/jp/storage/b/3.png", 1), 404);
        assert_eq!(published.reads(), [""; 0]);
        assert_eq!(status(&resolver, &published, "/jp/storage/d/2.png", 2), 200);
        assert_eq!(published.reads(), ["chunk"]);
    }

    /// A lookup of `path` that has made its reads up to the one of its chunk, which it waits
    /// for, as it does while another lookup of the same chunk makes that read.
    fn waiting_for_its_chunk<'a>(
        resolver: &'a Resolver,
        published: &Published,
        path: &'a str,
    ) -> Pending<'a> {
        let mut lookup = resolver.lookup(&get(path), Duration::ZERO);
        loop {
            lookup = match lookup {
                Lookup::Read(pending) if matches!(pending.read(), Read::Chunk(_)) => {
                    return pending;
                }
                Lookup::Read(pending) => {
                    let fetched = published.fetch(pending.read());
                    pending.supply(fetched, Duration::ZERO)
                }
                other => panic!("{path}: {other:?}"),
            };
        }
    }

    #[test]
    fn a_lookup_that_waited_goes_on_from_what_another_one_read() {
        let (resolver, published) = (resolver(""), published());
        let reading = waiting_for_its_chunk(&resolver, &published, "/jp/storage/b/1.png");
        let waiting = waiting_for_its_chunk(&resolver, &published, "/jp/storage/b/2.png");
        assert_eq!(published.reads(), ["pointer", "root"]);

        let fetched = published.fetch(reading.read());
        let Lookup::Done(read) = reading.supply(fetched, Duration::ZERO) else {
            panic!("the lookup that read the chunk is not done");
        };
        let Lookup::Done(waited) = waiting.resume(Duration::ZERO) else {
            panic!("the lookup that waited is not done");
        };
        assert_eq!((read.status, waited.status), (200, 200));
        assert_eq!(published.reads(), ["chunk"]);
    }

    #[test]
    fn a_lookup_takes_a_pointer_read_since_it_started_whatever_max_age() {
        let resolver = resolver(r#", "max_age": 0"#);
        let published = published();
        let Lookup::Read(waiting) = resolver.lookup(&get("/jp/storage/b/2.png"), Duration::ZERO)
        else {
            panic!("the lookup did not ask for the pointer");
        };
        assert!(matches!(waiting.read(), Read::Pointer(_)));
        assert_eq!(status(&resolver, &published, "/jp/storage/b/1.png", 1), 200);
        published.reads();
        let Lookup::Done(waited) = waiting.resume(Duration::from_secs(1)) else {
            panic!("the lookup that waited is not done");
        };
        assert_eq!(waited.status, 200);
        assert_eq!(published.reads(), [""; 0]);
        // A lookup that starts later reads the pointer again.
        assert_eq!(status(&resolver, &published, "/jp/storage/b/2.png", 2), 200);
        assert_eq!(published.reads(), ["pointer"]);
    }

    #[test]
    fn a_lookup_that_waited_for_what_is_not_remembered_reads_it_itself() {
        let limits = Limits {
            index_bytes: 0,
            ..Limits::default()
        };
        let (resolver, published) = (Resolver::new(routes(""), limits), published());
        let reading = waiting_for_its_chunk(&resolver, &published, "/jp/storage/b/1.png");
        let waiting = waiting_for_its_chunk(&resolver, &published, "/jp/storage/b/2.png");
        let chunk = waiting.read().clone();
        let fetched = published.fetch(reading.read());
        assert!(matches!(
            reading.supply(fetched, Duration::ZERO),
            Lookup::Done(_)
        ));
        let Lookup::Read(again) = waiting.resume(Duration::ZERO) else {
            panic!("the lookup that waited did not ask for its chunk");
        };
        assert_eq!(again.read(), &chunk);
    }

    #[test]
    fn a_path_before_the_first_chunk_reads_no_chunk() {
        let (resolver, published) = (resolver(""), published());
        assert_eq!(status(&resolver, &published, "/jp/storage/a.png", 0), 404);
        assert_eq!(published.reads(), ["pointer", "root"]);
        // Paths between the chunks and after the last are looked up in the chunk before them.
        assert_eq!(status(&resolver, &published, "/jp/storage/c.png", 0), 404);
        assert_eq!(status(&resolver, &published, "/jp/storage/e.png", 0), 404);
        assert_eq!(published.reads(), ["chunk", "chunk"]);
    }

    #[test]
    fn a_new_publication_takes_effect_after_max_age() {
        let (resolver, mut published) = (resolver(r#", "max_age": 30"#), published());
        assert_eq!(status(&resolver, &published, "/jp/storage/b/1.png", 0), 200);
        published.publish("jp/storage", "2", &files(&["b/2.png", "new.png"]), 2);
        published.reads();

        // The reading of the pointer holds for 30 seconds, so the first version still answers.
        assert_eq!(
            status(&resolver, &published, "/jp/storage/b/1.png", 29),
            200
        );
        assert_eq!(
            status(&resolver, &published, "/jp/storage/new.png", 29),
            404
        );
        assert_eq!(published.reads(), ["chunk"]);

        assert_eq!(
            status(&resolver, &published, "/jp/storage/new.png", 30),
            200
        );
        assert_eq!(published.reads(), ["pointer", "root", "chunk"]);
        assert_eq!(
            status(&resolver, &published, "/jp/storage/b/1.png", 31),
            404
        );
        // The first version stays reachable by its name.
        assert_eq!(
            status(&resolver, &published, "/jp@1/storage/b/1.png", 31),
            200
        );
        assert_eq!(published.reads(), ["pointer"]);
    }

    #[test]
    fn a_pointer_that_names_the_same_index_reads_nothing_else() {
        let (resolver, published) = (resolver(""), published());
        assert_eq!(status(&resolver, &published, "/jp/storage/b/1.png", 0), 200);
        published.reads();
        assert_eq!(
            status(&resolver, &published, "/jp/storage/b/1.png", 60),
            200
        );
        assert_eq!(published.reads(), ["pointer"]);
    }

    #[test]
    fn the_pointer_of_a_named_version_is_read_again_after_max_age() {
        let (resolver, published) = (resolver(""), published());
        for (now, reads) in [(0, 3), (59, 0), (60, 1), (119, 0), (1_000_000, 1)] {
            assert_eq!(
                status(&resolver, &published, "/jp@1/storage/b/1.png", now),
                200
            );
            assert_eq!(published.reads().len(), reads, "{now}");
        }

        // A version that is not published may be published later, so it is looked for again.
        for (now, reads) in [(0, 1), (59, 0), (60, 1)] {
            assert_eq!(
                status(&resolver, &published, "/jp@2/storage/b/1.png", now),
                404
            );
            assert_eq!(published.reads().len(), reads, "{now}");
        }
    }

    #[test]
    fn a_version_published_again_under_its_name_takes_effect_after_max_age() {
        let (resolver, mut published) = (resolver(r#", "max_age": 30"#), published());
        let named = |path: &str| format!("/jp@1/storage/{path}");
        assert_eq!(status(&resolver, &published, &named("b/1.png"), 0), 200);
        // The name stays and the files change, as when `export` publishes `local` on its next
        // run, or `update --force` the version it has published before.
        published.publish("jp/storage", "1", &files(&["b/2.png", "new.png"]), 2);
        published.reads();

        // The reading of the pointer holds for 30 seconds, as that of the current version does.
        assert_eq!(status(&resolver, &published, &named("b/1.png"), 29), 200);
        assert_eq!(status(&resolver, &published, &named("new.png"), 29), 404);
        assert_eq!(published.reads(), ["chunk"]);

        assert_eq!(status(&resolver, &published, &named("new.png"), 30), 200);
        assert_eq!(published.reads(), ["pointer", "root", "chunk"]);
        assert_eq!(status(&resolver, &published, &named("b/1.png"), 31), 404);
        assert_eq!(status(&resolver, &published, &named("b/2.png"), 31), 200);
        assert_eq!(published.reads(), [""; 0]);

        // A version that is no longer published is not found, once its reading is over.
        published.pointers.remove("jp/storage/versions/1.json");
        assert_eq!(status(&resolver, &published, &named("b/2.png"), 59), 200);
        assert_eq!(status(&resolver, &published, &named("b/2.png"), 60), 404);
        assert_eq!(published.reads(), ["pointer"]);
    }

    #[test]
    fn what_is_not_published_is_remembered_within_the_limit() {
        let limits = Limits {
            pointers: 16,
            ..Limits::default()
        };
        let (resolver, published) = (Resolver::new(routes(""), limits), published());
        assert_eq!(status(&resolver, &published, "/jp/storage/b/1.png", 0), 200);
        published.reads();

        for region in 0..10_000 {
            let path = format!("/region-{region}/storage/a.png");
            assert_eq!(status(&resolver, &published, &path, 0), 404);
            assert!(resolver.usage().pointers <= 16);
        }
        assert_eq!(published.reads().len(), 10_000);
        // The last names asked for are remembered, and the rest are looked for again.
        assert_eq!(
            status(&resolver, &published, "/region-9999/storage/a.png", 0),
            404
        );
        assert_eq!(published.reads(), [""; 0]);
        assert_eq!(status(&resolver, &published, "/jp/storage/b/1.png", 0), 200);
        assert_eq!(published.reads(), ["pointer"]);
    }

    #[test]
    fn names_longer_than_a_file_name_are_not_looked_for() {
        let (resolver, published) = (resolver(""), published());
        let region = "r".repeat(201);
        for path in [
            format!("/{region}/storage/a.png"),
            format!("/jp/{region}/a.png"),
            format!("/jp@{region}/storage/a.png"),
        ] {
            assert_eq!(status(&resolver, &published, &path, 0), 404);
        }
        assert_eq!(published.reads(), [""; 0]);
        assert_eq!(resolver.usage().pointers, 0);
    }

    #[test]
    fn decoded_indexes_stay_within_their_budget() {
        let paths: Vec<String> = (0..400).map(|at| format!("dir/{at:04}.png")).collect();
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        let mut published = Published::default();
        published.publish("jp/storage", "1", &files(&paths), 10);

        let limits = Limits {
            index_bytes: 16 << 10,
            ..Limits::default()
        };
        let resolver = Resolver::new(routes(""), limits);
        for _ in 0..2 {
            for path in &paths {
                let path = format!("/jp/storage/{path}");
                assert_eq!(status(&resolver, &published, &path, 0), 200);
                assert!(resolver.usage().index_bytes <= 16 << 10);
            }
        }
        // Forty chunks do not fit, so some were read again; those that fit were not.
        let reads = published.reads();
        let chunks = reads.iter().filter(|kind| **kind == "chunk").count();
        assert!((41..400).contains(&chunks), "{chunks}");
        assert!(resolver.usage().index_bytes > 0);
    }

    #[test]
    fn a_resolver_that_remembers_nothing_still_answers() {
        let limits = Limits {
            pointers: 0,
            index_bytes: 0,
        };
        let (resolver, published) = (Resolver::new(routes(""), limits), published());
        for _ in 0..2 {
            assert_eq!(status(&resolver, &published, "/jp/storage/d/1.png", 0), 200);
            assert_eq!(published.reads(), ["pointer", "root", "chunk"]);
        }
        let usage = resolver.usage();
        assert_eq!((usage.pointers, usage.index_bytes), (0, 0));
    }

    /// What is wrong with an object of the store.
    enum Damage {
        /// One of its bytes has another value.
        Changed,
        /// It ends early.
        Cut,
        /// It is another object of its kind, whole and well formed.
        Replaced,
        /// It is not there.
        Missing,
        /// It cannot be read.
        Unreadable,
    }

    #[test]
    fn an_object_that_is_not_what_its_name_says_fails_the_lookup() {
        let path = "/jp/storage/b/1.png";
        let mut published = Published::default();
        let (other_root, _) = published.publish("en/storage", "1", &files(&["b/1.png"]), 2);
        let (root, chunks) = published.publish("jp/storage", "1", &files(&PATHS), 2);
        let whole = published.objects.clone();
        // What takes the place of a replaced object: an index that has the path as well, and
        // a chunk that does not.
        let others = [("root", &other_root), ("chunk", &chunks[1])];
        let cases = [
            ("chunk", &chunks[0], Damage::Changed),
            ("chunk", &chunks[0], Damage::Cut),
            ("chunk", &chunks[0], Damage::Replaced),
            ("chunk", &chunks[0], Damage::Missing),
            ("chunk", &chunks[0], Damage::Unreadable),
            ("root", &root, Damage::Changed),
            ("root", &root, Damage::Cut),
            ("root", &root, Damage::Replaced),
            ("root", &root, Damage::Missing),
            ("root", &root, Damage::Unreadable),
        ];
        for (kind, name, damage) in cases {
            match damage {
                Damage::Changed => {
                    *published.objects.get_mut(name).unwrap().last_mut().unwrap() ^= 1
                }
                Damage::Cut => published.objects.get_mut(name).unwrap().truncate(10),
                Damage::Replaced => {
                    let (_, other) = others.iter().find(|(of, _)| *of == kind).unwrap();
                    let other = whole[*other].clone();
                    published.objects.insert(name.clone(), other);
                }
                Damage::Missing => drop(published.objects.remove(name)),
                Damage::Unreadable => published.failing.push(name.clone()),
            }
            let resolver = resolver("");
            let (response, fault) = lookup(&resolver, &published, &get(path), 0);
            assert_eq!(response.status, 500);
            assert_eq!(response.body, Body::Text("the file could not be read\n"));
            let fault = fault.unwrap();
            let read = match &fault.read {
                Read::Pointer(_) => "pointer",
                Read::Root(_) => "root",
                Read::Chunk(_) => "chunk",
                Read::IndexRoot(_) => "index_root",
                Read::ListingChunk(_) => "listing_chunk",
            };
            assert_eq!(read, kind, "{fault}");

            // Nothing of the damaged object is remembered: once it is whole, it answers.
            published.objects = whole.clone();
            published.failing.clear();
            assert_eq!(status(&resolver, &published, path, 0), 200);
        }
    }

    #[test]
    fn a_pointer_that_cannot_be_read_fails_the_lookup_until_it_can() {
        let path = "/jp/storage/b/1.png";
        let name = "jp/storage/current.json";
        let (resolver, mut published) = (resolver(""), published());
        published.failing.push(name.to_owned());
        for _ in 0..2 {
            let (response, fault) = lookup(&resolver, &published, &get(path), 0);
            assert_eq!(response.status, 500);
            let fault = fault.unwrap();
            assert_eq!(fault.read, Read::Pointer(name.to_owned()));
            assert_eq!(
                fault.to_string(),
                format!("pointer {name}: the disk is on fire")
            );
            assert_eq!(published.reads(), ["pointer"]);
        }
        published.failing.clear();
        assert_eq!(status(&resolver, &published, path, 0), 200);

        // A pointer that is not one fails as well, and nothing of it is remembered either.
        let pointer = published.pointers[name].clone();
        let text = std::str::from_utf8(&pointer).unwrap();
        let listing = text.replace(r#""format":"pointer""#, r#""format":"listing""#);
        assert_ne!(listing, text);
        published.reads();
        for invalid in [&b"{}"[..], b"", b"[1]", listing.as_bytes()] {
            published.pointers.insert(name.to_owned(), invalid.to_vec());
            let (response, fault) = lookup(&resolver, &published, &get(path), 60);
            assert_eq!(response.status, 500);
            assert_eq!(fault.unwrap().read, Read::Pointer(name.to_owned()));
            assert_eq!(published.reads(), ["pointer"]);
        }
        published.pointers.insert(name.to_owned(), pointer);
        assert_eq!(status(&resolver, &published, path, 60), 200);

        // One that is gone publishes nothing, and that is remembered like any reading.
        published.pointers.remove(name);
        assert_eq!(status(&resolver, &published, path, 120), 404);
        published.publish("jp/storage", "1", &files(&PATHS), 2);
        published.reads();
        assert_eq!(status(&resolver, &published, path, 179), 404);
        assert_eq!(published.reads(), [""; 0]);
        assert_eq!(status(&resolver, &published, path, 180), 200);
        assert_eq!(published.reads(), ["pointer"]);
    }

    #[test]
    fn a_pointer_without_a_path_index_publishes_nothing() {
        let mut published = Published::default();
        published.point("jp/canonical", "1", None);
        let resolver = resolver("");
        assert_eq!(status(&resolver, &published, "/jp/canonical/a.png", 0), 404);
        assert_eq!(published.reads(), ["pointer"]);

        published.point("jp/canonical", "1", Some("not a hash".to_owned()));
        let (response, fault) = lookup(&resolver, &published, &get("/jp/canonical/a.png"), 60);
        assert_eq!(response.status, 500);
        assert!(matches!(fault.unwrap().read, Read::Pointer(_)));
    }

    #[test]
    fn a_root_names_its_chunks_by_their_hashes_only() {
        let path = "/jp/storage/b/1.png";
        let mut published = Published::default();
        for name in [
            "../../../secret",
            "",
            "00",
            &"g".repeat(64),
            &"A".repeat(64),
        ] {
            let index = PathIndex {
                format: "path-index".to_owned(),
                version: sekai_format::paths::VERSION,
                view: "storage".to_owned(),
                published_version: "1".to_owned(),
                revision: "0".repeat(8),
                files: 1,
                chunks: vec![ChunkReference {
                    first: "b/1.png".to_owned(),
                    sha256: name.to_owned(),
                    files: 1,
                }],
            };
            let bytes = serde_json::to_vec(&index).unwrap();
            let root = sha256(&bytes);
            published.objects.insert(object(root, "json").path(), bytes);
            published.point("jp/storage", "1", Some(hex(&root)));

            // The name is not that of a chunk, so no read is made for it.
            let (response, fault) = lookup(&resolver(""), &published, &get(path), 0);
            assert_eq!(response.status, 500, "{name:?}");
            assert_eq!(fault.unwrap().read, Read::Root(object(root, "json")));
            assert_eq!(published.reads(), ["pointer", "root"]);
        }
    }

    #[test]
    fn a_host_reads_no_more_than_a_read_allows() {
        let (resolver, published) = (resolver(""), published());
        let request = get("/jp/storage/b/1.png");
        let mut lookup = resolver.lookup(&request, Duration::ZERO);
        let mut limits = Vec::new();
        while let Lookup::Read(pending) = lookup {
            limits.push(pending.read().limit());
            let fetched = published.fetch(pending.read());
            lookup = pending.supply(fetched, Duration::ZERO);
        }
        assert_eq!(limits, [64 << 10, 64 << 20, 64 << 20]);
        assert!(matches!(lookup, Lookup::Done(response) if response.status == 200));
    }
}

mod directories {
    use super::*;
    use serde_json::Value;

    /// The listing that `response` carries.
    fn listing(response: &Response) -> Value {
        let Body::Bytes(bytes) = &response.body else {
            panic!("a listing is a document: {response:?}")
        };
        serde_json::from_slice(bytes).unwrap()
    }

    /// The subdirectories and file names of the listing of `path`.
    fn names(resolver: &Resolver, published: &Published, path: &str) -> (Vec<String>, Vec<String>) {
        let listed = listing(&answer(resolver, published, &get(path)));
        let strings = |values: &Value, key: Option<&str>| {
            let values = values.as_array().unwrap().iter();
            let names = values.map(|value| key.map_or(value, |key| &value[key]));
            names
                .map(|name| name.as_str().unwrap().to_owned())
                .collect()
        };
        (
            strings(&listed["directories"], None),
            strings(&listed["files"], Some("name")),
        )
    }

    fn published() -> Published {
        let mut published = Published::default();
        let paths = [
            "a.txt",
            "b/c.txt",
            "b/d/e.txt",
            "b/d/f.txt",
            "b/g.txt",
            "b/g/h.txt",
            "i/j.txt",
        ];
        published.publish("jp/storage", "1", &files(&paths), 2);
        published
    }

    #[test]
    fn a_directory_lists_what_is_right_under_it() {
        let (resolver, published) = (resolver(""), published());
        let list = |path| names(&resolver, &published, path);
        let strings =
            |names: &[&str]| -> Vec<String> { names.iter().map(|n| n.to_string()).collect() };
        assert_eq!(
            list("/jp/storage/"),
            (strings(&["b", "i"]), strings(&["a.txt"]))
        );
        assert_eq!(
            list("/jp/storage/b/"),
            (strings(&["d", "g"]), strings(&["c.txt", "g.txt"]))
        );
        assert_eq!(
            list("/jp@1/storage/b/d/"),
            (strings(&[]), strings(&["e.txt", "f.txt"]))
        );
        for missing in ["/jp/storage/x/", "/jp/storage/b/c.txt/", "/jp/storage/b//"] {
            assert_eq!(answer(&resolver, &published, &get(missing)).status, 404);
        }
        // Without its `/`, a directory is a path that names no file.
        assert_eq!(
            answer(&resolver, &published, &get("/jp/storage/b")).status,
            404
        );
    }

    #[test]
    fn a_listing_is_a_document_of_the_publication() {
        let (resolver, published) = (resolver(r#", "max_age": 5"#), published());
        let response = answer(&resolver, &published, &get("/jp/storage/b/d/"));
        assert_eq!(response.status, 200);
        assert_eq!(header(&response, "content-type"), Some("application/json"));
        assert_eq!(
            header(&response, "cache-control"),
            Some("public, max-age=5")
        );
        let Body::Bytes(bytes) = &response.body else {
            panic!("a listing is a document")
        };
        let etag = format!("\"{}\"", hex(&sha256(bytes)));
        assert_eq!(header(&response, "etag"), Some(etag.as_str()));
        let length = bytes.len().to_string();
        assert_eq!(header(&response, "content-length"), Some(length.as_str()));

        let e = file("b/d/e.txt");
        assert_eq!(
            listing(&response),
            serde_json::json!({
                "format": "directory-listing",
                "version": 1,
                "view": "storage",
                "published_version": "1",
                "revision": "00000000",
                "path": "b/d/",
                "directories": [],
                "files": [
                    { "name": "e.txt", "sha256": hex(&e.sha256), "extension": "txt",
                      "size": e.size, "media_type": "test/txt" },
                    { "name": "f.txt", "sha256": hex(&file("b/d/f.txt").sha256),
                      "extension": "txt", "size": 9, "media_type": "test/txt" },
                ],
            })
        );

        let again = Request {
            if_none_match: Some(&etag),
            ..get("/jp/storage/b/d/")
        };
        let unchanged = answer(&resolver, &published, &again);
        assert_eq!((unchanged.status, &unchanged.body), (304, &Body::Empty));
        let head = Request {
            method: "HEAD",
            ..get("/jp/storage/b/d/")
        };
        let headers = answer(&resolver, &published, &head);
        assert_eq!(headers.body, Body::Empty);
        assert_eq!(header(&headers, "content-length"), Some(length.as_str()));
    }

    #[test]
    fn a_listing_reads_only_the_chunks_where_its_entries_start() {
        let mut published = Published::default();
        let mut paths = vec!["a/0.txt".to_owned()];
        paths.extend((0..100).map(|n| format!("a/big/{n:03}.txt")));
        paths.extend([
            "a/big.txt".to_owned(),
            "a/z.txt".to_owned(),
            "b.txt".to_owned(),
        ]);
        paths.sort();
        let paths: Vec<&str> = paths.iter().map(String::as_str).collect();
        let (_, chunks) = published.publish("jp/storage", "1", &files(&paths), 10);
        assert!(chunks.len() > 10);
        let resolver = resolver("");
        assert_eq!(
            names(&resolver, &published, "/jp/storage/a/"),
            (
                vec!["big".to_owned()],
                vec!["0.txt".to_owned(), "big.txt".to_owned(), "z.txt".to_owned()]
            )
        );
        // The chunk of `a/0.txt` and `a/big.txt`, which starts the subdirectory, and the last
        // chunk, which holds the paths past it.
        assert_eq!(published.reads(), ["pointer", "root", "chunk", "chunk"]);
        names(&resolver, &published, "/jp/storage/a/");
        assert_eq!(published.reads(), [""; 0]);
    }

    #[test]
    fn the_root_of_a_view_without_files_is_listed() {
        let mut published = Published::default();
        published.publish("jp/storage", "1", &[], 2);
        let resolver = resolver("");
        assert_eq!(
            names(&resolver, &published, "/jp/storage/"),
            (Vec::new(), Vec::new())
        );
        assert_eq!(
            answer(&resolver, &published, &get("/en/storage/")).status,
            404
        );
    }
}

mod documents {
    use super::*;

    const TREE: &str = "/trees/jp/storage";

    fn resolver() -> Resolver {
        let json = r#"{ "routes": [
            { "pattern": "/trees/{region}/{view}", "tree": true },
            { "pattern": "/history/{region}/{view}", "history": true },
            { "pattern": "/changes/{region}/{view}@{version}", "changes": true },
            { "pattern": "/{region}/{view}/{path}" }
        ] }"#;
        Resolver::new(
            Routes::from_json(json.as_bytes()).unwrap(),
            Limits::default(),
        )
    }

    /// The storage view of `jp` at version `1`, whose pointer `name` gives its documents.
    fn published(name: impl FnOnce(&mut Pointer)) -> Published {
        let mut published = Published::default();
        published.publish("jp/storage", "1", &files(&["a.png"]), 8192);
        let current = &published.pointers["jp/storage/current.json"];
        let mut pointer: Pointer = serde_json::from_slice(current).unwrap();
        name(&mut pointer);
        published.make_current("jp/storage", &pointer);
        published.reads();
        published
    }

    /// A document of 42 bytes, named by the hash of `bytes`.
    fn document(bytes: &[u8]) -> ObjectReference {
        ObjectReference {
            sha256: hex(&sha256(bytes)),
            size: 42,
        }
    }

    /// Checks that `response` sends the document `bytes` names under `file_name`.
    fn sends(response: &Response, bytes: &[u8], file_name: &str) {
        assert_eq!(response.status, 200);
        let expected = Body::Object(Slice {
            object: object(sha256(bytes), "json.gz"),
            start: 0,
            length: 42,
            size: 42,
        });
        assert_eq!(response.body, expected);
        let etag = format!("\"{}\"", document(bytes).sha256);
        let disposition = format!("inline; filename=\"{file_name}\"");
        for (name, value) in [
            ("content-type", "application/gzip"),
            ("content-length", "42"),
            ("etag", etag.as_str()),
            ("content-disposition", disposition.as_str()),
        ] {
            assert_eq!(header(response, name), Some(value), "{name}");
        }
    }

    #[test]
    fn the_tree_is_the_object_the_pointer_names() {
        let resolver = resolver();
        let published = published(|pointer| pointer.tree = Some(document(b"tree")));
        let response = answer(&resolver, &published, &get(TREE));
        sends(&response, b"tree", "directory-tree.json.gz");
        assert_eq!(published.reads(), ["pointer"]);
        // The pointer is remembered, so the next request reads nothing, and the files of the
        // view are still found by path.
        assert_eq!(answer(&resolver, &published, &get(TREE)).status, 200);
        assert_eq!(published.reads(), [""; 0]);
        let file = answer(&resolver, &published, &get("/jp/storage/a.png"));
        assert_eq!(file.status, 200);
    }

    #[test]
    fn the_history_and_the_changes_are_the_objects_the_pointer_names() {
        let resolver = resolver();
        let published = published(|pointer| {
            pointer.history = Some(document(b"history"));
            pointer.changes = Some(document(b"changes"));
        });
        let history = answer(&resolver, &published, &get("/history/jp/storage"));
        sends(&history, b"history", "version-history.json.gz");
        let changes = answer(&resolver, &published, &get("/changes/jp/storage@1"));
        sends(&changes, b"changes", "version-changes.json.gz");
        let unpublished = answer(&resolver, &published, &get("/changes/jp/storage@2"));
        assert_eq!(unpublished.status, 404);
    }

    #[test]
    fn a_publication_without_a_document_has_none_to_send() {
        let resolver = resolver();
        let published = published(|_| {});
        for path in [
            TREE,
            "/trees/en/storage",
            "/history/jp/storage",
            "/changes/jp/storage@1",
        ] {
            assert_eq!(
                answer(&resolver, &published, &get(path)).status,
                404,
                "{path}"
            );
        }
    }

    #[test]
    fn a_document_named_by_something_other_than_a_hash_fails_the_lookup() {
        let named = ObjectReference {
            sha256: "../secret".to_owned(),
            size: 1,
        };
        let resolver = resolver();
        let published = published(|pointer| pointer.history = Some(named));
        let (response, fault) = lookup(&resolver, &published, &get(TREE), 0);
        assert_eq!(response.status, 500);
        assert!(matches!(fault.unwrap().read, Read::Pointer(_)));
    }
}

mod form_listings {
    use super::*;
    use serde_json::Value;

    fn routes(settings: &str) -> Routes {
        let json = format!(
            r#"{{ "routes": [
                {{ "pattern": "/listings/{{region}}/{{view}}@{{version}}/{{form}}", "listings": true }},
                {{ "pattern": "/listings/{{region}}/{{view}}/{{form}}", "listings": true }},
                {{ "pattern": "/listings/{{region}}/{{view}}@{{version}}", "listings": true }},
                {{ "pattern": "/listings/{{region}}/{{view}}", "listings": true }},
                {{ "pattern": "/live2d/{{region}}/models", "view": "canonical", "form": "live2d-model", "listings": true }},
                {{ "pattern": "/{{region}}/{{view}}/{{path}}" }}
            ]{settings} }}"#
        );
        Routes::from_json(json.as_bytes()).unwrap()
    }

    fn resolver(settings: &str) -> Resolver {
        Resolver::new(routes(settings), Limits::default())
    }

    fn json(response: &Response) -> Value {
        let Body::Bytes(bytes) = &response.body else {
            panic!("a form listing is a document: {response:?}");
        };
        serde_json::from_slice(bytes).unwrap()
    }

    fn entry(name: &str, size: u64) -> ListingEntry {
        ListingEntry {
            name: name.to_owned(),
            fingerprint: hex(&sha256(name.as_bytes())),
            size,
        }
    }

    /// Publishes an `index-root` with `forms` (each a form name and its sorted entries, split into
    /// chunks of `per_chunk`) onto `target` at `version` / `revision`, returning the store paths of
    /// the index root and each chunk.
    fn publish_index(
        published: &mut Published,
        target: &str,
        version: &str,
        revision: &str,
        forms: &[(&str, Vec<ListingEntry>)],
        per_chunk: usize,
    ) -> (String, Vec<String>) {
        let mut tables = Vec::new();
        let mut chunk_paths = Vec::new();
        for (form, rows) in forms {
            let mut chunks = Vec::new();
            for part in rows.chunks(per_chunk) {
                let bytes = serde_json::to_vec(&IndexChunk {
                    format: "index-chunk".to_owned(),
                    version: PUBLICATION_VERSION,
                    rows: part.to_vec(),
                })
                .unwrap();
                let hash = sha256(&bytes);
                let path = object(hash, "json").path();
                published.objects.insert(path.clone(), bytes);
                chunk_paths.push(path);
                chunks.push(TableChunk {
                    first: part[0].name.clone(),
                    sha256: hex(&hash),
                    rows: part.len() as u64,
                });
            }
            tables.push(ListingTable {
                form: (*form).to_owned(),
                chunks,
            });
        }
        let index = IndexRoot {
            format: "index-root".to_owned(),
            version: PUBLICATION_VERSION,
            published_version: version.to_owned(),
            revision: revision.to_owned(),
            directories: Vec::new(),
            listings: tables,
            failures: Vec::new(),
        };
        let bytes = serde_json::to_vec(&index).unwrap();
        let root_hash = sha256(&bytes);
        let root_path = object(root_hash, "json").path();
        published.objects.insert(root_path.clone(), bytes);
        let pointer = Pointer {
            format: "pointer".to_owned(),
            version: sekai_format::publication::POINTER_VERSION,
            published_version: version.to_owned(),
            revision: revision.to_owned(),
            root: Some(hex(&root_hash)),
            paths: None,
            tree: None,
            history: None,
            changes: None,
        };
        published.make_current(target, &pointer);
        (root_path, chunk_paths)
    }

    fn sample() -> (Published, String, Vec<String>) {
        let mut published = Published::default();
        let models = vec![
            entry("live2d/model/01ichika/buildmodeldata", 100),
            entry("live2d/model/02saki/buildmodeldata", 200),
            entry("live2d/model/18mafuyu/buildmodeldata", 300),
        ];
        let motions = vec![entry("live2d/motion/01ichika_base/buildmotiondata", 50)];
        let (root, chunks) = publish_index(
            &mut published,
            "jp/canonical",
            "7.1.0",
            "rev-1",
            &[("live2d-model", models), ("live2d-motions", motions)],
            2,
        );
        // Storage view has only a path index and no `index-root`.
        published.publish("jp/storage", "7.1.0", &files(&["a.png"]), 8192);
        (published, root, chunks)
    }

    #[test]
    fn forms_and_form_assets_are_listed_with_headers_and_caching() {
        let (published, _, _) = sample();
        let resolver = resolver(r#", "max_age": 15, "cors": "*""#);

        // Listing all forms reads only the pointer and the index root (no listing chunks).
        let forms_res = answer(&resolver, &published, &get("/listings/jp/canonical"));
        assert_eq!(forms_res.status, 200);
        assert_eq!(header(&forms_res, "content-type"), Some("application/json"));
        assert_eq!(
            header(&forms_res, "cache-control"),
            Some("public, max-age=15")
        );
        assert_eq!(
            header(&forms_res, "x-content-type-options"),
            Some("nosniff")
        );
        assert_eq!(header(&forms_res, "access-control-allow-origin"), Some("*"));
        assert_eq!(published.reads(), ["pointer", "index_root"]);
        assert_eq!(
            json(&forms_res),
            serde_json::json!({
                "format": "form-listings",
                "version": 1,
                "view": "canonical",
                "published_version": "7.1.0",
                "revision": "rev-1",
                "forms": [
                    { "form": "live2d-model", "assets": 3 },
                    { "form": "live2d-motions", "assets": 1 },
                ],
            })
        );
        // Repeat reads nothing.
        assert_eq!(
            answer(&resolver, &published, &get("/listings/jp/canonical")).status,
            200
        );
        assert_eq!(published.reads(), [""; 0]);

        // Listing a multi-chunk form reads its two chunks and concatenates them in order.
        let models_res = answer(
            &resolver,
            &published,
            &get("/listings/jp/canonical/live2d-model"),
        );
        assert_eq!(models_res.status, 200);
        assert_eq!(published.reads(), ["listing_chunk", "listing_chunk"]);
        let Body::Bytes(bytes) = &models_res.body else {
            panic!("expected bytes");
        };
        let etag = format!("\"{}\"", hex(&sha256(bytes)));
        assert_eq!(header(&models_res, "etag"), Some(etag.as_str()));
        let len = bytes.len().to_string();
        assert_eq!(header(&models_res, "content-length"), Some(len.as_str()));
        assert_eq!(
            json(&models_res),
            serde_json::json!({
                "format": "form-listing",
                "version": 1,
                "view": "canonical",
                "published_version": "7.1.0",
                "revision": "rev-1",
                "form": "live2d-model",
                "assets": [
                    {
                        "name": "live2d/model/01ichika/buildmodeldata",
                        "fingerprint": hex(&sha256(b"live2d/model/01ichika/buildmodeldata")),
                        "size": 100
                    },
                    {
                        "name": "live2d/model/02saki/buildmodeldata",
                        "fingerprint": hex(&sha256(b"live2d/model/02saki/buildmodeldata")),
                        "size": 200
                    },
                    {
                        "name": "live2d/model/18mafuyu/buildmodeldata",
                        "fingerprint": hex(&sha256(b"live2d/model/18mafuyu/buildmodeldata")),
                        "size": 300
                    }
                ]
            })
        );
        // Repeat (including fixed-form route targeting the same form) reads nothing.
        assert_eq!(
            answer(&resolver, &published, &get("/live2d/jp/models")).status,
            200
        );
        assert_eq!(published.reads(), [""; 0]);

        // Conditional GET (exact, weak, comma-separated list, wildcard, mismatch), Range ignored,
        // HEAD, OPTIONS, POST, and 414 URI too long.
        let listed_tags = format!("\"00\", {etag}");
        for (if_none_match, expected) in [
            (etag.as_str(), 304),
            (&format!("W/{etag}"), 304),
            (listed_tags.as_str(), 304),
            ("*", 304),
            ("\"00\"", 200),
        ] {
            let req = Request {
                if_none_match: Some(if_none_match),
                ..get("/listings/jp/canonical/live2d-model")
            };
            let res = answer(&resolver, &published, &req);
            assert_eq!(res.status, expected, "{if_none_match}");
            if expected == 304 {
                assert_eq!(res.body, Body::Empty);
            }
        }
        let ranged = answer(
            &resolver,
            &published,
            &Request {
                range: Some("bytes=0-10"),
                ..get("/listings/jp/canonical/live2d-model")
            },
        );
        assert_eq!(ranged.status, 200);
        assert_eq!(header(&ranged, "accept-ranges"), None);
        assert_eq!(header(&ranged, "content-range"), None);
        assert_eq!(header(&ranged, "content-length"), Some(len.as_str()));

        let head = answer(
            &resolver,
            &published,
            &Request {
                method: "HEAD",
                ..get("/listings/jp/canonical/live2d-model")
            },
        );
        assert_eq!((head.status, &head.body), (200, &Body::Empty));
        assert_eq!(header(&head, "content-length"), Some(len.as_str()));

        let options = answer(
            &resolver,
            &published,
            &Request {
                method: "OPTIONS",
                ..get("/listings/jp/canonical/live2d-model")
            },
        );
        assert_eq!(options.status, 204);
        let post = answer(
            &resolver,
            &published,
            &Request {
                method: "POST",
                ..get("/listings/jp/canonical/live2d-model")
            },
        );
        assert_eq!(post.status, 405);
        let too_long = format!("/listings/jp/canonical/{}", "a".repeat(8 << 10));
        assert_eq!(answer(&resolver, &published, &get(&too_long)).status, 414);
    }

    #[test]
    fn redirect_delivery_still_serves_form_listings_as_json() {
        let (published, _, _) = sample();
        let settings = r#", "delivery": "redirect", "store_url": "https://cdn.example/store""#;
        let res = answer(
            &resolver(settings),
            &published,
            &get("/listings/jp/canonical/live2d-motions"),
        );
        assert_eq!(res.status, 200);
        assert_eq!(header(&res, "content-type"), Some("application/json"));
        assert_eq!(json(&res)["assets"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn version_pinned_and_absent_listings_behave_as_specified() {
        let (mut published, _, _) = sample();
        let resolver = resolver(r#", "max_age": 30"#);

        // Version 7.1.0 is reachable by version pin and current.
        let v1 = answer(
            &resolver,
            &published,
            &get("/listings/jp/canonical@7.1.0/live2d-model"),
        );
        assert_eq!(v1.status, 200);
        assert_eq!(json(&v1)["published_version"], "7.1.0");
        assert_eq!(json(&v1)["assets"].as_array().unwrap().len(), 3);

        // Publish version 7.2.0: after max_age, current reflects 7.2.0 while @7.1.0 stays pinned.
        publish_index(
            &mut published,
            "jp/canonical",
            "7.2.0",
            "rev-2",
            &[(
                "live2d-model",
                vec![entry("live2d/model/01ichika/buildmodeldata", 101)],
            )],
            2,
        );
        let (v2, fault) = lookup(
            &resolver,
            &published,
            &get("/listings/jp/canonical/live2d-model"),
            30,
        );
        assert_eq!(fault, None);
        assert_eq!(json(&v2)["published_version"], "7.2.0");
        assert_eq!(json(&v2)["assets"].as_array().unwrap().len(), 1);
        let (still_v1, fault) = lookup(
            &resolver,
            &published,
            &get("/listings/jp/canonical@7.1.0/live2d-model"),
            30,
        );
        assert_eq!(fault, None);
        assert_eq!(json(&still_v1)["published_version"], "7.1.0");
        assert_eq!(json(&still_v1)["assets"].as_array().unwrap().len(), 3);

        // Missing form, view without index root (storage), missing region/version -> 404.
        for missing in [
            "/listings/jp/canonical/no-such-form",
            "/listings/jp/storage",
            "/listings/jp/storage/live2d-model",
            "/listings/en/canonical",
            "/listings/jp/canonical@9.9.9/live2d-model",
        ] {
            let (res, fault) = lookup(&resolver, &published, &get(missing), 30);
            assert_eq!((res.status, fault), (404, None), "{missing}");
        }

        // Empty index root (0 forms) lists empty forms array and 404s for any form.
        publish_index(&mut published, "en/canonical", "1.0", "rev-empty", &[], 2);
        let (empty, fault) = lookup(&resolver, &published, &get("/listings/en/canonical"), 100);
        assert_eq!((empty.status, fault), (200, None));
        assert_eq!(json(&empty)["forms"], serde_json::json!([]));
        let (absent, fault) = lookup(
            &resolver,
            &published,
            &get("/listings/en/canonical/live2d-model"),
            100,
        );
        assert_eq!((absent.status, fault), (404, None));
    }

    #[test]
    fn views_sharing_an_index_root_do_not_bleed_view_names_in_cache() {
        let mut published = Published::default();
        let forms = vec![("font", vec![entry("font/main", 12)])];
        publish_index(&mut published, "jp/canonical", "1", "rev-1", &forms, 2);
        publish_index(&mut published, "jp/client", "1", "rev-1", &forms, 2);

        let resolver = resolver("");
        let a_forms = json(&answer(
            &resolver,
            &published,
            &get("/listings/jp/canonical"),
        ));
        let b_forms = json(&answer(&resolver, &published, &get("/listings/jp/client")));
        assert_eq!(a_forms["view"], "canonical");
        assert_eq!(b_forms["view"], "client");

        let a_font = json(&answer(
            &resolver,
            &published,
            &get("/listings/jp/canonical/font"),
        ));
        let b_font = json(&answer(
            &resolver,
            &published,
            &get("/listings/jp/client/font"),
        ));
        assert_eq!(a_font["view"], "canonical");
        assert_eq!(b_font["view"], "client");
    }

    #[test]
    fn corrupt_pointer_index_root_or_listing_chunk_fails_and_recovers() {
        let (mut published, root_path, chunk_paths) = sample();
        let whole = published.objects.clone();

        // Bad pointer.root hash -> 500 on pointer.
        let mut bad_ptr: Pointer =
            serde_json::from_slice(&published.pointers["jp/canonical/current.json"]).unwrap();
        let good_ptr = bad_ptr.clone();
        bad_ptr.root = Some("../escape".to_owned());
        published.make_current("jp/canonical", &bad_ptr);
        let (res, fault) = lookup(
            &resolver(""),
            &published,
            &get("/listings/jp/canonical/live2d-model"),
            0,
        );
        assert_eq!(res.status, 500);
        assert_eq!(header(&res, "cache-control"), Some("no-store"));
        assert!(matches!(fault.unwrap().read, Read::Pointer(_)));
        published.make_current("jp/canonical", &good_ptr);

        // Mutated, truncated, missing, or unreadable index root or chunk -> 500, not remembered.
        for (expected_kind, path) in [
            ("index_root", root_path.as_str()),
            ("listing_chunk", chunk_paths[0].as_str()),
            ("listing_chunk", chunk_paths[1].as_str()),
        ] {
            for mode in ["changed", "truncated", "missing", "unreadable"] {
                match mode {
                    "changed" => {
                        *published.objects.get_mut(path).unwrap().last_mut().unwrap() ^= 1;
                    }
                    "truncated" => published.objects.get_mut(path).unwrap().truncate(8),
                    "missing" => {
                        published.objects.remove(path);
                    }
                    "unreadable" => published.failing.push(path.to_owned()),
                    _ => unreachable!(),
                }
                let r = resolver("");
                let (res, fault) = lookup(
                    &r,
                    &published,
                    &get("/listings/jp/canonical/live2d-model"),
                    0,
                );
                assert_eq!(res.status, 500, "{expected_kind} {mode}");
                let fault = fault.unwrap();
                let kind = match fault.read {
                    Read::IndexRoot(_) => "index_root",
                    Read::ListingChunk(_) => "listing_chunk",
                    _ => "other",
                };
                assert_eq!(kind, expected_kind, "{fault}");

                // Recovery once whole again on the SAME resolver (failure was not cached).
                published.objects = whole.clone();
                published.failing.clear();
                let (ok, fault) = lookup(
                    &r,
                    &published,
                    &get("/listings/jp/canonical/live2d-model"),
                    0,
                );
                assert_eq!((ok.status, fault), (200, None), "{expected_kind} {mode}");
            }
        }
    }

    #[test]
    fn pointer_and_index_root_metadata_mismatch_fails_without_false_cache_hit() {
        let (mut published, _, _) = sample();
        let good_ptr: Pointer =
            serde_json::from_slice(&published.pointers["jp/canonical/current.json"]).unwrap();
        // A second view points to the SAME valid index root hash, but claims a different version.
        let mismatched = Pointer {
            published_version: "9.9.9".to_owned(),
            ..good_ptr.clone()
        };
        published.make_current("en/canonical", &mismatched);

        let resolver = resolver("");
        // Populate the cache with the valid jp publication first.
        assert_eq!(
            answer(
                &resolver,
                &published,
                &get("/listings/jp/canonical/live2d-model")
            )
            .status,
            200
        );
        assert_eq!(
            answer(&resolver, &published, &get("/listings/jp/canonical")).status,
            200
        );

        // The mismatched pointer MUST NOT get a cache hit and MUST fail with 500.
        for path in [
            "/listings/en/canonical",
            "/listings/en/canonical/live2d-model",
        ] {
            let (res, fault) = lookup(&resolver, &published, &get(path), 0);
            assert_eq!(res.status, 500, "{path}");
            assert!(matches!(fault.unwrap().read, Read::IndexRoot(_)), "{path}");
        }

        // Revision mismatch also fails.
        let rev_mismatch = Pointer {
            revision: "wrong-rev".to_owned(),
            ..good_ptr
        };
        published.make_current("tw/canonical", &rev_mismatch);
        let (res, fault) = lookup(
            &resolver,
            &published,
            &get("/listings/tw/canonical/live2d-model"),
            0,
        );
        assert_eq!(res.status, 500);
        assert!(matches!(fault.unwrap().read, Read::IndexRoot(_)));

        // On a fresh resolver, a metadata-mismatched IndexRoot is NOT remembered in state.index.
        let fresh = super::form_listings::resolver("");
        let (res, fault) = lookup(
            &fresh,
            &published,
            &get("/listings/en/canonical/live2d-model"),
            0,
        );
        assert_eq!(res.status, 500);
        assert!(matches!(fault.unwrap().read, Read::IndexRoot(_)));
        assert_eq!(fresh.usage().index_bytes, 0);
    }

    #[test]
    fn invalid_index_root_and_chunk_invariants_are_rejected() {
        let store_obj = |published: &mut Published, bytes: Vec<u8>| -> String {
            let hash = sha256(&bytes);
            published.objects.insert(object(hash, "json").path(), bytes);
            hex(&hash)
        };
        let point_root = |published: &mut Published, root_hex: String| {
            published.make_current(
                "jp/canonical",
                &Pointer {
                    format: "pointer".to_owned(),
                    version: sekai_format::publication::POINTER_VERSION,
                    published_version: "1".to_owned(),
                    revision: "r1".to_owned(),
                    root: Some(root_hex),
                    paths: None,
                    tree: None,
                    history: None,
                    changes: None,
                },
            );
        };

        let valid_chunk = |rows: Vec<ListingEntry>| {
            serde_json::to_vec(&IndexChunk {
                format: "index-chunk".to_owned(),
                version: PUBLICATION_VERSION,
                rows,
            })
            .unwrap()
        };

        // 1. Bad IndexRoot cases: wrong format, wrong version, unsorted/duplicate forms, empty
        //    chunks, zero rows, unsorted/duplicate chunk firsts, non-hash chunk sha256, asset
        //    count overflow.
        let bad_roots = vec![
            IndexRoot {
                format: "not-index-root".to_owned(),
                version: PUBLICATION_VERSION,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                ..IndexRoot::default()
            },
            IndexRoot {
                format: "index-root".to_owned(),
                version: PUBLICATION_VERSION + 1,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                ..IndexRoot::default()
            },
            IndexRoot {
                format: "index-root".to_owned(),
                version: PUBLICATION_VERSION,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                listings: vec![
                    ListingTable {
                        form: "b".to_owned(),
                        chunks: vec![TableChunk {
                            first: "a".to_owned(),
                            sha256: "0".repeat(64),
                            rows: 1,
                        }],
                    },
                    ListingTable {
                        form: "a".to_owned(),
                        chunks: vec![TableChunk {
                            first: "a".to_owned(),
                            sha256: "0".repeat(64),
                            rows: 1,
                        }],
                    },
                ],
                ..IndexRoot::default()
            },
            IndexRoot {
                format: "index-root".to_owned(),
                version: PUBLICATION_VERSION,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                listings: vec![
                    ListingTable {
                        form: "a".to_owned(),
                        chunks: vec![TableChunk {
                            first: "a".to_owned(),
                            sha256: "0".repeat(64),
                            rows: 1,
                        }],
                    },
                    ListingTable {
                        form: "a".to_owned(),
                        chunks: vec![TableChunk {
                            first: "b".to_owned(),
                            sha256: "1".repeat(64),
                            rows: 1,
                        }],
                    },
                ],
                ..IndexRoot::default()
            },
            IndexRoot {
                format: "index-root".to_owned(),
                version: PUBLICATION_VERSION,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                listings: vec![ListingTable {
                    form: "a".to_owned(),
                    chunks: Vec::new(),
                }],
                ..IndexRoot::default()
            },
            IndexRoot {
                format: "index-root".to_owned(),
                version: PUBLICATION_VERSION,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                listings: vec![ListingTable {
                    form: "a".to_owned(),
                    chunks: vec![TableChunk {
                        first: "a".to_owned(),
                        sha256: "0".repeat(64),
                        rows: 0,
                    }],
                }],
                ..IndexRoot::default()
            },
            IndexRoot {
                format: "index-root".to_owned(),
                version: PUBLICATION_VERSION,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                listings: vec![ListingTable {
                    form: "a".to_owned(),
                    chunks: vec![
                        TableChunk {
                            first: "b".to_owned(),
                            sha256: "0".repeat(64),
                            rows: 1,
                        },
                        TableChunk {
                            first: "a".to_owned(),
                            sha256: "1".repeat(64),
                            rows: 1,
                        },
                    ],
                }],
                ..IndexRoot::default()
            },
            IndexRoot {
                format: "index-root".to_owned(),
                version: PUBLICATION_VERSION,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                listings: vec![ListingTable {
                    form: "a".to_owned(),
                    chunks: vec![
                        TableChunk {
                            first: "a".to_owned(),
                            sha256: "0".repeat(64),
                            rows: 1,
                        },
                        TableChunk {
                            first: "a".to_owned(),
                            sha256: "1".repeat(64),
                            rows: 1,
                        },
                    ],
                }],
                ..IndexRoot::default()
            },
            IndexRoot {
                format: "index-root".to_owned(),
                version: PUBLICATION_VERSION,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                listings: vec![ListingTable {
                    form: "a".to_owned(),
                    chunks: vec![TableChunk {
                        first: "a".to_owned(),
                        sha256: "../secret".to_owned(),
                        rows: 1,
                    }],
                }],
                ..IndexRoot::default()
            },
            IndexRoot {
                format: "index-root".to_owned(),
                version: PUBLICATION_VERSION,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                listings: vec![ListingTable {
                    form: "a".to_owned(),
                    chunks: vec![
                        TableChunk {
                            first: "a".to_owned(),
                            sha256: "0".repeat(64),
                            rows: u64::MAX,
                        },
                        TableChunk {
                            first: "b".to_owned(),
                            sha256: "1".repeat(64),
                            rows: 1,
                        },
                    ],
                }],
                ..IndexRoot::default()
            },
        ];
        for bad in bad_roots {
            let mut published = Published::default();
            let root_hex = store_obj(&mut published, serde_json::to_vec(&bad).unwrap());
            point_root(&mut published, root_hex);
            let r = resolver("");
            let (res, fault) = lookup(&r, &published, &get("/listings/jp/canonical"), 0);
            assert_eq!(res.status, 500, "{bad:?}");
            assert!(matches!(fault.unwrap().read, Read::IndexRoot(_)), "{bad:?}");
            assert_eq!(r.usage().index_bytes, 0, "{bad:?}");
        }

        // Fast-fail on oversized declared table.assets without reading any listing chunk.
        {
            let mut published = Published::default();
            let too_many = ((64u64 << 20) / (std::mem::size_of::<ListingEntry>() as u64)) + 1;
            let root = IndexRoot {
                format: "index-root".to_owned(),
                version: PUBLICATION_VERSION,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                listings: vec![ListingTable {
                    form: "huge".to_owned(),
                    chunks: vec![TableChunk {
                        first: "a".to_owned(),
                        sha256: "0".repeat(64),
                        rows: too_many,
                    }],
                }],
                ..IndexRoot::default()
            };
            let root_hex = store_obj(&mut published, serde_json::to_vec(&root).unwrap());
            point_root(&mut published, root_hex);
            let (res, fault) = lookup(
                &resolver(""),
                &published,
                &get("/listings/jp/canonical/huge"),
                0,
            );
            assert_eq!(res.status, 500);
            assert!(matches!(fault.unwrap().read, Read::IndexRoot(_)));
            assert_eq!(published.reads(), ["pointer", "index_root"]);
        }

        // 2. Bad ListingChunk cases: wrong format, wrong version, empty rows, unsorted/duplicate
        //    rows within a chunk, row count mismatch, first key mismatch, cross-chunk disorder
        //    and cross-chunk duplicate.
        let bad_chunks = [
            (
                vec![
                    serde_json::to_vec(&IndexChunk {
                        format: "other".to_owned(),
                        version: PUBLICATION_VERSION,
                        rows: vec![entry("a", 1)],
                    })
                    .unwrap(),
                ],
                vec![("a", 1)],
            ),
            (
                vec![
                    serde_json::to_vec(&IndexChunk {
                        format: "index-chunk".to_owned(),
                        version: PUBLICATION_VERSION + 1,
                        rows: vec![entry("a", 1)],
                    })
                    .unwrap(),
                ],
                vec![("a", 1)],
            ),
            (vec![valid_chunk(vec![])], vec![("a", 1)]),
            (
                vec![valid_chunk(vec![entry("b", 1), entry("a", 1)])],
                vec![("b", 2)],
            ),
            (
                vec![valid_chunk(vec![entry("a", 1), entry("a", 2)])],
                vec![("a", 2)],
            ),
            // Row count mismatch (chunk has 1, index root says 2).
            (vec![valid_chunk(vec![entry("a", 1)])], vec![("a", 2)]),
            // First row mismatch (chunk starts with "b", index root says "a").
            (vec![valid_chunk(vec![entry("b", 1)])], vec![("a", 1)]),
            // Cross-chunk disorder: chunk 0 ends with "c", chunk 1 starts with "b".
            (
                vec![
                    valid_chunk(vec![entry("a", 1), entry("c", 1)]),
                    valid_chunk(vec![entry("b", 1)]),
                ],
                vec![("a", 2), ("b", 1)],
            ),
            // Cross-chunk duplicate: chunk 0 ends with "b", chunk 1 starts with "b".
            (
                vec![
                    valid_chunk(vec![entry("a", 1), entry("b", 1)]),
                    valid_chunk(vec![entry("b", 2)]),
                ],
                vec![("a", 2), ("b", 1)],
            ),
        ];
        for (chunks_bytes, meta) in bad_chunks {
            let mut published = Published::default();
            let table_chunks: Vec<TableChunk> = chunks_bytes
                .into_iter()
                .zip(meta)
                .map(|(bytes, (first, rows))| TableChunk {
                    first: first.to_owned(),
                    sha256: store_obj(&mut published, bytes),
                    rows,
                })
                .collect();
            let root = IndexRoot {
                format: "index-root".to_owned(),
                version: PUBLICATION_VERSION,
                published_version: "1".to_owned(),
                revision: "r1".to_owned(),
                listings: vec![ListingTable {
                    form: "live2d-model".to_owned(),
                    chunks: table_chunks,
                }],
                ..IndexRoot::default()
            };
            let root_hex = store_obj(&mut published, serde_json::to_vec(&root).unwrap());
            point_root(&mut published, root_hex);
            let r = resolver("");
            // Warm the index root first so we can verify the failing chunk adds 0 bytes to cache.
            assert_eq!(
                answer(&r, &published, &get("/listings/jp/canonical")).status,
                200
            );
            let before = r.usage().index_bytes;
            let (res, fault) = lookup(
                &r,
                &published,
                &get("/listings/jp/canonical/live2d-model"),
                0,
            );
            assert_eq!(res.status, 500);
            assert!(matches!(fault.unwrap().read, Read::ListingChunk(_)));
            if root.listings[0].chunks.len() == 1 {
                assert_eq!(r.usage().index_bytes, before);
            } else {
                // Chunk 0 succeeded and was cached; chunk 1 failed and must NOT be cached.
                published.reads();
                let (again, _) = lookup(
                    &r,
                    &published,
                    &get("/listings/jp/canonical/live2d-model"),
                    0,
                );
                assert_eq!(again.status, 500);
                assert_eq!(published.reads(), ["listing_chunk"]);
            }
        }
    }

    #[test]
    fn pointer_root_aliasing_path_index_fails_without_poisoning_paths() {
        let mut published = Published::default();
        published.publish("jp/canonical", "1", &files(&["a.png"]), 8192);
        let mut ptr: Pointer =
            serde_json::from_slice(&published.pointers["jp/canonical/current.json"]).unwrap();
        ptr.root = ptr.paths.clone();
        published.make_current("jp/canonical", &ptr);

        let resolver = resolver("");
        // Populate PathIndex cache first.
        assert_eq!(
            answer(&resolver, &published, &get("/jp/canonical/a.png")).status,
            200
        );
        // Form listing must fail with 500 (wrong format) without corrupting PathIndex cache.
        let (res, fault) = lookup(&resolver, &published, &get("/listings/jp/canonical"), 0);
        assert_eq!(res.status, 500);
        assert!(matches!(fault.unwrap().read, Read::IndexRoot(_)));
        // File lookup still works from cached PathIndex.
        assert_eq!(
            answer(&resolver, &published, &get("/jp/canonical/a.png")).status,
            200
        );
    }

    #[test]
    fn form_listings_respect_cache_limits_and_singleflight_resume() {
        let (published, _, _) = sample();

        // Read limits are 64 KiB for pointer and 64 MiB for index root and listing chunks.
        let lim_resolver = resolver("");
        let mut step =
            lim_resolver.lookup(&get("/listings/jp/canonical/live2d-model"), Duration::ZERO);
        let mut limits_seen = Vec::new();
        while let Lookup::Read(pending) = step {
            limits_seen.push(pending.read().limit());
            let fetched = published.fetch(pending.read());
            step = pending.supply(fetched, Duration::ZERO);
        }
        assert_eq!(limits_seen, [64 << 10, 64 << 20, 64 << 20, 64 << 20]);
        assert!(matches!(step, Lookup::Done(res) if res.status == 200));
        published.reads();

        // Zero-budget resolver still answers without keeping anything.
        let zero = Resolver::new(
            routes(""),
            Limits {
                pointers: 0,
                index_bytes: 0,
            },
        );
        for _ in 0..2 {
            assert_eq!(
                answer(
                    &zero,
                    &published,
                    &get("/listings/jp/canonical/live2d-model")
                )
                .status,
                200
            );
            assert_eq!(
                published.reads(),
                ["pointer", "index_root", "listing_chunk", "listing_chunk"]
            );
        }
        assert_eq!(zero.usage().index_bytes, 0);

        // Tight budget stays within limit across many form reads.
        let tight = Resolver::new(
            routes(""),
            Limits {
                pointers: 16,
                index_bytes: 1024,
            },
        );
        for path in [
            "/listings/jp/canonical",
            "/listings/jp/canonical/live2d-model",
            "/listings/jp/canonical/live2d-motions",
        ] {
            assert_eq!(answer(&tight, &published, &get(path)).status, 200);
            assert!(tight.usage().index_bytes <= 1024);
        }

        // Singleflight resume on IndexRoot and ListingChunk (including waking after Listed cached).
        let r = resolver("");
        let waiting_for = |pred: fn(&Read) -> bool| -> Pending<'_> {
            let mut l = r.lookup(&get("/listings/jp/canonical/live2d-model"), Duration::ZERO);
            loop {
                l = match l {
                    Lookup::Read(p) if pred(p.read()) => return p,
                    Lookup::Read(p) => {
                        let f = published.fetch(p.read());
                        p.supply(f, Duration::ZERO)
                    }
                    other => panic!("unexpected: {other:?}"),
                };
            }
        };
        published.reads();
        let reader_root = waiting_for(|read| matches!(read, Read::IndexRoot(_)));
        let waiter_root = waiting_for(|read| matches!(read, Read::IndexRoot(_)));
        // Finish reader completely (caches IndexRoot, chunks, and Listed).
        let mut l = {
            let f = published.fetch(reader_root.read());
            reader_root.supply(f, Duration::ZERO)
        };
        while let Lookup::Read(p) = l {
            let f = published.fetch(p.read());
            l = p.supply(f, Duration::ZERO);
        }
        assert!(matches!(l, Lookup::Done(res) if res.status == 200));
        published.reads();
        // Waiter resumes on IndexRoot and hits cached Listed directly with 0 reads.
        let Lookup::Done(res) = waiter_root.resume(Duration::ZERO) else {
            panic!("waiter did not finish from cache");
        };
        assert_eq!(res.status, 200);
        assert_eq!(published.reads(), [""; 0]);

        // Also test resume on ListingChunk directly (both cached hit and 0-budget re-read).
        fn waiting_chunk<'a>(r: &'a Resolver, published: &Published) -> Pending<'a> {
            let mut l = r.lookup(&get("/listings/jp/canonical/live2d-model"), Duration::ZERO);
            loop {
                l = match l {
                    Lookup::Read(p) if matches!(p.read(), Read::ListingChunk(_)) => return p,
                    Lookup::Read(p) => {
                        let f = published.fetch(p.read());
                        p.supply(f, Duration::ZERO)
                    }
                    other => panic!("unexpected: {other:?}"),
                };
            }
        }
        let r2 = resolver("");
        let reader_chunk = waiting_chunk(&r2, &published);
        let waiter_chunk = waiting_chunk(&r2, &published);
        let mut l = {
            let f = published.fetch(reader_chunk.read());
            reader_chunk.supply(f, Duration::ZERO)
        };
        while let Lookup::Read(p) = l {
            let f = published.fetch(p.read());
            l = p.supply(f, Duration::ZERO);
        }
        assert!(matches!(l, Lookup::Done(res) if res.status == 200));
        published.reads();
        let Lookup::Done(res) = waiter_chunk.resume(Duration::ZERO) else {
            panic!("chunk waiter did not finish from cache");
        };
        assert_eq!(res.status, 200);
        assert_eq!(published.reads(), [""; 0]);

        let uncached = Resolver::new(
            routes(""),
            Limits {
                index_bytes: 0,
                ..Limits::default()
            },
        );
        let reader_zero = waiting_chunk(&uncached, &published);
        let waiter_zero = waiting_chunk(&uncached, &published);
        let expected_chunk = waiter_zero.read().clone();
        let f = published.fetch(reader_zero.read());
        let _ = reader_zero.supply(f, Duration::ZERO);
        let Lookup::Read(again) = waiter_zero.resume(Duration::ZERO) else {
            panic!("uncached chunk waiter did not re-ask for its chunk");
        };
        assert_eq!(again.read(), &expected_chunk);
    }
}
