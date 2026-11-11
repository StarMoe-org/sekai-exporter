//! The boundary that keeps a panic in a converter, a decoder or an encoder to the asset or the
//! bundle it was working on, which then fails with the code [`PANICKED`] while the run goes on.

use std::any::Any;
use std::panic::{self, AssertUnwindSafe};

/// The error code of an asset or a bundle whose work panicked.
pub const PANICKED: &str = "panicked";

/// What `work` returns, or the message of its panic.
///
/// The state that `work` shares with other work stays usable after a panic: the caches and
/// queues of the pipeline hold their locks only around their own map operations, and a flight
/// whose maker panics is made again by the next caller.
pub fn isolated<T>(work: impl FnOnce() -> T) -> Result<T, String> {
    panic::catch_unwind(AssertUnwindSafe(work)).map_err(message)
}

fn message(payload: Box<dyn Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(text) => *text,
        Err(payload) => payload.downcast_ref::<&str>().map_or_else(
            || "a panic without a message".to_owned(),
            |text| (*text).to_owned(),
        ),
    }
}
