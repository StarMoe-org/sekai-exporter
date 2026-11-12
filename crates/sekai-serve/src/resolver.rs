//! Lookups: the reads that answer a request, and what a resolver remembers between requests.
//!
//! A lookup is a short sequence of reads that the host makes: the pointer of the view, the root
//! of the path index the pointer names, and the chunk of the index that holds the path. The
//! resolver remembers what the reads gave, within fixed limits, so most lookups read nothing.

use std::fmt;
use std::mem::size_of;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use sekai_format::paths::{Chunk, ChunkReference, PathIndex};
use sekai_format::publication::Pointer;
use sha2::{Digest, Sha256};

use crate::cache::Lru;
use crate::response::{ObjectId, Request, Response};
use crate::routes::{Routes, Target};

/// The longest URL path the resolver looks at, in bytes.
const MAX_PATH_BYTES: usize = 8 << 10;

/// The most bytes a host reads for a pointer.
const POINTER_BYTES: u64 = 64 << 10;

/// The most bytes a host reads for the root or a chunk of a path index.
const INDEX_BYTES: u64 = 64 << 20;

/// How much a resolver remembers between requests. Nothing a request names makes it remember
/// more.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// How many pointers are remembered, whether they were found or not.
    pub pointers: usize,
    /// The memory that the decoded roots and chunks of path indexes may take, in bytes.
    pub index_bytes: usize,
}

impl Default for Limits {
    /// 1024 pointers and 512 MiB of path indexes.
    fn default() -> Self {
        Self {
            pointers: 1024,
            index_bytes: 512 << 20,
        }
    }
}

/// What a resolver remembers at the moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// How many pointers are remembered.
    pub pointers: usize,
    /// The estimated memory of the decoded roots and chunks, in bytes.
    pub index_bytes: usize,
}

/// What a host reads for a lookup.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Read {
    /// The pointer of a view: the file at this path below the published root, such as
    /// `jp/storage/current.json`. The path is made of names, none of them the name of a device,
    /// so it is a file below the root on every system.
    Pointer(String),
    /// The root of a path index, an object of the store.
    Root(ObjectId),
    /// A chunk of a path index, an object of the store.
    Chunk(ObjectId),
}

impl Read {
    /// The most bytes the read may give. A host reports a longer file as [`Fetched::Failed`], so
    /// no request makes it hold more than this at once.
    pub fn limit(&self) -> u64 {
        match self {
            Self::Pointer(_) => POINTER_BYTES,
            Self::Root(_) | Self::Chunk(_) => INDEX_BYTES,
        }
    }
}

impl fmt::Display for Read {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pointer(name) => write!(f, "pointer {name}"),
            Self::Root(object) => write!(f, "path index root {object}"),
            Self::Chunk(object) => write!(f, "path index chunk {object}"),
        }
    }
}

/// What a read gave.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fetched<'b> {
    /// The bytes stored under the name.
    Found(&'b [u8]),
    /// Nothing is stored under the name.
    Absent,
    /// The read failed, or gave more than [`Read::limit`] bytes; with the reason, which becomes
    /// that of the [`Fault`].
    Failed(&'b str),
}

/// Why a lookup failed, for the log of a host: the read and what is wrong with what it gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    /// The read.
    pub read: Read,
    /// What is wrong.
    pub reason: String,
}

impl fmt::Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.read, self.reason)
    }
}

/// A lookup: the response to a request, or the read it needs next.
#[derive(Debug)]
pub enum Lookup<'a> {
    /// The host makes the read [`Pending::read`] names and goes on with [`Pending::supply`].
    Read(Pending<'a>),
    /// The response.
    Done(Response),
    /// The response to a request whose publication cannot be read, with the reason.
    Failed(Response, Fault),
}

/// A lookup that waits for a read.
#[derive(Debug)]
pub struct Pending<'a> {
    asked: Asked<'a>,
    read: Read,
}

/// The resolver of the paths of published views: it answers requests from what its host reads.
///
/// The resolver does no I/O and reads no clock. A host starts a lookup for each request, makes
/// the reads the lookup asks for, at most three, and writes the response:
///
/// ```
/// # use std::time::Duration;
/// # use sekai_serve::{Fetched, Limits, Lookup, Request, Resolver, Routes};
/// # let routes = br#"{ "routes": [{ "pattern": "/{region}/{view}/{path}" }] }"#;
/// # let resolver = Resolver::new(Routes::from_json(routes).unwrap(), Limits::default());
/// # let request = Request {
/// #     method: "GET",
/// #     path: "/jp/storage/a.png",
/// #     range: None,
/// #     if_none_match: None,
/// #     if_range: None,
/// # };
/// # let now = Duration::ZERO;
/// let mut lookup = resolver.lookup(&request, now);
/// let response = loop {
///     match lookup {
///         Lookup::Done(response) | Lookup::Failed(response, _) => break response,
///         // A host reads what `pending.read()` names; here nothing is published.
///         Lookup::Read(pending) => lookup = pending.supply(Fetched::Absent, now),
///     }
/// };
/// assert_eq!(response.status, 404);
/// ```
#[derive(Debug)]
pub struct Resolver {
    routes: Routes,
    max_age: Duration,
    state: Mutex<State>,
}

