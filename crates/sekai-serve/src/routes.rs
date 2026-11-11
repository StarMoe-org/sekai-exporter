//! The routes file: which URL paths name which files of which views.

use std::fmt;

use sekai_format::publication::is_name;
use serde::Deserialize;

/// How long a response for the current version may be cached, in seconds, unless the routes
/// file says otherwise.
const DEFAULT_MAX_AGE: u64 = 60;

/// The longest region, view or version a request can name, in bytes. A name is part of the file
/// name of a pointer, so a longer one names nothing.
const MAX_NAME_BYTES: usize = 200;

/// A file of a view that a request names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The region.
    pub region: String,
    /// The view.
    pub view: String,
    /// The published version, or `None` for the current one.
    pub version: Option<String>,
    /// The path of the file in the view.
    pub path: String,
}

impl Target {
    /// The path of the pointer that publishes the target, relative to the published root:
    /// `<region>/<view>/current.json`, or `<region>/<view>/versions/<version>.json`.
    pub fn pointer(&self) -> String {
        let Self { region, view, .. } = self;
        match &self.version {
            None => format!("{region}/{view}/current.json"),
            Some(version) => format!("{region}/{view}/versions/{version}.json"),
        }
    }
}

/// A routes file that is not valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutesError(String);

impl fmt::Display for RoutesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid routes: {}", self.0)
    }
}

impl std::error::Error for RoutesError {}

fn invalid(message: impl Into<String>) -> RoutesError {
    RoutesError(message.into())
}

/// How a file is delivered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Delivery {
    /// The response carries the file.
    Bytes,
    /// The response redirects to the object under this URL, which has no trailing `/`.
    Redirect(String),
}

/// The routes of a resolver, and how it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routes {
    routes: Vec<Route>,
    pub(crate) delivery: Delivery,
    pub(crate) max_age: u64,
    pub(crate) cors: Option<String>,
}

/// A placeholder that names a region, a view or a version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Region,
    View,
    Version,
}

/// A part of a segment of a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Name(Field),
}

/// A pattern and the names it fixes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Route {
    /// The segments before `{path}`.
    segments: Vec<Vec<Piece>>,
    region: Option<String>,
    view: Option<String>,
}

/// The names a request gives.
#[derive(Default)]
struct Captures<'a> {
    region: Option<&'a str>,
    view: Option<&'a str>,
    version: Option<&'a str>,
}

impl<'a> Captures<'a> {
    fn set(&mut self, field: Field, value: &'a str) {
        let slot = match field {
            Field::Region => &mut self.region,
            Field::View => &mut self.view,
            Field::Version => &mut self.version,
        };
        *slot = Some(value);
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RoutesFile {
    routes: Vec<RouteFile>,
    #[serde(default)]
    delivery: DeliveryName,
    store_url: Option<String>,
    #[serde(default = "default_max_age")]
    max_age: u64,
    cors: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RouteFile {
    pattern: String,
    region: Option<String>,
    view: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "lowercase")]
enum DeliveryName {
    #[default]
    Bytes,
    Redirect,
}

fn default_max_age() -> u64 {
    DEFAULT_MAX_AGE
}

/// Whether `text` can be the value of a response header.
pub(crate) fn is_header_value(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_graphic() || c == ' ')
}

impl Routes {
    /// Reads a routes file.
    pub fn from_json(bytes: &[u8]) -> Result<Self, RoutesError> {
        let file: RoutesFile = serde_json::from_slice(bytes).map_err(|e| invalid(e.to_string()))?;
        if file.routes.is_empty() {
            return Err(invalid("there are no routes"));
        }
        let routes = file
            .routes
            .into_iter()
            .map(Route::new)
            .collect::<Result<_, _>>()?;
        let delivery = match (file.delivery, file.store_url) {
            (DeliveryName::Bytes, None) => Delivery::Bytes,
            (DeliveryName::Redirect, Some(url)) if is_header_value(&url) => {
                Delivery::Redirect(url.trim_end_matches('/').to_owned())
            }
            (DeliveryName::Redirect, Some(url)) => {
                return Err(invalid(format!("{url:?} is not a store URL")));
            }
            (DeliveryName::Redirect, None) => {
                return Err(invalid("\"redirect\" needs a store_url"));
            }
            (DeliveryName::Bytes, Some(_)) => {
                return Err(invalid("store_url is only for \"redirect\""));
            }
        };
        if let Some(origin) = file.cors.as_deref().filter(|o| !is_header_value(o)) {
            return Err(invalid(format!("{origin:?} is not a CORS origin")));
        }
        Ok(Self {
            routes,
            delivery,
            max_age: file.max_age,
            cors: file.cors,
        })
    }

    /// How many seconds a response for the current version may be cached, which is also how
    /// often a host reads the pointer of a view again.
    pub fn max_age(&self) -> u64 {
        self.max_age
    }

    /// The file that the URL path `path` names, by the first route that matches it.
    pub fn resolve(&self, path: &str) -> Option<Target> {
        let rest = path.strip_prefix('/')?;
        self.routes.iter().find_map(|route| route.resolve(rest))
    }
}

impl Route {
    fn new(file: RouteFile) -> Result<Self, RoutesError> {
        let pattern = &file.pattern;
        let error = |message: &str| invalid(format!("pattern {pattern:?}: {message}"));
        let texts: Vec<&str> = pattern
            .strip_prefix('/')
            .ok_or_else(|| error("it does not start with /"))?
            .split('/')
            .collect();
        let Some((&"{path}", before)) = texts.split_last() else {
            return Err(error("its last segment is not {path}"));
        };
        let segments = before
            .iter()
            .map(|segment| pieces(segment).map_err(|message| error(&message)))
            .collect::<Result<Vec<_>, _>>()?;
        check_names(&segments, &file).map_err(|message| error(&message))?;
        Ok(Self {
            segments,
            region: file.region,
            view: file.view,
        })
    }

