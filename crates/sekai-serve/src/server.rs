//! The HTTP/1.1 server that answers requests from an output directory: the host of the resolver
//! behind `sekai-exporter serve`, described in `docs/serving.md`.
//!
//! The server only reads: pointers below `<output>/published` and objects below
//! `<output>/store`. It holds a bounded number of connections, each for a bounded time without
//! progress, and the resolver it hosts remembers a bounded amount.

use std::convert::Infallible;
use std::fmt;
use std::io::{self, SeekFrom, Write as _};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
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
use hyper_util::rt::{TokioIo, TokioTimer};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::time::Sleep;
use tokio_util::io::ReaderStream;

use crate::{Body, Fetched, Limits, Lookup, Read, Request, Resolver, Response, Routes, Slice};

/// The largest head of a request, in bytes: its request line and headers.
const HEAD_BYTES: usize = 16 << 10;

/// How many bytes of a file are read at a time.
const READ_SIZE: usize = 64 << 10;

/// How long the server waits before it accepts again after accepting failed for a reason that
/// lasts, as it does when the process has no file descriptor left.
const ACCEPT_PAUSE: Duration = Duration::from_millis(100);

type ResponseBody = BoxBody<Bytes, io::Error>;

/// How a server listens and what it allows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// The address to listen on.
    pub listen: SocketAddr,
    /// The number of threads that answer requests.
    pub threads: usize,
    /// How many connections are held at once. Further ones wait until one of them closes.
    pub connections: usize,
    /// How long a client may take to send the head of a request, which is also how long a
    /// connection is kept for its next request.
    pub head_timeout: Duration,
    /// How long a connection may go without a byte read or written before it is closed.
    pub idle_timeout: Duration,
    /// How much the resolver remembers.
    pub limits: Limits,
}

impl Default for Options {
    /// `127.0.0.1:8080`, a thread for each logical CPU, 256 connections, 30 seconds for the
    /// head of a request, 60 seconds without progress, and the default [`Limits`].
    fn default() -> Self {
        Self {
            listen: SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 8080)),
            threads: std::thread::available_parallelism().map_or(1, |n| n.get()),
            connections: 256,
            head_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(60),
            limits: Limits::default(),
        }
    }
}

/// A server that is bound to its address and has not started to answer.
#[derive(Debug)]
pub struct Server {
    listener: std::net::TcpListener,
    host: Arc<Host>,
    options: Options,
}

impl Server {
    /// Binds the address of `options` to answer by `routes` for the views published in
    /// `output`, the output directory of `export` or `update`. Views may be published there
    /// later; the directory itself has to exist.
    pub fn bind(output: &Path, routes: Routes, options: Options) -> io::Result<Self> {
        if !output.is_dir() {
            let message = format!("{} is not a directory", output.display());
            return Err(io::Error::new(io::ErrorKind::NotFound, message));
        }
        let listen = options.listen;
        let bound = |error: io::Error| io::Error::new(error.kind(), format!("{listen}: {error}"));
        let listener = std::net::TcpListener::bind(listen).map_err(bound)?;
        listener.set_nonblocking(true).map_err(bound)?;
        let host = Host {
            resolver: Resolver::new(routes, options.limits),
            published: output.join("published"),
            store: output.join("store"),
            started: Instant::now(),
        };
        Ok(Self {
            listener,
            host: Arc::new(host),
            options,
        })
    }