/// What the reads of earlier lookups gave.
#[derive(Debug)]
struct State {
    /// The pointers by their path, each counting as one.
    pointers: Lru<String, Pointed>,
    /// The decoded roots and chunks by their hash, each counting its estimated memory.
    index: Lru<[u8; 32], Indexed>,
}

/// What a pointer named when it was read.
#[derive(Debug, Clone, Copy)]
struct Pointed {
    /// When it was read.
    read_at: Duration,
    /// The root of the path index it names; none when nothing is published under it.
    root: Option<[u8; 32]>,
}

impl Pointed {
    /// Whether the reading holds at `now` for a lookup that started at `started`: when it was
    /// made since then, or less than `max_age` ago. That is so for the pointer of a named
    /// version as well, which changes when the version is published again under its name.
    fn holds(&self, started: Duration, now: Duration, max_age: Duration) -> bool {
        self.read_at >= started || now.saturating_sub(self.read_at) < max_age
    }
}

/// A decoded object of a path index.
#[derive(Debug, Clone)]
enum Indexed {
    Root(Arc<PathIndex>),
    Chunk(Arc<Chunk>),
}

impl Resolver {
    /// A resolver of `routes` that remembers nothing yet and never more than `limits`.
    pub fn new(routes: Routes, limits: Limits) -> Self {
        Self {
            max_age: Duration::from_secs(routes.max_age()),
            routes,
            state: Mutex::new(State {
                pointers: Lru::new(limits.pointers),
                index: Lru::new(limits.index_bytes),
            }),
        }
    }

    /// The routes the resolver answers by.
    pub fn routes(&self) -> &Routes {
        &self.routes
    }

    /// The response to `request` when the object of the file it names cannot be read.
    pub fn failure(&self, request: &Request<'_>) -> Response {
        shaped(request, self.routes.failure())
    }

    /// What the resolver remembers at the moment.
    pub fn usage(&self) -> Usage {
        let state = self.state();
        Usage {
            pointers: state.pointers.len(),
            index_bytes: state.index.weight(),
        }
    }

    /// Starts the lookup that answers `request`. `now` is the time since any fixed moment the
    /// host chooses; the resolver uses it to tell how old its reading of a pointer is.
    pub fn lookup<'a>(&'a self, request: &Request<'a>, now: Duration) -> Lookup<'a> {
        let routes = &self.routes;
        let reply = |response: Response| Lookup::Done(shaped(request, response));
        match request.method {
            "GET" | "HEAD" => {}
            "OPTIONS" => return reply(routes.options()),
            _ => return reply(routes.not_allowed()),
        }
        if request.path.len() > MAX_PATH_BYTES {
            return reply(routes.too_long());
        }
        let Some(target) = routes.resolve(request.path) else {
            return reply(routes.not_found());
        };
        let asked = Asked {
            resolver: self,
            request: *request,
            target,
            started: now,
        };
        asked.pointer(now)
    }

    /// The state. The lock is held only around single operations of the caches. Should one of
    /// them panic, what it leaves may be half changed, and all of it can be read again: the
    /// resolver goes on with nothing remembered, rather than fail every lookup from then on.
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| {
            let mut state = poisoned.into_inner();
            state.pointers.clear();
            state.index.clear();
            self.state.clear_poison();
            state
        })
    }

    /// The last reading of the pointer at `name`, if it is remembered.
    fn reading(&self, name: &str) -> Option<Pointed> {
        self.state().pointers.get(name).copied()
    }

    /// The decoded root or chunk with the hash `sha256`, if it is remembered.
    fn decoded(&self, sha256: &[u8; 32]) -> Option<Indexed> {
        self.state().index.get(sha256).cloned()
    }

    fn remember(&self, name: String, pointed: Pointed) {
        self.state().pointers.insert(name, pointed, 1);
    }

    fn keep(&self, sha256: [u8; 32], indexed: Indexed, weight: usize) {
        self.state().index.insert(sha256, indexed, weight);
    }
}

/// `response` as the method of `request` gets it: without a body for `HEAD`.
fn shaped(request: &Request<'_>, response: Response) -> Response {
    match request.method {
        "HEAD" => response.without_body(),
        _ => response,
    }
}

/// A request and the file it names.
#[derive(Debug)]
struct Asked<'a> {
    resolver: &'a Resolver,
    request: Request<'a>,
    target: Target,
    /// When the lookup started.
    started: Duration,
}