    /// The target that `rest`, a URL path without its leading `/`, names by this route.
    fn resolve(&self, mut rest: &str) -> Option<Target> {
        let mut captures = Captures::default();
        for pieces in &self.segments {
            let (segment, after) = rest.split_once('/')?;
            capture(pieces, segment, &mut captures)?;
            rest = after;
        }
        let path = percent_decode(rest).filter(|path| !path.is_empty())?;
        Some(Target {
            region: captures.region.or(self.region.as_deref())?.to_owned(),
            view: captures.view.or(self.view.as_deref())?.to_owned(),
            version: captures.version.map(str::to_owned),
            path,
        })
    }
}

/// Whether the placeholders of `segments` and the names that `file` fixes give each of the
/// region and the view once, and the version at most once.
fn check_names(segments: &[Vec<Piece>], file: &RouteFile) -> Result<(), String> {
    let fields: Vec<Field> = segments
        .iter()
        .flatten()
        .filter_map(|piece| match piece {
            Piece::Name(field) => Some(*field),
            Piece::Text(_) => None,
        })
        .collect();
    if (1..fields.len()).any(|at| fields[..at].contains(&fields[at])) {
        return Err("it has a placeholder twice".to_owned());
    }
    for (field, fixed, name) in [
        (Field::Region, &file.region, "region"),
        (Field::View, &file.view, "view"),
    ] {
        match (fields.contains(&field), fixed) {
            (true, None) => {}
            (false, Some(value)) if is_request_name(value) => {}
            (false, Some(value)) => return Err(format!("{value:?} is not a {name}")),
            (true, Some(_)) => return Err(format!("both it and the route give the {name}")),
            (false, None) => return Err(format!("neither it nor the route gives the {name}")),
        }
    }
    Ok(())
}

/// The pieces of one segment of a pattern before `{path}`.
fn pieces(segment: &str) -> Result<Vec<Piece>, String> {
    let mut pieces = Vec::new();
    let mut rest = segment;
    while !rest.is_empty() {
        let piece = match rest.strip_prefix('{') {
            Some(after) => {
                let (name, after) = after
                    .split_once('}')
                    .ok_or_else(|| format!("{segment:?} has an unclosed placeholder"))?;
                rest = after;
                Piece::Name(match name {
                    "region" => Field::Region,
                    "view" => Field::View,
                    "version" => Field::Version,
                    "path" => return Err("{path} is not the last segment".to_owned()),
                    other => return Err(format!("{{{other}}} is not a placeholder")),
                })
            }
            None => {
                let end = rest.find('{').unwrap_or(rest.len());
                let (text, after) = rest.split_at(end);
                if text.contains('}') {
                    return Err(format!("{segment:?} has an unopened placeholder"));
                }
                rest = after;
                Piece::Text(text.to_owned())
            }
        };
        if let (Some(Piece::Name(_)), Piece::Name(_)) = (pieces.last(), &piece) {
            return Err(format!(
                "{segment:?} has two placeholders with no text between"
            ));
        }
        pieces.push(piece);
    }
    if pieces.is_empty() {
        return Err("it has an empty segment".to_owned());
    }
    Ok(pieces)
}

/// Matches `segment` of a URL path against `pieces`, recording the names it gives. A placeholder
/// takes the characters up to the first occurrence of the text that follows it, and they must
/// make a name.
fn capture<'a>(pieces: &[Piece], mut segment: &'a str, captures: &mut Captures<'a>) -> Option<()> {
    for (at, piece) in pieces.iter().enumerate() {
        match piece {
            Piece::Text(text) => segment = segment.strip_prefix(text.as_str())?,
            Piece::Name(field) => {
                let end = match pieces.get(at + 1) {
                    Some(Piece::Text(next)) => segment.find(next.as_str())?,
                    _ => segment.len(),
                };
                let (value, after) = segment.split_at(end);
                if !is_request_name(value) {
                    return None;
                }
                captures.set(*field, value);
                segment = after;
            }
        }
    }
    segment.is_empty().then_some(())
}

/// Whether `text` can name a region, a view or a version in a request: a name that is not too
/// long to be part of a file name, and that no host takes for a device.
fn is_request_name(text: &str) -> bool {
    text.len() <= MAX_NAME_BYTES && is_name(text) && !is_device_name(text)
}

/// Whether Windows takes a file named `name` for a device, which it does whatever follows the
/// first dot: `con`, `prn`, `aux`, `nul`, and `com` or `lpt` with a digit, in either case. A
/// host there would open the device where it means to open a pointer.
fn is_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or(name);
    match stem.to_ascii_lowercase().as_bytes() {
        b"con" | b"prn" | b"aux" | b"nul" => true,
        [b'c', b'o', b'm', digit] | [b'l', b'p', b't', digit] => digit.is_ascii_digit(),
        _ => false,
    }
}

/// `text` with every `%XX` replaced by the byte it encodes, if that makes UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let mut bytes = text.bytes();
    let mut decoded = Vec::with_capacity(text.len());
    while let Some(byte) = bytes.next() {
        decoded.push(match byte {
            b'%' => hex_digit(bytes.next()?)? << 4 | hex_digit(bytes.next()?)?,
            byte => byte,
        });
    }
    String::from_utf8(decoded).ok()
}

fn hex_digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}
