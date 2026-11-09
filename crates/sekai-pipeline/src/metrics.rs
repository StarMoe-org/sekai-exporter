//! The instrumentation points of the evaluator.
//!
//! The evaluator reports its work through a [`Recorder`]. It reads no clock itself, so a recorder
//! cannot influence any output, and a build without a clock, such as WebAssembly, uses
//! [`NoRecorder`].

/// A piece of work in progress. It ends when it is dropped, on the thread that started it.
pub trait Span {
    /// Adds to the bytes the work has read and written.
    fn bytes(&mut self, read: u64, written: u64);

    /// Reports the estimated and the measured peak memory of the work, in bytes.
    fn memory(&mut self, _estimated: u64, _actual: u64) {}
}

/// Receives the work of an evaluation.
pub trait Recorder: Sync {
    /// Starts a piece of work. `stage` is a fixed name, `label` comes from a small set such as
    /// the asset forms, and `subject` names the item, or is empty.
    fn span<'a>(&'a self, stage: &'static str, label: &str, subject: &str) -> Box<dyn Span + 'a>;
}

/// A [`Recorder`] that records nothing.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoRecorder;

impl Span for NoRecorder {
    fn bytes(&mut self, _read: u64, _written: u64) {}
}

impl Recorder for NoRecorder {
    fn span<'a>(&'a self, _: &'static str, _: &str, _: &str) -> Box<dyn Span + 'a> {
        Box::new(NoRecorder)
    }
}
