//! Codec error types.

use thiserror::Error;

/// Result alias for codec operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from encoding or compression interfaces.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Error {
    /// Requested encoder / compressor is not available in this build.
    #[error("codec `{id}` is not implemented yet")]
    NotImplemented {
        /// Codec identifier.
        id: String,
    },

    /// Input was incompatible with the selected codec.
    #[error("codec input error: {message}")]
    InvalidInput {
        /// Explanation.
        message: String,
    },

    /// Encoding plan is invalid for the data or step order.
    #[error("invalid encoding plan: {message}")]
    InvalidPlan {
        /// Explanation.
        message: String,
    },

    /// Encoded bytes could not be decoded.
    #[error("codec decode error: {message}")]
    Decode {
        /// Explanation.
        message: String,
    },
}
