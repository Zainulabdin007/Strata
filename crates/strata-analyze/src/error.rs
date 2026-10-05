//! Profiler errors.

use thiserror::Error;

/// Result alias for analyze operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from dataset profiling.
#[derive(Debug, Error)]
pub enum Error {
    /// Schema / record validation failure from core.
    #[error(transparent)]
    Core(#[from] strata_core::Error),

    /// Profiler input was invalid.
    #[error("analyze error: {message}")]
    Invalid {
        /// Explanation.
        message: String,
    },
}
