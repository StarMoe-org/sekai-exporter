//! Responses: what a request is answered with, given the file it names.

use sekai_format::paths::PathFile;

use crate::routes::{Delivery, Routes, Target};

/// How long a response for a named version may be cached: a year, the longest that caches keep.
const PINNED_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// The methods the resolver answers.
const ALLOW: &str = "GET, HEAD, OPTIONS";

/// The request headers a cross-origin client may send.
const ALLOW_HEADERS: &str = "Range, If-None-Match, If-Range";

/// The response headers a cross-origin client may read.
const EXPOSE_HEADERS: &str = "Accept-Ranges, Content-Disposition, Content-Range, ETag";

/// How long a client may keep the answer to a cross-origin preflight, in seconds.
const PREFLIGHT_MAX_AGE: &str = "86400";

/// The parts of an HTTP request that decide its response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request<'a> {
    /// The method.
    pub method: &'a str,
    /// The path of the URL, without its query.
    pub path: &'a str,
    /// The `Range` header.
    pub range: Option<&'a str>,
    /// The `If-None-Match` header.
    pub if_none_match: Option<&'a str>,
    /// The `If-Range` header.
    pub if_range: Option<&'a str>,
}

/// A response for a host to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The headers, with lowercase names.
    pub headers: Vec<(&'static str, String)>,
    /// The body.
    pub body: Body,
}

/// The body of a response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Body {
    /// No body.
    Empty,
    /// A short text.
    Text(&'static str),
    /// A byte range of an object of the store.
    Object(Slice),
}

/// A byte range of an object of the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    /// The SHA-256 of the object.
    pub sha256: [u8; 32],
    /// The extension of the object.
    pub extension: String,
    /// The first byte.
    pub start: u64,
    /// The number of bytes.
    pub length: u64,
}

impl Slice {
    /// The name of the object relative to the store, `<aa>/<sha256>.<extension>`.
    pub fn object(&self) -> String {
        object_name(&self.sha256, &self.extension)
    }
}

/// What a host does with a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Routing {
    /// Look the target up and answer with [`Routes::respond`].
    Find(Target),
    /// Answer with this response.
    Reply(Response),
}

/// The range of a file that a `Range` header asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Range {
    /// The whole file: no range, several, or one that is not well formed.
    Whole,
    /// The bytes from `first` to `last`, both included.
    Part { first: u64, last: u64 },
    /// A range that the file does not have.
    Unsatisfiable,
}

impl Routes {
    /// What to do with `request`: look up the file it names, or answer it at once.
    pub fn route(&self, request: &Request<'_>) -> Routing {
        match request.method {
            "GET" | "HEAD" => match self.resolve(request.path) {
                Some(target) => Routing::Find(target),
                None => Routing::Reply(self.not_found()),
            },
            "OPTIONS" => Routing::Reply(self.options()),
            _ => Routing::Reply(self.not_allowed()),
        }
    }

    /// The response to `request` for `target`, given the file the target names, if the view
    /// has it.
    pub fn respond(
        &self,
        request: &Request<'_>,
        target: &Target,
        file: Option<&PathFile>,
    ) -> Response {
        let Some(file) = file else {
            return self.not_found();
        };
        let cache_control = match target.version {
            Some(_) => PINNED_CACHE_CONTROL.to_owned(),
            None => format!("public, max-age={}", self.max_age),
        };
        match &self.delivery {
            Delivery::Bytes => self.file(request, &target.path, file, cache_control),
            Delivery::Redirect(store_url) => {
                let location =
                    format!("{store_url}/{}", object_name(&file.sha256, &file.extension));
                let headers = vec![("location", location), ("cache-control", cache_control)];
                self.finish(302, headers, Body::Empty)
            }
        }
    }

    /// The response when a file that a view has cannot be read.
    pub fn failure(&self) -> Response {
        let headers = vec![("cache-control", "no-store".to_owned())];
        self.reply(500, headers, "the file could not be read\n")
    }

    fn not_found(&self) -> Response {
        let headers = vec![("cache-control", format!("public, max-age={}", self.max_age))];
        self.reply(404, headers, "not found\n")
    }

    fn not_allowed(&self) -> Response {
        let headers = vec![("allow", ALLOW.to_owned())];
        self.reply(405, headers, "method not allowed\n")
    }

    fn options(&self) -> Response {
        let mut headers = vec![("allow", ALLOW.to_owned())];
        if self.cors.is_some() {
            headers.extend([
                ("access-control-allow-methods", ALLOW.to_owned()),
                ("access-control-allow-headers", ALLOW_HEADERS.to_owned()),
                ("access-control-max-age", PREFLIGHT_MAX_AGE.to_owned()),
            ]);
        }
        self.finish(204, headers, Body::Empty)
    }

