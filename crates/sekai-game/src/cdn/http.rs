//! Requests with retries, back-off and a rate limit.

use std::sync::{Mutex, PoisonError};
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

/// A client that speaks HTTP/2 to the hosts that offer it, where the requests that run at the
/// same time share one connection, and HTTP/1.1 to the others. Its methods block the calling
/// thread; the connections are driven by a thread of the client.
pub(super) struct Http {
    client: Client,
    runtime: Runtime,
    retries: u32,
    back_off: Duration,
    interval: Option<Duration>,
    next: Mutex<Instant>,
}

impl Http {
    pub(super) fn new(network: &Network) -> Result<Self, SourceError> {
        // The process has one TLS provider; a client made earlier has installed it already.
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut client = Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_secs(60))
            .http2_adaptive_window(true);
        if let Some(proxy) = &network.proxy {
            client = client.proxy(Proxy::all(proxy).map_err(|e| SourceError::new("config", e))?);
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| SourceError::new("config", e))?;
        Ok(Self {
            client: client.build().map_err(|e| SourceError::new("config", e))?,
            runtime,
            retries: network.retries,
            back_off: Duration::from_millis(network.back_off_ms),
            interval: network
                .rate_limit
                .filter(|rate| *rate > 0.0)
                .map(|rate| Duration::from_secs_f64(1.0 / rate)),
            next: Mutex::new(Instant::now()),
        })
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
        self.send(url, with(self.client.get(url), headers))
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
        self.exchange(self.client.post(url), url, headers, body)
            .map_err(|(Failure::Retry(error) | Failure::Fatal(error))| error)
    }

    /// Sends a PUT request with `body` and reads the response.
    pub(super) fn put(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
    ) -> Result<Vec<u8>, SourceError> {
        self.attempts(|_| self.exchange(self.client.put(url), url, headers, body))
    }

    /// Sends an empty POST request and returns the cookies it sets.
    pub(super) fn cookies(&self, url: &str, headers: &Headers) -> Result<Vec<String>, SourceError> {
        self.attempts(|_| {
            let response = self.send(url, with(self.client.post(url), headers))?;
            Ok(response
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|value| value.to_str().ok())
                .map(str::to_owned)
                .collect())
        })
    }

    /// Streams `response` into `sink` and checks the number of bytes against `expected`.
    pub(super) fn copy(
        &self,
        url: &str,
        mut response: Response,
        sink: &mut dyn Sink,
        expected: Option<u64>,
    ) -> Result<u64, Failure> {
        let mut total = 0;
        while let Some(chunk) = self
            .runtime
            .block_on(response.chunk())
            .map_err(|e| network(url, e))?
        {
            sink.write_all(&chunk)
                .map_err(|e| Failure::Fatal(SourceError::new("sink", e)))?;
            total += chunk.len() as u64;
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
