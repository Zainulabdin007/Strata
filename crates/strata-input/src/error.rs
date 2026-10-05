//! Input discovery and parse errors.

use std::path::PathBuf;

use thiserror::Error;

/// Result alias for input operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors from discovery and parsing.
#[derive(Debug, Error)]
pub enum Error {
    /// DBN framing, metadata, schema or decompression failure.
    #[error("invalid DBN at `{}`: {message}", path.display())]
    MalformedDbn {
        /// Source path.
        path: PathBuf,
        /// Detailed explanation.
        message: String,
    },
    /// Underlying I/O failure.
    #[error("I/O error at `{}`: {source}", path.display())]
    Io {
        /// Path related to the failure.
        path: PathBuf,
        /// Source error.
        source: std::io::Error,
    },

    /// Ingest path does not exist.
    #[error("input path does not exist: `{}`", path.display())]
    PathNotFound {
        /// Missing path.
        path: PathBuf,
    },

    /// No files matched the configured filters.
    #[error("no input files matched under `{}` (extensions: {extensions})", path.display())]
    NoFilesMatched {
        /// Search root.
        path: PathBuf,
        /// Extensions that were required.
        extensions: String,
    },

    /// File extension is not allowed for this profile.
    #[error(
        "unsupported extension for `{}`: found `.{found}`, expected one of: {expected}",
        path.display()
    )]
    UnsupportedExtension {
        /// File path.
        path: PathBuf,
        /// Extension that was found (without dot), or `<none>`.
        found: String,
        /// Allowed extensions.
        expected: String,
    },

    /// Declared input format has no parser yet.
    #[error("input format `{format}` is not implemented yet")]
    FormatNotImplemented {
        /// Format name.
        format: String,
    },

    /// CSV was malformed or could not be decoded.
    #[error("malformed CSV at `{}` (line {line}): {message}", path.display())]
    MalformedCsv {
        /// File path.
        path: PathBuf,
        /// 1-based line number when known (0 if unknown).
        line: u64,
        /// Explanation.
        message: String,
    },

    /// JSONL was malformed or could not be decoded.
    #[error("malformed JSONL at `{}` (line {line}): {message}", path.display())]
    MalformedJsonl {
        /// File path.
        path: PathBuf,
        /// 1-based line number.
        line: u64,
        /// Explanation.
        message: String,
    },

    /// A required schema column was missing from the input row.
    #[error("missing column `{column}` in `{}` (line {line})", path.display())]
    MissingColumn {
        /// File path.
        path: PathBuf,
        /// Line number.
        line: u64,
        /// Column name.
        column: String,
    },

    /// Value could not be coerced to the column logical type.
    #[error(
        "invalid value for column `{column}` in `{}` (line {line}): {message}",
        path.display()
    )]
    InvalidValue {
        /// File path.
        path: PathBuf,
        /// Line number.
        line: u64,
        /// Column name.
        column: String,
        /// Explanation.
        message: String,
    },

    /// Core validation error (schema / record).
    #[error(transparent)]
    Core(#[from] strata_core::Error),
}
