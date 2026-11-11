//! `sekai-exporter serve`: an HTTP server that resolves the paths of the views published in an
//! output directory, as described in `docs/serving.md`.

use std::collections::HashMap;
use std::convert::Infallible;
use std::ffi::OsString;
use std::io::{self, SeekFrom};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use bytes::Bytes;
use futures_util::TryStreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full, StreamBody};
use hyper::StatusCode;
use hyper::body::{Frame, Incoming};
use hyper::header::{HeaderName, IF_NONE_MATCH, IF_RANGE, RANGE};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use sekai_catalog::store::parse_hex;
use sekai_catalog::{Key, LocalStore};
use sekai_format::paths::{Chunk, PathFile, PathIndex};
use sekai_format::publication::Pointer;
use sekai_serve::{Body, Request, Response, Routes, Routing, Slice, Target};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio::net::TcpListener;
use tokio_util::io::ReaderStream;

/// The address the server listens on unless `--listen` says otherwise.
const DEFAULT_LISTEN: SocketAddr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080));

/// How many bytes of a file are read at a time.
const READ_SIZE: usize = 256 << 10;

type ResponseBody = BoxBody<Bytes, io::Error>;

struct Options {
    output: PathBuf,
    routes: PathBuf,
    listen: SocketAddr,
    threads: usize,
}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Options, lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let mut output = None;
    let mut routes = None;
    let mut listen = DEFAULT_LISTEN;
    let mut threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    while let Some(arg) = parser.next()? {
        match arg {
            Long("routes") => routes = Some(PathBuf::from(parser.value()?)),
            Long("listen") => listen = parser.value()?.parse()?,
            Long("threads") => threads = parser.value()?.parse()?,
            Value(v) if output.is_none() => output = Some(PathBuf::from(v)),
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(Options {
        output: output.ok_or("expected <output>")?,
        routes: routes.ok_or("expected --routes <file>")?,
        listen,
        threads: threads.max(1),
    })
}

pub(crate) fn run(args: impl Iterator<Item = OsString>) -> Result<bool, lexopt::Error> {
    let options = parse(args)?;
    let routes =
        read_routes(&options.routes).map_err(|e| format!("{}: {e}", options.routes.display()))?;
    let refresh = Duration::from_secs(routes.max_age());
    let views = Views::open(&options.output, refresh)
        .map_err(|e| format!("{}: {e}", options.output.display()))?;
    let server = Arc::new(Server { routes, views });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(options.threads)
        .enable_io()
        .build()
        .map_err(|e| e.to_string())?;
    runtime
        .block_on(listen(server, options.listen))
        .map_err(|e| format!("{}: {e}", options.listen))?;
    Ok(true)
}

fn read_routes(path: &Path) -> Result<Routes, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Routes::from_json(&bytes).map_err(|e| e.to_string())
}

/// Accepts connections on `address` and answers their requests until the process ends.
async fn listen(server: Arc<Server>, address: SocketAddr) -> io::Result<()> {
    let listener = TcpListener::bind(address).await?;
    eprintln!("serving on http://{}", listener.local_addr()?);
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _)) => stream,
            Err(error) => {
                eprintln!("a connection could not be accepted: {error}");
                continue;
            }
        };
        let server = Arc::clone(&server);
        tokio::spawn(async move {
            let service = service_fn(|request| answer(&server, request));
            // A client that goes away mid-response ends its connection; nothing is left to do.
            let _ = http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .await;
        });
    }
}

/// The routes and the views they resolve in.
struct Server {
    routes: Routes,
    views: Views,
}

/// The response to `incoming`.
async fn answer(
    server: &Server,
    incoming: hyper::Request<Incoming>,
) -> Result<hyper::Response<ResponseBody>, Infallible> {
    let header = |name: HeaderName| incoming.headers().get(name).and_then(|v| v.to_str().ok());
    let request = Request {
        method: incoming.method().as_str(),
        path: incoming.uri().path(),
        range: header(RANGE),
        if_none_match: header(IF_NONE_MATCH),
        if_range: header(IF_RANGE),
    };
    let response = match server.routes.route(&request) {
        Routing::Reply(response) => response,
        Routing::Find(target) => match tokio::task::block_in_place(|| server.views.find(&target)) {
            Ok(file) => server.routes.respond(&request, &target, file.as_ref()),
            Err(error) => {
                eprintln!("{}: {error}", target.pointer());
                server.routes.failure()
            }
        },
    };
    let body = match &response.body {
        Body::Object(slice) => server.views.object(slice).await,
        fixed => Ok(fixed_body(fixed)),
    };
    Ok(match body {
        Ok(body) => write(response, body),
        Err(error) => {
            eprintln!("{}: {error}", request.path);
            let failure = server.routes.failure();
            let body = fixed_body(&failure.body);
            write(failure, body)
        }
    })
}

/// The HTTP response for `response` with `body`.
fn write(response: Response, body: ResponseBody) -> hyper::Response<ResponseBody> {
    let mut builder = hyper::Response::builder().status(response.status);
    for (name, value) in &response.headers {
        builder = builder.header(*name, value.as_str());
    }
    builder.body(body).unwrap_or_else(|error| {
        eprintln!("a response could not be written: {error}");
        let mut failure = hyper::Response::new(fixed_body(&Body::Empty));
        *failure.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        failure
    })
}

