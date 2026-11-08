use std::fmt;

/// Why a payload could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
    /// The payload uses a format, codec or option that no decoder covers.
    Unsupported {
        /// What is unsupported, such as `"texture format"`.
        what: &'static str,
        /// The stored value that selects it.
        value: i64,
    },
    /// The payload ends before the data it declares.
    Truncated {
        /// What was being read.
        what: &'static str,
        /// The bytes needed.
        needed: usize,
        /// The bytes available.
        available: usize,
    },
    /// The payload contradicts itself.
    Malformed {
        /// What was being read.
        what: &'static str,
        /// What is wrong.
        message: String,
    },
}

impl DecodeError {
    /// The fixed code of the error, as written in asset errors.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unsupported { .. } => "unsupported",
            Self::Truncated { .. } => "truncated",
            Self::Malformed { .. } => "malformed",
        }
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported { what, value } => write!(f, "unsupported {what} {value}"),
            Self::Truncated {
                what,
                needed,
                available,
            } => write!(f, "{what}: needs {needed} bytes, {available} available"),
            Self::Malformed { what, message } => write!(f, "{what}: {message}"),
        }
    }
}

impl std::error::Error for DecodeError {}
