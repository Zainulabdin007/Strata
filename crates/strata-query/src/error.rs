//! Query engine errors.

use thiserror::Error;

/// Result alias.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from parsing or executing queries.
#[derive(Debug, Error)]
pub enum Error {
    /// Query text could not be parsed.
    #[error("query parse error: {message}")]
    Parse {
        /// Explanation.
        message: String,
    },

    /// Query is invalid against the dataset schema.
    #[error("invalid query: {message}")]
    Invalid {
        /// Explanation.
        message: String,
    },

    /// Storage / index failure.
    #[error(transparent)]
    Storage(#[from] strata_storage::Error),

    /// Underlying I/O (e.g. reading a query file).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Parse helper.
    pub fn parse(message: impl Into<String>) -> Self {
        Self::Parse {
            message: message.into(),
        }
    }

    /// Validation helper.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            message: message.into(),
        }
    }
}
