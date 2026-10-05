//! Storage error types.

use std::path::PathBuf;

use thiserror::Error;

/// Result alias for storage operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from dataset / segment / block I/O.
#[derive(Debug, Error)]
pub enum Error {
    /// Underlying I/O failure.
    #[error("I/O error at `{}`: {source}", path.display())]
    Io {
        /// Path related to the failure, when known.
        path: PathBuf,
        /// Source I/O error.
        source: std::io::Error,
    },

    /// Dataset / segment path was invalid.
    #[error("invalid storage path: {message}")]
    InvalidPath {
        /// Explanation.
        message: String,
    },

    /// Writer was used incorrectly.
    #[error("invalid writer state: {message}")]
    InvalidState {
        /// Explanation.
        message: String,
    },

    /// On-disk bytes are truncated, malformed, or fail checksum verification.
    #[error("corrupt Strata data at `{}`: {message}", path.display())]
    Corrupt {
        /// Path of the corrupt file (or `<memory>` for in-memory blocks).
        path: PathBuf,
        /// Explanation.
        message: String,
    },

    /// Format version is not supported by this reader.
    #[error(
        "unsupported format version {}.{} (reader supports {}.{})",
        found_major,
        found_minor,
        supported_major,
        supported_minor
    )]
    UnsupportedVersion {
        /// File major version.
        found_major: u16,
        /// File minor version.
        found_minor: u16,
        /// Reader major version.
        supported_major: u16,
        /// Reader minor version.
        supported_minor: u16,
    },

    /// Core schema/record validation failure.
    #[error(transparent)]
    Core(#[from] strata_core::Error),

    /// Codec encode/decode failure.
    #[error(transparent)]
    Codec(#[from] strata_codec::Error),
}

impl Error {
    /// Helper for corrupt in-memory block bytes.
    pub fn corrupt_mem(message: impl Into<String>) -> Self {
        Self::Corrupt {
            path: PathBuf::from("<memory>"),
            message: message.into(),
        }
    }

    /// Helper for corrupt file bytes.
    pub fn corrupt(path: impl Into<PathBuf>, message: impl Into<String>) -> Self {
        Self::Corrupt {
            path: path.into(),
            message: message.into(),
        }
    }
}