impl<'a> Asked<'a> {
    fn done(&self, response: Response) -> Lookup<'a> {
        Lookup::Done(shaped(&self.request, response))
    }

    fn not_found(&self) -> Lookup<'a> {
        self.done(self.resolver.routes.not_found())
    }

    fn failed(&self, read: Read, reason: impl Into<String>) -> Lookup<'a> {
        let response = self.resolver.failure(&self.request);
        let reason = reason.into();
        Lookup::Failed(response, Fault { read, reason })
    }

    fn read(self, read: Read) -> Lookup<'a> {
        Lookup::Read(Pending { asked: self, read })
    }

    /// Goes on from the pointer of the target: from its last reading while that holds, or by
    /// reading it.
    fn pointer(self, now: Duration) -> Lookup<'a> {
        let name = self.target.pointer();
        match self.resolver.reading(&name) {
            Some(pointed) if pointed.holds(self.started, now, self.resolver.max_age) => {
                self.pointed(pointed.root)
            }
            _ => self.read(Read::Pointer(name)),
        }
    }

    /// Goes on from what the pointer names.
    fn pointed(self, root: Option<[u8; 32]>) -> Lookup<'a> {
        match root {
            Some(root) => self.root(root),
            None => self.not_found(),
        }
    }

    /// Goes on from the root of the path index: a decoded one, or by reading it.
    fn root(self, sha256: [u8; 32]) -> Lookup<'a> {
        match self.resolver.decoded(&sha256) {
            Some(Indexed::Root(index)) => self.indexed(sha256, &index),
            _ => self.read(Read::Root(ObjectId::root(sha256))),
        }
    }

    /// Goes on from the chunk of `index` whose range holds the path: a decoded one, or by
    /// reading it.
    fn indexed(self, root: [u8; 32], index: &PathIndex) -> Lookup<'a> {
        let Some(at) = index.chunk_of(&self.target.path) else {
            return self.not_found();
        };
        match parse_sha256(&index.chunks[at].sha256) {
            Some(sha256) => self.chunk(sha256),
            None => {
                let reason = format!("chunk {at} is not named by a SHA-256");
                self.failed(Read::Root(ObjectId::root(root)), reason)
            }
        }
    }

    /// Goes on from the chunk that holds the path: a decoded one, or by reading it.
    fn chunk(self, sha256: [u8; 32]) -> Lookup<'a> {
        match self.resolver.decoded(&sha256) {
            Some(Indexed::Chunk(chunk)) => self.found(sha256, &chunk),
            _ => self.read(Read::Chunk(ObjectId::chunk(sha256))),
        }
    }

    /// Answers with the file of the path in `chunk`, if it has one.
    fn found(self, sha256: [u8; 32], chunk: &Chunk) -> Lookup<'a> {
        let file = chunk.get(&self.target.path);
        match self
            .resolver
            .routes
            .respond(&self.request, &self.target, file)
        {
            Ok(response) => self.done(response),
            Err(reason) => {
                let reason = format!("{:?}: {reason}", self.target.path);
                self.failed(Read::Chunk(ObjectId::chunk(sha256)), reason)
            }
        }
    }
}

impl<'a> Pending<'a> {
    /// What the host reads.
    pub fn read(&self) -> &Read {
        &self.read
    }

    /// Goes on without the read, at the time `now`, from what the resolver remembers by now.
    ///
    /// A host that lets one lookup make a read while others that need the same read wait calls
    /// this on each of those once the read is made: the pointer, root or chunk it gave is then
    /// remembered, and the lookup goes on from it. When it is not remembered, the lookup asks
    /// for the read again.
    pub fn resume(self, now: Duration) -> Lookup<'a> {
        let Self { asked, read } = self;
        match read {
            Read::Pointer(_) => asked.pointer(now),
            Read::Root(object) => asked.root(*object.sha256()),
            Read::Chunk(object) => asked.chunk(*object.sha256()),
        }
    }

    /// Goes on with what the read gave, at the time `now`.
    ///
    /// A pointer that is absent publishes nothing, and the request is not found. Anything else
    /// that cannot be used fails the lookup, and nothing of it is remembered: a read that
    /// failed, a root or a chunk that is absent, one that does not have the hash it is named
    /// by, and whatever does not follow its format.
    pub fn supply(self, fetched: Fetched<'_>, now: Duration) -> Lookup<'a> {
        let Self { asked, read } = self;
        let resolver = asked.resolver;
        let bytes = match fetched {
            Fetched::Found(bytes) => bytes,
            Fetched::Failed(reason) => return asked.failed(read, reason),
            Fetched::Absent => {
                return match read {
                    Read::Pointer(name) => {
                        let root = None;
                        resolver.remember(name, Pointed { read_at: now, root });
                        asked.not_found()
                    }
                    read => asked.failed(read, "is not in the store"),
                };
            }
        };
        match read {
            Read::Pointer(name) => match pointer_root(bytes) {
                Ok(root) => {
                    resolver.remember(name, Pointed { read_at: now, root });
                    asked.pointed(root)
                }
                Err(reason) => asked.failed(Read::Pointer(name), reason),
            },
            Read::Root(object) => match decoded_root(&object, bytes) {
                Ok(index) => {
                    let index = Arc::new(index);
                    let sha256 = *object.sha256();
                    let weight = root_weight(&index);
                    resolver.keep(sha256, Indexed::Root(Arc::clone(&index)), weight);
                    asked.indexed(sha256, &index)
                }
                Err(reason) => asked.failed(Read::Root(object), reason),
            },
            Read::Chunk(object) => match decoded_chunk(&object, bytes) {
                Ok(chunk) => {
                    let chunk = Arc::new(chunk);
                    let sha256 = *object.sha256();
                    let weight = chunk_weight(&chunk);
                    resolver.keep(sha256, Indexed::Chunk(Arc::clone(&chunk)), weight);
                    asked.found(sha256, &chunk)
                }
                Err(reason) => asked.failed(Read::Chunk(object), reason),
            },
        }
    }
}

