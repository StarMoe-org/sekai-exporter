//! The resolver of published views, specified in `docs/serving.md`: it maps the URL path of a
//! request to a file of a view and describes the response.
//!
//! The resolver does no I/O. A host routes a request, looks the target up in the path index that
//! its pointer names, and writes the response the resolver describes for the file it found.

mod response;
mod routes;

pub use response::{Body, Request, Response, Routing, Slice};
pub use routes::{Routes, RoutesError, Target};