    /// The address the server listens on.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Answers requests on the threads of the options until the process ends. Returns only
    /// when the server cannot start.
    pub fn run(self) -> io::Result<Infallible> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(self.options.threads.max(1))
            .enable_io()
            .enable_time()
            .build()?;
        runtime.block_on(self.serve())
    }

    /// Answers requests on the runtime this is called in, which has I/O and time enabled, for
    /// as long as it is polled. Returns only when the server cannot start.
    pub async fn serve(self) -> io::Result<Infallible> {
        let Self {
            listener,
            host,
            options,
        } = self;
        let listener = TcpListener::from_std(listener)?;
        let connections = Arc::new(Semaphore::new(options.connections.max(1)));
        let mut http = http1::Builder::new();
        // A client may end its side of the connection once its request is sent, as simple
        // clients do, and is answered all the same.
        http.timer(TokioTimer::new())
            .header_read_timeout(options.head_timeout)
            .max_buf_size(HEAD_BYTES)
            .half_close(true);
        loop {
            // A connection is accepted only once there is room for it, so the connections the
            // server does not hold wait in the queue of the listener.
            let room = Arc::clone(&connections).acquire_owned().await;
            let room = room.map_err(io::Error::other)?;
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(error) if concerns_one_connection(&error) => continue,
                Err(error) => {
                    report(format_args!("a connection could not be accepted: {error}"));
                    tokio::time::sleep(ACCEPT_PAUSE).await;
                    continue;
                }
            };
            // Small responses are not held back to be sent with later ones.
            let _ = stream.set_nodelay(true);
            let host = Arc::clone(&host);
            let service = service_fn(move |request| {
                let host = Arc::clone(&host);
                async move { Ok::<_, Infallible>(host.answer(request).await) }
            });
            let io = TokioIo::new(Quiet::new(stream, options.idle_timeout));
            let connection = http.serve_connection(io, service);
            tokio::spawn(async move {
                // A client that goes away or stalls ends its connection; nothing is left to do.
                let _ = connection.await;
                drop(room);
            });
        }
    }
}

/// What the server reads from, and the resolver that says what to read.
#[derive(Debug)]
struct Host {
    resolver: Resolver,
    /// The directory of the pointers.
    published: PathBuf,
    /// The directory of the objects.
    store: PathBuf,
    /// The moment the resolver measures time from.
    started: Instant,
}

impl Host {
    /// The response to `incoming`. The body of the request is dropped unread.
    async fn answer(&self, incoming: hyper::Request<Incoming>) -> hyper::Response<ResponseBody> {
        let (head, _) = incoming.into_parts();
        let header = |name: HeaderName| head.headers.get(name).and_then(|v| v.to_str().ok());
        let request = Request {
            method: head.method.as_str(),
            path: head.uri.path(),
            range: header(RANGE),
            if_none_match: header(IF_NONE_MATCH),
            if_range: header(IF_RANGE),
        };
        let response = self.resolve(&request).await;
        let Body::Object(slice) = &response.body else {
            let body = fixed_body(&response.body);
            return written(response, body);
        };
        match self.body(slice).await {
            Ok(body) => written(response, body),
            Err(error) => {
                report(format_args!("object {}: {error}", slice.object));
                let failure = self.resolver.failure(&request);
                let body = fixed_body(&failure.body);
                written(failure, body)
            }
        }
    }

    /// The response of the resolver to `request`, after the reads its lookup asks for.
    async fn resolve(&self, request: &Request<'_>) -> Response {
        let mut lookup = self.resolver.lookup(request, self.started.elapsed());
        loop {
            lookup = match lookup {
                Lookup::Done(response) => return response,
                Lookup::Failed(response, fault) => {
                    report(&fault);
                    return response;
                }
                Lookup::Read(pending) => {
                    let read = self.read(pending.read()).await;
                    let fetched = match &read {
                        Ok(Some(bytes)) => Fetched::Found(bytes),
                        Ok(None) => Fetched::Absent,
                        Err(reason) => Fetched::Failed(reason),
                    };
                    pending.supply(fetched, self.started.elapsed())
                }
            };
        }
    }

    /// The bytes that `read` names, none when nothing is stored under the name, or why they
    /// could not be read.
    async fn read(&self, read: &Read) -> Result<Option<Vec<u8>>, String> {
        let path = match read {
            Read::Pointer(name) => self.published.join(name),
            Read::Root(object) | Read::Chunk(object) => self.store.join(object.path()),
        };
        match read_up_to(&path, read.limit()).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) => match error.kind() {
                io::ErrorKind::NotFound | io::ErrorKind::NotADirectory => Ok(None),
                _ => Err(error.to_string()),
            },
        }
    }

    /// The bytes of `slice`, read from the store as they are sent. The object has to be a file
    /// of the size its index gives; one of another size was cut short or replaced, and sending
    /// it would break off in the middle of the response.
    async fn body(&self, slice: &Slice) -> io::Result<ResponseBody> {
        let mut file = tokio::fs::File::open(self.store.join(slice.object.path())).await?;
        let metadata = file.metadata().await?;
        if !metadata.is_file() || metadata.len() != slice.size {
            let message = format!(
                "it has {} bytes and its index says {}",
                metadata.len(),
                slice.size
            );
            return Err(io::Error::new(io::ErrorKind::InvalidData, message));
        }
        file.seek(SeekFrom::Start(slice.start)).await?;
        let stream = ReaderStream::with_capacity(file.take(slice.length), READ_SIZE);
        Ok(StreamBody::new(stream.map_ok(Frame::data)).boxed())
    }
}