/// The root of the path index that the pointer in `bytes` names, if it names one.
fn pointer_root(bytes: &[u8]) -> Result<Option<[u8; 32]>, String> {
    let pointer: Pointer = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if pointer.format != "pointer" {
        return Err(format!("its format is {:?}", pointer.format));
    }
    match &pointer.paths {
        None => Ok(None),
        Some(paths) => parse_sha256(paths)
            .map(Some)
            .ok_or_else(|| format!("it names the path index {paths:?}, which is not a SHA-256")),
    }
}

/// Checks that `bytes` are the object `object` names.
fn verified(object: &ObjectId, bytes: &[u8]) -> Result<(), String> {
    if Sha256::digest(bytes).as_slice() == object.sha256() {
        Ok(())
    } else {
        Err("does not have the hash it is named by".to_owned())
    }
}

fn decoded_root(object: &ObjectId, bytes: &[u8]) -> Result<PathIndex, String> {
    verified(object, bytes)?;
    PathIndex::from_bytes(bytes).map_err(|e| e.to_string())
}

fn decoded_chunk(object: &ObjectId, bytes: &[u8]) -> Result<Chunk, String> {
    verified(object, bytes)?;
    Chunk::decode(bytes).map_err(|e| e.to_string())
}

/// The 32 bytes that `text` gives as 64 lowercase hexadecimal digits.
fn parse_sha256(text: &str) -> Option<[u8; 32]> {
    let digit = |byte: u8| match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    };
    let (pairs, []) = text.as_bytes().as_chunks::<2>() else {
        return None;
    };
    let pairs: &[[u8; 2]; 32] = pairs.try_into().ok()?;
    let mut sha256 = [0; 32];
    for (byte, [high, low]) in sha256.iter_mut().zip(pairs) {
        *byte = digit(*high)? << 4 | digit(*low)?;
    }
    Some(sha256)
}

/// The memory of a decoded root: what it allocates, without the bookkeeping of the allocator.
fn root_weight(index: &PathIndex) -> usize {
    let texts = [
        &index.format,
        &index.view,
        &index.published_version,
        &index.revision,
    ];
    let chunks = index
        .chunks
        .iter()
        .map(|chunk| chunk.first.capacity() + chunk.sha256.capacity());
    size_of::<PathIndex>()
        + texts.into_iter().map(String::capacity).sum::<usize>()
        + index.chunks.capacity() * size_of::<ChunkReference>()
        + chunks.sum::<usize>()
}

/// The memory of a decoded chunk.
fn chunk_weight(chunk: &Chunk) -> usize {
    size_of::<Chunk>() + chunk.allocated()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panic_under_the_lock_only_empties_the_resolver() {
        let routes = br#"{ "routes": [{ "pattern": "/{region}/{view}/{path}" }] }"#;
        let resolver = Resolver::new(Routes::from_json(routes).unwrap(), Limits::default());
        let remember = || {
            let read_at = Duration::ZERO;
            resolver.remember(
                "jp/storage/current.json".to_owned(),
                Pointed {
                    read_at,
                    root: None,
                },
            );
            resolver.usage().pointers
        };
        assert_eq!(remember(), 1);

        let holder = std::thread::scope(|scope| {
            let holder = scope.spawn(|| {
                let _state = resolver.state();
                panic!("a panic while the state is locked");
            });
            holder.join()
        });
        assert!(holder.is_err());
        assert!(resolver.state.is_poisoned());

        assert_eq!(resolver.usage().pointers, 0);
        assert!(!resolver.state.is_poisoned());
        assert_eq!(remember(), 1);
    }
}
