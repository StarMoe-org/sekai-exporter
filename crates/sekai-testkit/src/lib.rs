//! Local servers that play the services the exporter talks to, for tests: an S3-compatible bucket
//! that checks signatures ([`s3`]) and a CDN of the `nuverse` protocol ([`nuverse`]).
//!
//! Each server listens on a free local port, answers one request per connection on its own
//! thread, and lives until the test process ends.

mod http;
pub mod nuverse;
pub mod s3;
