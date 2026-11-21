//! Requests with retries, back-off and a rate limit.
//!
//! A download whose connection has all but stopped is dropped and tried again on new
//! connections ([`Crawl`]).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use reqwest::{Client, Proxy, RequestBuilder, Response};
use tokio::runtime::Runtime;

use super::SourceError;
use super::region::Network;
use crate::sink::Sink;

pub(super) type Headers = Vec<(&'static str, String)>;

/// How an attempt failed.
pub(super) enum Failure {
    /// Trying again can help.
    Retry(SourceError),
    /// Trying again cannot help.
    Fatal(SourceError),
}

/// When a download counts as crawling. Through a proxy, an HTTP/2 connection now and then
/// slows to a few kilobytes a second while a new connection to the same host is fast; the
/// read timeout does not end it, since bytes still come. While many downloads share the
/// connections each is slow for its share of them, so a download is judged only when few are
/// left, as at the end of a batch.
#[derive(Debug, Clone, Copy)]
struct Crawl {
    /// Downloads in flight at most, for one to be judged.
    tail: usize,
    window: Duration,
    /// A judged download that brings less than this in `window` crawls.
    bytes: u64,
}

const CRAWL: Crawl = Crawl {
    tail: 8,
    window: Duration::from_secs(10),
    bytes: 512 * 1024,
};

/// A client that speaks HTTP/2 to the hosts that offer it, where the requests that run at the
/// same time share a connection, and HTTP/1.1 to the others. Requests take its connection pools
/// in turn, so a host that speaks HTTP/2 gets one connection per pool. Its methods block the
/// calling thread; the connections are driven by a thread of the client.
pub(super) struct Http {
    /// The proxy of the network, which new clients go through.
    proxy: Option<String>,
    /// One client per connection pool. A download that crawls replaces them all, so that the
    /// requests after it take new connections.
    clients: RwLock<Vec<Client>>,
    /// The number of requests started, which picks the client of the next one.
    turn: AtomicUsize,
    runtime: Runtime,
    retries: u32,
    back_off: Duration,
    interval: Option<Duration>,
    next: Mutex<Instant>,
    /// Downloads in flight.
    downloads: AtomicUsize,
    crawl: Crawl,
}

impl Http {
    pub(super) fn new(network: &Network) -> Result<Self, SourceError> {
        // The process has one TLS provider; a client made earlier has installed it already.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let clients = clients(network.connections.get(), network.proxy.as_deref())?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| SourceError::new("config", e))?;
        Ok(Self {
            proxy: network.proxy.clone(),
            clients: RwLock::new(clients),
            turn: AtomicUsize::new(0),
            runtime,
            retries: network.retries,
            back_off: Duration::from_millis(network.back_off_ms),
            interval: network
                .rate_limit
                .filter(|rate| *rate > 0.0)
                .map(|rate| Duration::from_secs_f64(1.0 / rate)),
            next: Mutex::new(Instant::now()),
            downloads: AtomicUsize::new(0),
            crawl: CRAWL,
        })
    }

    /// The client whose turn it is.
    fn client(&self) -> Client {
        let turn = self.turn.fetch_add(1, Ordering::Relaxed);
        let clients = self.clients.read().unwrap_or_else(PoisonError::into_inner);
        clients[turn % clients.len()].clone()
    }

    /// Replaces the clients, so that the requests from now on take new connections. Requests
    /// in flight keep theirs.
    fn renew(&self) -> Result<(), Failure> {
        let mut clients = self.clients.write().unwrap_or_else(PoisonError::into_inner);
        *clients = self::clients(clients.len(), self.proxy.as_deref()).map_err(Failure::Fatal)?;
        Ok(())
    }

    /// Waits for the rate limit.
    fn pace(&self) {
        let Some(interval) = self.interval else {
            return;
        };
        let wait = {
            let mut next = self.next.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            let start = (*next).max(now);
            *next = start + interval;
            start - now
        };
        std::thread::sleep(wait);
    }

    /// Runs `attempt` until it succeeds, fails for good, or the retries run out. It is given the
    /// number of the attempt, from zero.
    pub(super) fn attempts<T>(
        &self,
        mut attempt: impl FnMut(u32) -> Result<T, Failure>,
    ) -> Result<T, SourceError> {
        let mut number = 0;
        loop {
            self.pace();
            match attempt(number) {
                Ok(value) => return Ok(value),
                Err(Failure::Fatal(error)) => return Err(error),
                Err(Failure::Retry(error)) if number >= self.retries => return Err(error),
                Err(Failure::Retry(_)) => {
                    std::thread::sleep(self.back_off.saturating_mul(1 << number.min(8)));
                    number += 1;
                }
            }
        }
    }

    /// Sends one GET request.
    pub(super) fn get(&self, url: &str, headers: &Headers) -> Result<Response, Failure> {
        self.send(url, with(self.client().get(url), headers))
    }

    /// Sends `request` and returns the response when its status is a success.
    fn send(&self, url: &str, request: RequestBuilder) -> Result<Response, Failure> {
        // The request starts its timers when it is sent, which needs the runtime.
        let response = self
            .runtime
            .block_on(async { request.send().await })
            .map_err(|e| network(url, e))?;
        let status = response.status().as_u16();
        if (200..300).contains(&status) {
            return Ok(response);
        }
        let mut error = SourceError::new("status", format!("HTTP {status} for {url}"));
        error.status = Some(status);
        Err(if status == 408 || status == 429 || status >= 500 {
            Failure::Retry(error)
        } else {
            Failure::Fatal(error)
        })
    }

    /// Reads a whole response.
    fn whole(&self, url: &str, response: Response) -> Result<Vec<u8>, Failure> {
        let bytes = self
            .runtime
            .block_on(response.bytes())
            .map_err(|e| network(url, e))?;
        Ok(bytes.into())
    }

    /// Sends `request` with `headers` and `body` and reads the response.
    fn exchange(
        &self,
        request: RequestBuilder,
        url: &str,
        headers: &Headers,
        body: &[u8],
    ) -> Result<Vec<u8>, Failure> {
        let response = self.send(url, with(request, headers).body(body.to_vec()))?;
        self.whole(url, response)
    }

    /// Reads a whole response.
    pub(super) fn bytes(&self, url: &str, headers: &Headers) -> Result<Vec<u8>, SourceError> {
        self.attempts(|_| self.whole(url, self.get(url, headers)?))
    }

    /// Sends one POST request with `body`, without retrying it, and reads the response.
    pub(super) fn post(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
    ) -> Result<Vec<u8>, SourceError> {
        self.pace();
        self.exchange(self.client().post(url), url, headers, body)
            .map_err(|(Failure::Retry(error) | Failure::Fatal(error))| error)
    }

    /// Sends a PUT request with `body` and reads the response.
    pub(super) fn put(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
    ) -> Result<Vec<u8>, SourceError> {
        self.attempts(|_| self.exchange(self.client().put(url), url, headers, body))
    }

    /// Sends an empty POST request and returns the cookies it sets.
    pub(super) fn cookies(&self, url: &str, headers: &Headers) -> Result<Vec<String>, SourceError> {
        self.attempts(|_| {
            let response = self.send(url, with(self.client().post(url), headers))?;
            Ok(response
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|value| value.to_str().ok())
                .map(str::to_owned)
                .collect())
        })
    }

    /// Streams `response` into `sink` and checks the number of bytes against `expected`. A
    /// download that crawls ([`Crawl`]) is dropped, the clients are renewed, and it fails to be
    /// tried again.
    pub(super) fn copy(
        &self,
        url: &str,
        mut response: Response,
        sink: &mut dyn Sink,
        expected: Option<u64>,
    ) -> Result<u64, Failure> {
        let _download = InFlight::count(&self.downloads);
        let mut total = 0;
        // The start of the window being judged and what it has brought.
        let mut window: Option<(Instant, u64)> = None;
        while let Some(chunk) = self
            .runtime
            .block_on(response.chunk())
            .map_err(|e| network(url, e))?
        {
            sink.write_all(&chunk)
                .map_err(|e| Failure::Fatal(SourceError::new("sink", e)))?;
            total += chunk.len() as u64;
            if self.downloads.load(Ordering::Relaxed) > self.crawl.tail {
                window = None;
                continue;
            }
            let (start, brought) = window.get_or_insert((Instant::now(), 0));
            *brought += chunk.len() as u64;
            let elapsed = start.elapsed();
            if elapsed < self.crawl.window {
                continue;
            }
            if *brought < self.crawl.bytes {
                self.renew()?;
                return Err(Failure::Retry(SourceError::new(
                    "network",
                    format!("{url}: {brought} bytes in {:.0} s", elapsed.as_secs_f64()),
                )));
            }
            window = None;
        }
        match expected {
            Some(expected) if expected != total => Err(Failure::Retry(SourceError::new(
                "length",
                format!("{url}: expected {expected} bytes, received {total}"),
            ))),
            _ => Ok(total),
        }
    }
}