/// Whether an error of accepting concerns only the connection that was to be accepted, such as
/// one that its client has reset already, so the next one can be accepted at once. Any other
/// error, such as having no file descriptor left, would only repeat.
fn concerns_one_connection(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionRefused
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::Interrupted
    )
}

/// The content of the file at `path`, which fails when it is longer than `limit` bytes.
async fn read_up_to(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let file = tokio::fs::File::open(path).await?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() as u64 > limit {
        let message = format!("it is longer than {limit} bytes");
        return Err(io::Error::new(io::ErrorKind::InvalidData, message));
    }
    Ok(bytes)
}

/// The HTTP response for `response` with `body`.
fn written(response: Response, body: ResponseBody) -> hyper::Response<ResponseBody> {
    let mut builder = hyper::Response::builder().status(response.status);
    for (name, value) in &response.headers {
        builder = builder.header(*name, value.as_str());
    }
    builder.body(body).unwrap_or_else(|error| {
        report(format_args!("a response could not be written: {error}"));
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

/// Writes a line about something that went wrong to the standard error stream. A stream that
/// takes no line, such as a pipe whose reader is gone, loses the line and nothing else: the
/// request that the line is about is answered, and the server goes on.
fn report(message: impl fmt::Display) {
    let _ = writeln!(io::stderr().lock(), "{message}");
}

/// A stream that fails once no byte has been read or written for a time, so a client that
/// stops sending or stops receiving does not hold its connection.
#[derive(Debug)]
struct Quiet<S> {
    stream: S,
    /// How long the stream may be quiet.
    limit: Duration,
    /// The wait that ends when it has been quiet for that long.
    wait: Pin<Box<Sleep>>,
}

impl<S> Quiet<S> {
    fn new(stream: S, limit: Duration) -> Self {
        Self {
            stream,
            limit,
            wait: Box::pin(tokio::time::sleep(limit)),
        }
    }

    /// What `polled` gave. Bytes that moved start the wait anew; while none can move, the end
    /// of the wait is the end of the stream.
    fn moved<T>(
        &mut self,
        cx: &mut Context<'_>,
        polled: Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        if polled.is_ready() {
            let deadline = tokio::time::Instant::now() + self.limit;
            self.wait.as_mut().reset(deadline);
            return polled;
        }
        self.waited(cx, polled)
    }

    /// What `polled` gave, or the end of the stream when the wait is over.
    fn waited<T>(
        &mut self,
        cx: &mut Context<'_>,
        polled: Poll<io::Result<T>>,
    ) -> Poll<io::Result<T>> {
        if polled.is_pending() && self.wait.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Err(io::ErrorKind::TimedOut.into()));
        }
        polled
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for Quiet<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let polled = Pin::new(&mut self.stream).poll_read(cx, buffer);
        self.moved(cx, polled)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for Quiet<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let polled = Pin::new(&mut self.stream).poll_write(cx, bytes);
        self.moved(cx, polled)
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        slices: &[io::IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let polled = Pin::new(&mut self.stream).poll_write_vectored(cx, slices);
        self.moved(cx, polled)
    }

    fn is_write_vectored(&self) -> bool {
        self.stream.is_write_vectored()
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let polled = Pin::new(&mut self.stream).poll_flush(cx);
        self.waited(cx, polled)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let polled = Pin::new(&mut self.stream).poll_shutdown(cx);
        self.waited(cx, polled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepting_pauses_only_after_an_error_that_would_repeat() {
        use io::ErrorKind::{ConnectionAborted, ConnectionRefused, ConnectionReset, Interrupted};
        for kind in [
            ConnectionAborted,
            ConnectionRefused,
            ConnectionReset,
            Interrupted,
        ] {
            assert!(concerns_one_connection(&kind.into()), "{kind}");
        }
        let lasting = [
            io::Error::other("no file descriptor is left"),
            io::ErrorKind::OutOfMemory.into(),
            io::ErrorKind::PermissionDenied.into(),
            io::ErrorKind::InvalidInput.into(),
        ];
        for error in lasting {
            assert!(!concerns_one_connection(&error), "{error}");
        }
    }
}
