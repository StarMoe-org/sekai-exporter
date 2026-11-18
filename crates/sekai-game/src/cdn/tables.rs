//! The game data tables of a region.

use std::path::PathBuf;

use serde::Deserialize;

use super::SourceError;
use super::dir::{hash_files, read_error};
use super::http::{Headers, Http};
use super::region::{Network, render};

/// Where the game data tables of a region come from.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum TableSource {
    /// A repository of table dumps, read over HTTP.
    Git {
        /// The address of the current revision: a JSON document that gives it, or the ref
        /// advertisement of the repository from its Git service
        /// (`<repository>/info/refs?service=git-upload-pack`), whose `HEAD` it is.
        revision: String,
        /// The field of the JSON document that holds the revision.
        #[serde(default = "default_revision_field")]
        revision_field: String,
        /// The address of a table at a revision, with `{revision}` and `{name}`.
        table: String,
    },
    /// Files named `<table>.json` in a local directory.
    Dir {
        /// The directory.
        directory: PathBuf,
    },
}

fn default_revision_field() -> String {
    "sha".to_owned()
}

/// The tables of one revision.
pub struct Tables {
    origin: Origin,
    revision: String,
}

enum Origin {
    Http { http: Http, template: String },
    Dir(PathBuf),
}

impl Tables {
    /// Opens `source` at its current revision.
    pub fn open(source: &TableSource, network: &Network) -> Result<Self, SourceError> {
        match source {
            TableSource::Git {
                revision,
                revision_field,
                table,
            } => {
                let http = Http::new(network)?;
                let current = current_revision(&http, revision, revision_field)?;
                Ok(Self {
                    origin: Origin::Http {
                        http,
                        template: table.clone(),
                    },
                    revision: current,
                })
            }
            TableSource::Dir { directory } => Ok(Self {
                revision: hash_files(directory)?,
                origin: Origin::Dir(directory.clone()),
            }),
        }
    }

    /// The revision of the tables.
    pub fn revision(&self) -> &str {
        &self.revision
    }

    /// The JSON bytes of the table `name`.
    pub fn read(&self, name: &str) -> Result<Vec<u8>, SourceError> {
        match &self.origin {
            Origin::Http { http, template } => {
                let url = render(template, &[("revision", &self.revision), ("name", name)]);
                http.bytes(&url, &user_agent())
            }
            Origin::Dir(directory) => {
                let path = directory.join(format!("{name}.json"));
                std::fs::read(&path).map_err(|e| read_error(&path, e))
            }
        }
    }
}

/// Repository hosts refuse requests that do not name a client.
fn user_agent() -> Headers {
    vec![("user-agent", "sekai-exporter".to_owned())]
}

/// The revision that the document at `url` gives: the commit of `HEAD` in a ref advertisement, or
/// else the string at `field` of a JSON document.
fn current_revision(http: &Http, url: &str, field: &str) -> Result<String, SourceError> {
    let body = http.bytes(url, &user_agent())?;
    if is_ref_advertisement(&body) {
        let head = advertised_head(&body).map(str::to_owned);
        return head.ok_or_else(|| SourceError::new("decode", format!("{url}: no HEAD")));
    }
    let document: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| SourceError::new("decode", format!("{url}: {e}")))?;
    document
        .get(field)
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| SourceError::new("decode", format!("{url}: no field {field}")))
}

/// Whether `body` is the ref advertisement of a Git service over smart HTTP, which begins with the
/// pkt-line `# service=git-upload-pack`: four hexadecimal digits, then `#`.
fn is_ref_advertisement(body: &[u8]) -> bool {
    body.len() > 4 && body[..4].iter().all(u8::is_ascii_hexdigit) && body[4] == b'#'
}

/// The object that `HEAD` names in the ref advertisement `body`, a sequence of pkt-lines: four
/// hexadecimal digits giving the length of the line with them, or `0000` between sections. A ref
/// line is `<object> <name>`, and the first one carries the capabilities of the service after a
/// NUL.
fn advertised_head(body: &[u8]) -> Option<&str> {
    let mut rest = body;
    while let Some(digits) = rest.get(..4) {
        let length = usize::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()?;
        if length == 0 {
            rest = &rest[4..];
            continue;
        }
        let line = rest.get(4..length)?;
        rest = &rest[length..];
        let reference = line.split(|&byte| byte == 0).next()?;
        let reference = std::str::from_utf8(reference).ok()?.trim_end();
        if let Some((object, "HEAD")) = reference.split_once(' ') {
            return Some(object);
        }
    }
    None
}
