//! The HTTP/1.1 the servers speak: one request per connection, closed after the response.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};

/// A request as a server received it.
pub(crate) struct Request {
    pub(crate) method: String,
    /// The path, still percent-encoded.
    pub(crate) path: String,
    /// The query parameters, decoded, in name order.
    pub(crate) query: BTreeMap<String, String>,
    /// The headers, by lowercase name.
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Vec<u8>,
}

/// A response: its status, extra header lines, each ending in CRLF, and its body.
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) headers: String,
    pub(crate) body: Vec<u8>,
    /// Whether the connection is closed halfway through the body, as when it drops.
    pub(crate) cut: bool,
}

impl Response {
    pub(crate) fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            headers: String::new(),
            body: body.into(),
            cut: false,
        }
    }
}

/// Listens on a free local port and answers each request with `answer` on its own thread.
/// Returns `host:port`.
pub(crate) fn listen(answer: impl Fn(&Request) -> Response + Send + Sync + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let answer = std::sync::Arc::new(answer);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let answer = answer.clone();
            std::thread::spawn(move || respond(stream, answer.as_ref()));
        }
    });
    address
}

fn respond(mut stream: TcpStream, answer: &dyn Fn(&Request) -> Response) {
    let Some(request) = read_request(&stream) else {
        return;
    };
    let response = answer(&request);
    let mut bytes = format!(
        "HTTP/1.1 {} X\r\ncontent-length: {}\r\n{}connection: close\r\n\r\n",
        response.status,
        response.body.len(),
        response.headers
    )
    .into_bytes();
    if request.method != "HEAD" {
        let sent = match response.cut {
            true => response.body.len() / 2,
            false => response.body.len(),
        };
        bytes.extend_from_slice(&response.body[..sent]);
    }
    let _ = stream.write_all(&bytes);
}

fn read_request(stream: &TcpStream) -> Option<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_owned();
    let target = parts.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let path = path.to_owned();
    let query = query
        .split('&')
        .filter(|parameter| !parameter.is_empty())
        .filter_map(|parameter| parameter.split_once('='))
        .map(|(name, value)| (decode(name), decode(value)))
        .collect();
    let mut headers = BTreeMap::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).ok()?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let length: usize = headers
        .get("content-length")
        .map_or(Some(0), |value| value.parse().ok())?;
    let mut body = vec![0; length];
    reader.read_exact(&mut body).ok()?;
    Some(Request {
        method,
        path,
        query,
        headers,
        body,
    })
}

/// `text` with every `%XX` replaced by its byte.
fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| text.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