/// The body of a response that carries no object.
fn fixed_body(body: &Body) -> ResponseBody {
    match body {
        Body::Text(text) => Full::new(Bytes::from_static(text.as_bytes()))
            .map_err(never)
            .boxed(),
        Body::Empty | Body::Object(_) => Empty::new().map_err(never).boxed(),
    }
}

fn never(never: Infallible) -> io::Error {
    match never {}
}

/// The published views of an output directory, with the path indexes that their pointers name.
struct Views {
    published: PathBuf,
    store: LocalStore,
    /// How long a pointer is used before it is read again.
    refresh: Duration,
    /// The last reading of each pointer, by its path relative to `published`.
    pointers: Mutex<HashMap<String, Reading>>,
}

/// What a pointer named when it was last read.
struct Reading {
    at: Instant,
    index: Option<Arc<Index>>,
}

impl Reading {
    /// Whether the reading still holds for `target`: the pointer of a named version does not
    /// change once it exists, and the pointer of the current version is read again once the
    /// reading is older than `refresh`.
    fn holds(&self, target: &Target, refresh: Duration) -> bool {
        let lasting = target.version.is_some() && self.index.is_some();
        lasting || self.at.elapsed() < refresh
    }
}

/// A path index, with each chunk decoded on first use.
struct Index {
    /// The SHA-256 of the root, as named by the pointer.
    root: String,
    paths: PathIndex,
    chunks: Vec<OnceLock<Chunk>>,
}

impl Index {
    /// The chunk at `at`, read from `store` and decoded on first use.
    fn chunk(&self, at: usize, store: &LocalStore) -> io::Result<&Chunk> {
        if let Some(chunk) = self.chunks[at].get() {
            return Ok(chunk);
        }
        let object = store.read(&key(&self.paths.chunks[at].sha256, "paths")?)?;
        let chunk = Chunk::decode(&object).map_err(invalid_data)?;
        Ok(self.chunks[at].get_or_init(|| chunk))
    }
}

impl Views {
    fn open(output: &Path, refresh: Duration) -> io::Result<Self> {
        Ok(Self {
            published: output.join("published"),
            store: LocalStore::open(output.join("store"))?,
            refresh,
            pointers: Mutex::new(HashMap::new()),
        })
    }

    /// The file that `target` names, if its view has one.
    fn find(&self, target: &Target) -> io::Result<Option<PathFile>> {
        let Some(index) = self.index(target)? else {
            return Ok(None);
        };
        let Some(at) = index.paths.chunk_of(&target.path) else {
            return Ok(None);
        };
        Ok(index.chunk(at, &self.store)?.get(&target.path).cloned())
    }

    /// The path index of the publication that `target` is in, from the last reading of its
    /// pointer while that holds.
    fn index(&self, target: &Target) -> io::Result<Option<Arc<Index>>> {
        let pointer = target.pointer();
        let previous = match self.pointers.lock().unwrap().get(&pointer) {
            Some(reading) if reading.holds(target, self.refresh) => {
                return Ok(reading.index.clone());
            }
            Some(reading) => reading.index.clone(),
            None => None,
        };
        let index = self.read(&pointer, previous)?;
        let reading = Reading {
            at: Instant::now(),
            index: index.clone(),
        };
        self.pointers.lock().unwrap().insert(pointer, reading);
        Ok(index)
    }

    /// Reads the pointer at `pointer` and the path index it names, keeping `previous` when the
    /// pointer still names it.
    fn read(&self, pointer: &str, previous: Option<Arc<Index>>) -> io::Result<Option<Arc<Index>>> {
        let bytes = match std::fs::read(self.published.join(pointer)) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let pointer: Pointer = serde_json::from_slice(&bytes).map_err(invalid_data)?;
        let Some(root) = pointer.paths else {
            return Ok(None);
        };
        if let Some(previous) = previous.filter(|index| index.root == root) {
            return Ok(Some(previous));
        }
        let paths =
            PathIndex::from_bytes(&self.store.read(&key(&root, "json")?)?).map_err(invalid_data)?;
        let chunks = paths.chunks.iter().map(|_| OnceLock::new()).collect();
        Ok(Some(Arc::new(Index {
            root,
            paths,
            chunks,
        })))
    }

    /// The bytes of `slice`, read from the store as they are sent.
    async fn object(&self, slice: &Slice) -> io::Result<ResponseBody> {
        let key = Key {
            sha256: slice.sha256,
            extension: slice.extension.clone(),
        };
        let mut file = tokio::fs::File::open(self.store.path(&key)).await?;
        file.seek(SeekFrom::Start(slice.start)).await?;
        let stream = ReaderStream::with_capacity(file.take(slice.length), READ_SIZE);
        Ok(StreamBody::new(stream.map_ok(Frame::data)).boxed())
    }
}

/// The key of the object `<sha256>.<extension>`.
fn key(sha256: &str, extension: &str) -> io::Result<Key> {
    let sha256 =
        parse_hex(sha256).ok_or_else(|| invalid_data(format!("{sha256:?} is not a SHA-256")))?;
    Ok(Key {
        sha256,
        extension: extension.to_owned(),
    })
}

fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}
