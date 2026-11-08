//! Errors of the reader.

use std::fmt;

/// The result type of this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// An error raised while reading a Unity file or an object.
///
/// Every error names what was being read, so that a report can say where reading stopped. The
/// object-level variants correspond to the error codes of the object document specification; see
/// [`Error::code`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A read needed more bytes than remain.
    Truncated {
        /// What was being read.
        what: &'static str,
        /// The offset at which the read started.
        offset: u64,
        /// The number of bytes the read needed.
        needed: u64,
        /// The number of bytes that remained.
        available: u64,
    },
    /// The bytes do not start with the expected signature.
    Signature {
        /// The format that was expected.
        expected: &'static str,
    },
    /// A value names a variant of the format that this reader does not support.
    Unsupported {
        /// What the value selects.
        what: &'static str,
        /// The stored value.
        value: u64,
    },
    /// A compressed block could not be decompressed to its declared size.
    Decompress {
        /// The index of the block.
        block: u32,
        /// The reason given by the decompressor.
        message: String,
    },
    /// A structure is inconsistent with itself, for example an offset outside the file.
    Malformed {
        /// The structure.
        what: &'static str,
        /// What is inconsistent.
        message: String,
    },
    /// No layout is available for an object's class.
    NoLayout {
        /// The class ID of the object.
        class_id: i32,
    },
    /// Reading by the layout went past the end of the object.
    ObjectTruncated {
        /// The field being read.
        field: String,
        /// The offset in the object at which the read started.
        offset: u64,
        /// The size of the object.
        size: u64,
    },
    /// The layout ended before the end of the object.
    TrailingBytes {
        /// The number of bytes the layout consumed.
        consumed: u64,
        /// The size of the object.
        size: u64,
    },
    /// The layout cannot be interpreted.
    InvalidLayout {
        /// What is wrong with it.
        message: String,
    },
    /// A `[SerializeReference]` entry names a type that the file's table of referenced types does
    /// not contain.
    UnresolvedReferenceType {
        /// The class name of the entry.
        class: String,
        /// The namespace of the entry.
        namespace: String,
        /// The assembly of the entry.
        assembly: String,
    },
    /// A visitor stopped reading.
    Visitor {
        /// The reason given by the visitor.
        message: String,
    },
}

impl Error {
    /// The stable code of the error, as written in object documents and reports.
    pub fn code(&self) -> &'static str {
        match self {
            Error::Truncated { .. } => "truncated_file",
            Error::Signature { .. } => "signature",
            Error::Unsupported { .. } => "unsupported",
            Error::Decompress { .. } => "decompress",
            Error::Malformed { .. } => "malformed",
            Error::NoLayout { .. } => "no_layout",
            Error::ObjectTruncated { .. } => "truncated",
            Error::TrailingBytes { .. } => "trailing_bytes",
            Error::InvalidLayout { .. } => "invalid_layout",
            Error::UnresolvedReferenceType { .. } => "unresolved_reference_type",
            Error::Visitor { .. } => "visitor",
        }
    }

    pub(crate) fn malformed(what: &'static str, message: impl Into<String>) -> Self {
        Error::Malformed {
            what,
            message: message.into(),
        }
    }

    pub(crate) fn invalid_layout(message: impl Into<String>) -> Self {
        Error::InvalidLayout {
            message: message.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated {
                what,
                offset,
                needed,
                available,
            } => write!(
                f,
                "{what}: needed {needed} bytes at offset {offset}, {available} available"
            ),
            Error::Signature { expected } => write!(f, "not a {expected} file"),
            Error::Unsupported { what, value } => write!(f, "unsupported {what}: {value}"),
            Error::Decompress { block, message } => {
                write!(f, "block {block} could not be decompressed: {message}")
            }
            Error::Malformed { what, message } => write!(f, "malformed {what}: {message}"),
            Error::NoLayout { class_id } => write!(f, "no layout for class ID {class_id}"),
            Error::ObjectTruncated {
                field,
                offset,
                size,
            } => write!(
                f,
                "field {field} at offset {offset} reads past the end of the {size}-byte object"
            ),
            Error::TrailingBytes { consumed, size } => write!(
                f,
                "the layout consumed {consumed} of {size} bytes of the object"
            ),
            Error::InvalidLayout { message } => write!(f, "invalid layout: {message}"),
            Error::UnresolvedReferenceType {
                class,
                namespace,
                assembly,
            } => write!(
                f,
                "referenced type {assembly}/{namespace}.{class} is not in the file's table of referenced types"
            ),
            Error::Visitor { message } => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for Error {}