/// Counts a download in flight until dropped.
struct InFlight<'a>(&'a AtomicUsize);

impl<'a> InFlight<'a> {
    fn count(downloads: &'a AtomicUsize) -> Self {
        downloads.fetch_add(1, Ordering::Relaxed);
        Self(downloads)
    }
}

impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// `count` clients, each with a connection pool of its own, through `proxy` when there is one.
fn clients(count: usize, proxy: Option<&str>) -> Result<Vec<Client>, SourceError> {
    (0..count).map(|_| build_client(proxy)).collect()
}

/// A client with a pool of its own, through `proxy` when there is one.
fn build_client(proxy: Option<&str>) -> Result<Client, SourceError> {
    let mut client = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .http2_adaptive_window(true);
    if let Some(proxy) = proxy {
        client = client.proxy(Proxy::all(proxy).map_err(|e| SourceError::new("config", e))?);
    }
    client.build().map_err(|e| SourceError::new("config", e))
}

/// `request` with `headers`.
fn with(request: RequestBuilder, headers: &Headers) -> RequestBuilder {
    headers.iter().fold(request, |request, (name, value)| {
        request.header(*name, value.as_str())
    })
}

fn network(url: &str, error: impl std::fmt::Display) -> Failure {
    Failure::Retry(SourceError::new("network", format!("{url}: {error}")))
}

