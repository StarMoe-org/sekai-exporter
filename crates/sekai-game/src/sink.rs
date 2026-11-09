//! Where the bytes of a download go.

use std::io::{self, Write};

/// A writer that can start again, for a transfer that is tried again.
pub trait Sink: Write {
    /// Forgets everything written so far.
    fn reset(&mut self) -> io::Result<()>;
}
