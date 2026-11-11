//! Responses: files with their headers, byte ranges, conditional requests, redirects and the
//! answers that need no file.

use sekai_format::paths::PathFile;
use sekai_serve::{Body, Request, Response, Routes, Routing, Slice, Target};

const SHA256: [u8; 32] = [0xab; 32];

fn etag() -> String {
    format!("\"{}\"", "ab".repeat(32))
}

fn routes(settings: &str) -> Routes {
    let json =
        format!(r#"{{ "routes": [{{ "pattern": "/{{region}}/{{view}}/{{path}}" }}]{settings} }}"#);
    Routes::from_json(json.as_bytes()).unwrap()
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

fn file(size: u64) -> PathFile {
    PathFile {
        path: "music/long/星.mp3".to_owned(),
        sha256: SHA256,
        extension: "mp3".to_owned(),
        size,
        media_type: "audio/mpeg".to_owned(),
    }
}

/// The response to `request` when its target is `file`.
fn answer(routes: &Routes, request: &Request<'_>, found: Option<&PathFile>) -> Response {
    match routes.route(request) {
        Routing::Find(target) => routes.respond(request, &target, found),
        Routing::Reply(response) => response,
    }
}

fn header<'r>(response: &'r Response, name: &str) -> Option<&'r str> {
    response
        .headers
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, value)| value.as_str())
}

fn slice(start: u64, length: u64) -> Body {
    Body::Object(Slice {
        sha256: SHA256,
        extension: "mp3".to_owned(),
        start,
        length,
    })
}

#[test]
fn a_file_is_sent_with_its_name_type_and_hash() {
    let routes = routes("");
    let path = "/jp/canonical/music/long/%E6%98%9F.mp3";
    let response = answer(&routes, &get(path), Some(&file(10)));
    assert_eq!(response.status, 200);
    assert_eq!(response.body, slice(0, 10));
    for (name, value) in [
        ("content-type", "audio/mpeg"),
        ("content-length", "10"),
        ("accept-ranges", "bytes"),
        ("etag", etag().as_str()),
        ("cache-control", "public, max-age=60"),
        (
            "content-disposition",
            "inline; filename=\"_.mp3\"; filename*=UTF-8''%E6%98%9F.mp3",
        ),
    ] {
        assert_eq!(header(&response, name), Some(value), "{name}");
    }
    let ascii = answer(&routes, &get("/jp/canonical/a/b c.png"), Some(&file(1)));
    assert_eq!(
        header(&ascii, "content-disposition"),
        Some("inline; filename=\"b c.png\"")
    );
    if let Body::Object(slice) = &response.body {
        assert_eq!(slice.object(), format!("ab/{}.mp3", "ab".repeat(32)));
    }
}

#[test]
fn a_head_request_gets_the_headers_without_the_body() {
    let routes = routes("");
    let request = Request {
        method: "HEAD",
        ..get("/jp/canonical/a.mp3")
    };
    let response = answer(&routes, &request, Some(&file(10)));
    assert_eq!((response.status, &response.body), (200, &Body::Empty));
    assert_eq!(header(&response, "content-length"), Some("10"));
}

#[test]
fn a_named_version_is_cached_for_good() {
    let routes = routes("");
    let target = Target {
        region: "jp".to_owned(),
        view: "canonical".to_owned(),
        version: Some("6.1.0".to_owned()),
        path: "a.mp3".to_owned(),
    };
    let request = get("/jp@6.1.0/a.mp3");
    let response = routes.respond(&request, &target, Some(&file(1)));
    assert_eq!(
        header(&response, "cache-control"),
        Some("public, max-age=31536000, immutable")
    );
}

/// The response to a request for `range` of a file of `size` bytes.
fn ranged(range: &str, size: u64) -> Response {
    let request = Request {
        range: Some(range),
        ..get("/jp/canonical/a.mp3")
    };
    answer(&routes(""), &request, Some(&file(size)))
}

