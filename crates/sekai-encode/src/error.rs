use std::fmt;

/// Why content could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum EncodeError {
    /// The encoder cannot run here: its library is not installed, has an unsupported version,
    /// or the encoder is not available on this target.
    Unavailable {
        /// The encoder name.
        encoder: &'static str,
        /// Why it is unavailable.
        message: String,
    },
    /// The encoder rejected the input or its parameters.
    Failed {
        /// The encoder name.
        encoder: &'static str,
        /// What failed.
        message: String,
    },
}

impl EncodeError {
    /// The fixed code of the error, as written in asset errors.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "encoder_unavailable",
            Self::Failed { .. } => "encode_failed",
        }
    }

    #[cfg(feature = "aac")]
    pub(crate) fn not_written(encoder: &'static str) -> Self {
        Self::Unavailable {
            encoder,
            message: "the encoder is not written yet".to_owned(),
        }
    }
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable { encoder, message } => write!(f, "{encoder} unavailable: {message}"),
            Self::Failed { encoder, message } => write!(f, "{encoder} failed: {message}"),
        }
    }
}

impl std::error::Error for EncodeError {}
