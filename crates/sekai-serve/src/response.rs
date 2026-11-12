//! Responses: what a request is answered with, given the file it names.

use std::fmt;

use sekai_format::paths::ChunkFile;
use sekai_format::publication::is_name;

use crate::routes::{Delivery, Routes, Target, is_header_value};

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
    /// The headers, with lowercase names. `content-length` is among them whenever the response
    /// to `GET` has a body, and then in the response to `HEAD` as well, which has none.
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

/// The name of an object of the store: the SHA-256 of its bytes and its file extension.
///
/// The extension is a name, ASCII letters, digits, `.`, `_` and `-` and not only dots, so the
/// path of an object never leaves the store, whatever an index says.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ObjectId {
    sha256: [u8; 32],
    extension: String,
}

impl ObjectId {
    /// The object with the hash `sha256` and `extension`, if `extension` is a name.
    pub fn new(sha256: [u8; 32], extension: &str) -> Option<Self> {
        is_name(extension).then(|| Self {
            sha256,
            extension: extension.to_owned(),
        })
    }

    /// The root of a path index, `<aa>/<sha256>.json`.
    pub(crate) fn root(sha256: [u8; 32]) -> Self {
        Self {
            sha256,
            extension: "json".to_owned(),
        }
    }

    /// A chunk of a path index, `<aa>/<sha256>.paths`.
    pub(crate) fn chunk(sha256: [u8; 32]) -> Self {
        Self {
            sha256,
            extension: "paths".to_owned(),
        }
    }

    /// The SHA-256 of the bytes of the object.
    pub fn sha256(&self) -> &[u8; 32] {
        &self.sha256
    }

    /// The file extension, without the dot.
    pub fn extension(&self) -> &str {
        &self.extension
    }

    /// The path of the object relative to the store, `<aa>/<sha256>.<extension>`.
    pub fn path(&self) -> String {
        let hex = hex(&self.sha256);
        format!("{}/{hex}.{}", &hex[..2], self.extension)
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.path())
    }
}

/// A byte range of an object of the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slice {
    /// The object.
    pub object: ObjectId,
    /// The first byte.
    pub start: u64,
    /// The number of bytes.
    pub length: u64,
    /// The size of the whole object, as its index gives it. An object of another size is not the
    /// one the index describes, and a host does not send it.
    pub size: u64,
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
    /// The response to `request` for `target`, given the file the target names, if the view
    /// has it. Fails with the reason when the index describes the file with an extension or a
    /// media type that cannot be sent.
    pub(crate) fn respond(
        &self,
        request: &Request<'_>,
        target: &Target,
        file: Option<ChunkFile<'_>>,
    ) -> Result<Response, String> {
        let Some(file) = file else {
            return Ok(self.not_found());
        };
        let object = ObjectId::new(file.sha256, file.extension)
            .ok_or_else(|| format!("{:?} is not a file extension", file.extension))?;
        if !is_header_value(file.media_type) {
            return Err(format!("{:?} is not a media type", file.media_type));
        }
        // A named version is cached no longer than the current one: its name can be published
        // again with other files, and the resolver answers with them within `max_age`.
        let cache_control = format!("public, max-age={}", self.max_age);
        Ok(match &self.delivery {
            Delivery::Bytes => self.file(request, &target.path, file, object, cache_control),
            Delivery::Redirect(store_url) => {
                let location = format!("{store_url}/{object}");
                let headers = vec![("location", location), ("cache-control", cache_control)];
                self.finish(302, headers, Body::Empty)
            }
        })
    }

    /// The response when the publication cannot be read.
    pub(crate) fn failure(&self) -> Response {
        let headers = vec![("cache-control", "no-store".to_owned())];
        self.reply(500, headers, "the file could not be read\n")
    }

    /// The response to a path that no route matches or that the view does not have.
    pub(crate) fn not_found(&self) -> Response {
        let headers = vec![("cache-control", format!("public, max-age={}", self.max_age))];
        self.reply(404, headers, "not found\n")
    }

    /// The response to a method the resolver does not answer.
    pub(crate) fn not_allowed(&self) -> Response {
        let headers = vec![("allow", ALLOW.to_owned())];
        self.reply(405, headers, "method not allowed\n")
    }

    /// The response to a URL path that is longer than any path the resolver looks at.
    pub(crate) fn too_long(&self) -> Response {
        self.reply(414, Vec::new(), "URI too long\n")
    }

    /// The response to `OPTIONS`, which is also the answer to a cross-origin preflight.
    pub(crate) fn options(&self) -> Response {
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
        file: ChunkFile<'_>,
        object: ObjectId,
        cache_control: String,
    ) -> Response {
        let etag = format!("\"{}\"", hex(&file.sha256));
        let mut headers = vec![("etag", etag.clone()), ("cache-control", cache_control)];
        if request.if_none_match.is_some_and(|tags| lists(tags, &etag)) {
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
            ("content-type", file.media_type.to_owned()),
            ("content-length", length.to_string()),
            ("accept-ranges", "bytes".to_owned()),
            ("content-disposition", disposition(path)),
        ]);
        let body = Body::Object(Slice {
            object,
            start,
            length,
            size: file.size,
        });
        self.finish(status, headers, body)
    }

    /// A response with a short text. It gives the length of the text, as a response with a file
    /// gives the length of the file, so the answer to `HEAD` has the headers that the answer to
    /// `GET` has.
    fn reply(
        &self,
        status: u16,
        mut headers: Vec<(&'static str, String)>,
        text: &'static str,
    ) -> Response {
        headers.extend([
            ("content-type", "text/plain; charset=utf-8".to_owned()),
            ("content-length", text.len().to_string()),
        ]);
        self.finish(status, headers, Body::Text(text))
    }

    /// A response with the headers every response has: the instruction not to guess another
    /// media type than the one given, and the CORS headers when the routes allow other origins.
    fn finish(
        &self,
        status: u16,
        mut headers: Vec<(&'static str, String)>,
        body: Body,
    ) -> Response {
        headers.push(("x-content-type-options", "nosniff".to_owned()));
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

impl Response {
    /// The response to a `HEAD` request: the same status and headers, and no body.
    pub(crate) fn without_body(mut self) -> Self {
        self.body = Body::Empty;
        self
    }
}

/// Whether an `If-None-Match` header lists `etag`, by weak comparison.
fn lists(tags: &str, etag: &str) -> bool {
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

/// `bytes` as lowercase hexadecimal digits.
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