#[test]
fn byte_ranges_are_served() {
    for (range, first, length) in [
        ("bytes=0-3", 0, 4),
        ("bytes=4-", 4, 6),
        ("bytes=8-100", 8, 2),
        ("bytes=-3", 7, 3),
        ("bytes=-30", 0, 10),
    ] {
        let response = ranged(range, 10);
        assert_eq!(response.status, 206, "{range}");
        assert_eq!(response.body, slice(first, length), "{range}");
        let content_range = format!("bytes {first}-{}/10", first + length - 1);
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
    for (range, size) in [("bytes=10-", 10), ("bytes=-0", 10), ("bytes=0-", 0)] {
        let response = ranged(range, size);
        assert_eq!(response.status, 416, "{range}");
        let content_range = format!("bytes */{size}");
        assert_eq!(
            header(&response, "content-range"),
            Some(content_range.as_str())
        );
    }
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
        let response = ranged(range, 10);
        assert_eq!(
            (response.status, &response.body),
            (200, &slice(0, 10)),
            "{range}"
        );
        assert_eq!(header(&response, "content-range"), None, "{range}");
    }
}

#[test]
fn a_range_for_another_version_gets_the_whole_file() {
    let routes = routes("");
    let tag = etag();
    for (if_range, status) in [(tag.as_str(), 206), ("\"00\"", 200), ("W/\"ab\"", 200)] {
        let request = Request {
            range: Some("bytes=0-0"),
            if_range: Some(if_range),
            ..get("/jp/canonical/a.mp3")
        };
        assert_eq!(
            answer(&routes, &request, Some(&file(10))).status,
            status,
            "{if_range}"
        );
    }
}

#[test]
fn a_known_file_is_not_modified() {
    let routes = routes("");
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
            ..get("/jp/canonical/a.mp3")
        };
        let response = answer(&routes, &request, Some(&file(10)));
        assert_eq!(response.status, status, "{if_none_match}");
        if status == 304 {
            assert_eq!(response.body, Body::Empty);
            assert_eq!(header(&response, "etag"), Some(tag.as_str()));
        }
    }
}

#[test]
fn redirects_point_at_the_object() {
    let routes = routes(r#", "delivery": "redirect", "store_url": "https://cdn.example/store/""#);
    let response = answer(&routes, &get("/jp/canonical/a.mp3"), Some(&file(10)));
    assert_eq!((response.status, &response.body), (302, &Body::Empty));
    let location = format!("https://cdn.example/store/ab/{}.mp3", "ab".repeat(32));
    assert_eq!(header(&response, "location"), Some(location.as_str()));
    assert_eq!(
        header(&response, "cache-control"),
        Some("public, max-age=60")
    );
}

#[test]
fn requests_without_a_file_are_answered_at_once() {
    let routes = routes(r#", "cors": "*""#);
    let missing = answer(&routes, &get("/jp/canonical/none.mp3"), None);
    assert_eq!(
        (missing.status, &missing.body),
        (404, &Body::Text("not found\n"))
    );
    assert_eq!(header(&missing, "access-control-allow-origin"), Some("*"));
    let unrouted = answer(&routes, &get("/jp/canonical"), Some(&file(1)));
    assert_eq!(unrouted.status, 404);

    let post = Request {
        method: "POST",
        ..get("/jp/canonical/a.mp3")
    };
    let refused = answer(&routes, &post, Some(&file(1)));
    assert_eq!(refused.status, 405);
    assert_eq!(header(&refused, "allow"), Some("GET, HEAD, OPTIONS"));

    let options = Request {
        method: "OPTIONS",
        ..get("/jp/canonical/a.mp3")
    };
    let preflight = answer(&routes, &options, None);
    assert_eq!((preflight.status, &preflight.body), (204, &Body::Empty));
    assert_eq!(
        header(&preflight, "access-control-allow-headers"),
        Some("Range, If-None-Match, If-Range")
    );

    let failure = routes.failure();
    assert_eq!(failure.status, 500);
    assert_eq!(header(&failure, "cache-control"), Some("no-store"));
}

#[test]
fn cors_headers_are_only_sent_when_configured() {
    let routes = routes("");
    let response = answer(&routes, &get("/jp/canonical/a.mp3"), Some(&file(1)));
    assert_eq!(header(&response, "access-control-allow-origin"), None);
}