    /// The response that carries `file`, the file at `path`, or the range of it that the request
    /// asks for.
    fn file(
        &self,
        request: &Request<'_>,
        path: &str,
        file: &PathFile,
        cache_control: String,
    ) -> Response {
        let etag = format!("\"{}\"", hex(&file.sha256));
        let mut headers = vec![("etag", etag.clone()), ("cache-control", cache_control)];
        if request.if_none_match.is_some_and(|tags| names(tags, &etag)) {
            return self.finish(304, headers, Body::Empty);
        }
        let range = match request.range {
            Some(range) if request.if_range.is_none_or(|tag| tag == etag) => {
                parse_range(range, file.size)
            }
            _ => Range::Whole,
        };
        let (status, start, length) = match range {
            Range::Whole => (200, 0, file.size),
            Range::Part { first, last } => {
                headers.push((
                    "content-range",
                    format!("bytes {first}-{last}/{}", file.size),
                ));
                (206, first, last - first + 1)
            }
            Range::Unsatisfiable => {
                headers.push(("content-range", format!("bytes */{}", file.size)));
                return self.reply(416, headers, "range not satisfiable\n");
            }
        };
        headers.extend([
            ("content-type", file.media_type.clone()),
            ("content-length", length.to_string()),
            ("accept-ranges", "bytes".to_owned()),
            ("content-disposition", disposition(path)),
        ]);
        let body = match request.method {
            "HEAD" => Body::Empty,
            _ => Body::Object(Slice {
                sha256: file.sha256,
                extension: file.extension.clone(),
                start,
                length,
            }),
        };
        self.finish(status, headers, body)
    }

    /// A response with a short text.
    fn reply(
        &self,
        status: u16,
        mut headers: Vec<(&'static str, String)>,
        text: &'static str,
    ) -> Response {
        headers.push(("content-type", "text/plain; charset=utf-8".to_owned()));
        self.finish(status, headers, Body::Text(text))
    }

    /// A response with the CORS headers, when the routes allow other origins.
    fn finish(
        &self,
        status: u16,
        mut headers: Vec<(&'static str, String)>,
        body: Body,
    ) -> Response {
        if let Some(origin) = &self.cors {
            headers.extend([
                ("access-control-allow-origin", origin.clone()),
                ("access-control-expose-headers", EXPOSE_HEADERS.to_owned()),
            ]);
        }
        Response {
            status,
            headers,
            body,
        }
    }
}

/// Whether an `If-None-Match` header names `etag`, by weak comparison.
fn names(tags: &str, etag: &str) -> bool {
    tags.split(',')
        .map(str::trim)
        .any(|tag| tag == "*" || tag.strip_prefix("W/").unwrap_or(tag) == etag)
}

/// The range of a file of `size` bytes that a `Range` header asks for.
fn parse_range(header: &str, size: u64) -> Range {
    let spec = match header.trim().strip_prefix("bytes=") {
        Some(spec) if !spec.contains(',') => spec.trim(),
        _ => return Range::Whole,
    };
    let Some((first, last)) = spec.split_once('-') else {
        return Range::Whole;
    };
    let number = |text: &str| {
        Some(text)
            .filter(|text| text.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|text| text.parse::<u64>().ok())
    };
    if first.is_empty() {
        // The last bytes of the file.
        return match number(last).map(|length| length.min(size)) {
            None => Range::Whole,
            Some(0) => Range::Unsatisfiable,
            Some(length) => Range::Part {
                first: size - length,
                last: size - 1,
            },
        };
    }
    let Some(first) = number(first) else {
        return Range::Whole;
    };
    let last = match (last, number(last)) {
        ("", _) => u64::MAX,
        (_, Some(last)) if last >= first => last,
        _ => return Range::Whole,
    };
    if first < size {
        Range::Part {
            first,
            last: last.min(size - 1),
        }
    } else {
        Range::Unsatisfiable
    }
}

/// The `Content-Disposition` of the file at `path`: shown inline, saved under its own name.
fn disposition(path: &str) -> String {
    let name = path.rsplit_once('/').map_or(path, |(_, name)| name);
    let plain = |c: char| c == ' ' || (c.is_ascii_graphic() && c != '"' && c != '\\');
    if name.chars().all(plain) {
        return format!("inline; filename=\"{name}\"");
    }
    let fallback: String = name
        .chars()
        .map(|c| if plain(c) { c } else { '_' })
        .collect();
    let attribute = |byte: u8| byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte);
    let encoded: String = name
        .bytes()
        .map(|byte| {
            if attribute(byte) {
                char::from(byte).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect();
    format!("inline; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

/// The name of an object relative to the store.
fn object_name(sha256: &[u8; 32], extension: &str) -> String {
    let hex = hex(sha256);
    format!("{}/{hex}.{extension}", &hex[..2])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