/// Starts a transfer again in `sink`.
pub(super) fn restart(sink: &mut dyn Sink) -> Result<(), Failure> {
    sink.reset()
        .map_err(|e| Failure::Fatal(SourceError::new("sink", e)))
}

#[cfg(test)]
mod tests {
    use std::io::{self, Read, Write};
    use std::net::TcpListener;
    use std::sync::Arc;

    use super::*;

    /// Collects what a download writes.
    #[derive(Default)]
    struct Buffer {
        bytes: Vec<u8>,
        resets: usize,
    }

    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Sink for Buffer {
        fn reset(&mut self) -> io::Result<()> {
            self.bytes.clear();
            self.resets += 1;
            Ok(())
        }
    }

    /// Serves `body` on a local port: the first connection a byte at a time, slowly, and every
    /// later one at once. Returns the address and the count of connections.
    fn serve_crawling_once(body: &'static [u8]) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}/file", listener.local_addr().unwrap());
        let connections = Arc::new(AtomicUsize::new(0));
        let counted = connections.clone();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let first = counted.fetch_add(1, Ordering::SeqCst) == 0;
                std::thread::spawn(move || {
                    let mut head = Vec::new();
                    let mut byte = [0];
                    while !head.ends_with(b"\r\n\r\n") && stream.read_exact(&mut byte).is_ok() {
                        head.push(byte[0]);
                    }
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    if first {
                        for byte in body {
                            if stream.write_all(&[*byte]).is_err() {
                                return;
                            }
                            std::thread::sleep(Duration::from_millis(20));
                        }
                    } else {
                        let _ = stream.write_all(body);
                    }
                });
            }
        });
        (address, connections)
    }

    #[test]
    fn a_crawling_download_is_tried_again_on_a_new_connection() {
        let body: &'static [u8] = &[7; 200];
        let (url, connections) = serve_crawling_once(body);
        let network = Network {
            back_off_ms: 1,
            ..Network::default()
        };
        let mut http = Http::new(&network).unwrap();
        http.crawl = Crawl {
            tail: 8,
            window: Duration::from_millis(200),
            bytes: 100,
        };
        let mut sink = Buffer::default();
        let copied = http
            .attempts(|_| {
                restart(&mut sink)?;
                let response = http.get(&url, &Headers::new())?;
                http.copy(&url, response, &mut sink, Some(body.len() as u64))
            })
            .unwrap();
        assert_eq!(copied, body.len() as u64);
        assert_eq!(sink.bytes, body);
        assert_eq!(sink.resets, 2);
        assert_eq!(connections.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_slow_download_among_many_is_left_alone() {
        let body: &'static [u8] = &[7; 30];
        let (url, connections) = serve_crawling_once(body);
        let mut http = Http::new(&Network::default()).unwrap();
        http.crawl = Crawl {
            tail: 8,
            window: Duration::from_millis(200),
            bytes: 100,
        };
        // Nine others in flight: the batch is not down to its tail.
        let _others: Vec<_> = (0..9).map(|_| InFlight::count(&http.downloads)).collect();
        let mut sink = Buffer::default();
        let response = http.get(&url, &Headers::new()).ok().unwrap();
        let copied = http
            .copy(&url, response, &mut sink, Some(body.len() as u64))
            .ok()
            .unwrap();
        assert_eq!(copied, body.len() as u64);
        assert_eq!(connections.load(Ordering::SeqCst), 1);
    }
}
