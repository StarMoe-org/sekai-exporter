//! Errors of the table reader.

use std::fmt;

/// A TPK file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TpkError {
    message: String,
}

impl TpkError {
    /// An error that says what is wrong with the file.
    pub(crate) fn new(message: impl Into<String>) -> Self {
        TpkError {
            message: message.into(),
        }
    }

    /// What is wrong with the file.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TpkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TPK: {}", self.message)
    }
}

impl std::error::Error for TpkError {}
