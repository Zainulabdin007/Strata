//! Optimizer errors.

use thiserror::Error;

/// Result alias for optimize operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from compression optimization.
#[derive(Debug, Error)]
pub enum Error {
    /// Schema / profile failure from core.
    #[error(transparent)]
    Core(#[from] strata_core::Error),

    /// Codec / encoding failure.
    #[error(transparent)]
    Codec(#[from] strata_codec::Error),

    /// Analyze / profiler failure.
    #[error(transparent)]
    Analyze(#[from] strata_analyze::Error),

    /// Optimizer input or selection failure.
    #[error("optimize error: {message}")]
    Invalid {
        /// Explanation.
        message: String,
    },
}
