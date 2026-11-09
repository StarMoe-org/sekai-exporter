//! Requests with retries, back-off and a rate limit.

use std::io::Read;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use ureq::http::Response;
use ureq::typestate::WithBody;
use ureq::{Agent, Body, Proxy, RequestBuilder};

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

pub(super) struct Http {
    agent: Agent,
    retries: u32,
    back_off: Duration,
    interval: Option<Duration>,
    next: Mutex<Instant>,
}

impl Http {
    pub(super) fn new(network: &Network) -> Result<Self, SourceError> {
        let mut config = Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_per_read(Some(Duration::from_secs(60)));
        if let Some(proxy) = &network.proxy {
            let proxy = Proxy::new(proxy).map_err(|e| SourceError::new("config", e))?;
            config = config.proxy(Some(proxy));
        }
        Ok(Self {
            agent: config.build().into(),
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
    pub(super) fn get(&self, url: &str, headers: &Headers) -> Result<Response<Body>, Failure> {
        let mut request = self.agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, value.as_str());
        }
        accepted(url, request.call())
    }

    /// Reads a whole response.
    pub(super) fn bytes(&self, url: &str, headers: &Headers) -> Result<Vec<u8>, SourceError> {
        self.attempts(|_| whole(url, self.get(url, headers)?))
    }

    /// Sends one POST request with `body`, without retrying it, and reads the response.
    pub(super) fn post(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
    ) -> Result<Vec<u8>, SourceError> {
        self.pace();
        send(self.agent.post(url), url, headers, body)
            .map_err(|(Failure::Retry(error) | Failure::Fatal(error))| error)
    }

    /// Sends a PUT request with `body` and reads the response.
    pub(super) fn put(
        &self,
        url: &str,
        headers: &Headers,
        body: &[u8],
    ) -> Result<Vec<u8>, SourceError> {
        self.attempts(|_| send(self.agent.put(url), url, headers, body))
    }

    /// Sends an empty POST request and returns the cookies it sets.
    pub(super) fn cookies(&self, url: &str, headers: &Headers) -> Result<Vec<String>, SourceError> {
        self.attempts(|_| {
            let mut request = self.agent.post(url);
            for (name, value) in headers {
                request = request.header(*name, value.as_str());
            }
            let response = accepted(url, request.send_empty())?;
            Ok(response
                .headers()
                .get_all("set-cookie")
                .iter()
                .filter_map(|value| value.to_str().ok())
                .map(str::to_owned)
                .collect())
        })
    }
}

fn send(
    mut request: RequestBuilder<WithBody>,
    url: &str,
    headers: &Headers,
    body: &[u8],
) -> Result<Vec<u8>, Failure> {
    for (name, value) in headers {
        request = request.header(*name, value.as_str());
    }
    whole(url, accepted(url, request.send(body))?)
}

/// Reads a whole response.
fn whole(url: &str, mut response: Response<Body>) -> Result<Vec<u8>, Failure> {
    response
        .body_mut()
        .with_config()
        .limit(u64::MAX)
        .read_to_vec()
        .map_err(|e| Failure::Retry(SourceError::new("network", format!("{url}: {e}"))))
}

fn accepted(
    url: &str,
    result: Result<Response<Body>, ureq::Error>,
) -> Result<Response<Body>, Failure> {
    let response =
        result.map_err(|e| Failure::Retry(SourceError::new("network", format!("{url}: {e}"))))?;
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

/// The length the response says it has.
pub(super) fn content_length(response: &Response<Body>) -> Option<u64> {
    response
        .headers()
        .get("content-length")?
        .to_str()
        .ok()?
        .parse()
        .ok()
}

/// Starts a transfer again in `sink`.
pub(super) fn restart(sink: &mut dyn Sink) -> Result<(), Failure> {
    sink.reset()
        .map_err(|e| Failure::Fatal(SourceError::new("sink", e)))
}

/// Streams `response` into `sink` and checks the number of bytes against `expected`.
pub(super) fn copy(
    url: &str,
    response: Response<Body>,
    sink: &mut dyn Sink,
    expected: Option<u64>,
) -> Result<u64, Failure> {
    let mut reader = response
        .into_body()
        .into_with_config()
        .limit(u64::MAX)
        .reader();
    let mut buffer = vec![0; 1 << 16];
    let mut total = 0;
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|e| Failure::Retry(SourceError::new("network", format!("{url}: {e}"))))?;
        if read == 0 {
            break;
        }
        sink.write_all(&buffer[..read])
            .map_err(|e| Failure::Fatal(SourceError::new("sink", e)))?;
        total += read as u64;
    }
    match expected {
        Some(expected) if expected != total => Err(Failure::Retry(SourceError::new(
            "length",
            format!("{url}: expected {expected} bytes, received {total}"),
        ))),
        _ => Ok(total),
    }
}
