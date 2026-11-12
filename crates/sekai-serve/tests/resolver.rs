//! The resolver over publications held in memory: the responses to requests, the reads its
//! lookups make, and what it remembers between them.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Duration;

use sekai_format::paths::{Chunk, ChunkReference, PathFile, PathIndex};
use sekai_format::publication::Pointer;
use sekai_serve::{
    Body, Fault, Fetched, Limits, Lookup, ObjectId, Read, Request, Resolver, Response, Routes,
    Slice,
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

    /// Writes the pointer of `version` of `target` and makes it current.
    fn point(&mut self, target: &str, version: &str, paths: Option<String>) {
        let pointer = Pointer {
            format: "pointer".to_owned(),
            version: sekai_format::publication::VERSION,
            published_version: version.to_owned(),
            revision: "0".repeat(8),
            root: None,
            sqlite: None,
            paths,
        };
        let bytes = serde_json::to_vec(&pointer).unwrap();
        for name in ["current".to_owned(), format!("versions/{version}")] {
            self.pointers
                .insert(format!("{target}/{name}.json"), bytes.clone());
        }
    }

    fn fetch(&self, read: &Read) -> Fetched<'_> {
        self.reads.borrow_mut().push(read.clone());
        let (name, stored) = match read {
            Read::Pointer(name) => (name.clone(), self.pointers.get(name)),
            Read::Root(object) | Read::Chunk(object) => {
                (object.path(), self.objects.get(&object.path()))
            }
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
