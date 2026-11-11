//! The resolver of published views, specified in `docs/serving.md`: it maps the URL path of a
//! request to a file of a view and describes the response.
//!
//! The resolver does no I/O. A [`Resolver`] answers a request through a [`Lookup`], which asks
//! its host for the few reads it needs, a pointer and objects of a path index, and remembers what
//! they gave within fixed [`Limits`]. The host writes the response the lookup ends with.
//!
//! The `server` feature adds one host, [`server`]: the HTTP/1.1 server of
//! `sekai-exporter serve`, which reads from an output directory.

mod cache;
mod resolver;
mod response;
mod routes;
#[cfg(feature = "server")]
pub mod server;

pub use resolver::{Fault, Fetched, Limits, Lookup, Pending, Read, Resolver, Usage};
pub use response::{Body, ObjectId, Request, Response, Slice};
pub use routes::{Routes, RoutesError, Target};
