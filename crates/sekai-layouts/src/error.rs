//! Errors of the table reader.

use std::fmt;

/// A table file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableError {
    message: String,
}

impl TableError {
    /// An error that says what is wrong with the file.
    pub(crate) fn new(message: impl Into<String>) -> Self {
        TableError {
            message: message.into(),
        }
    }

    /// What is wrong with the file.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "layout table: {}", self.message)
    }
}

impl std::error::Error for TableError {}
