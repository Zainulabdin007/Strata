//! Shared error types for `strata-core`.

use std::path::PathBuf;

use thiserror::Error;

/// Result alias for core operations.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors originating from schema, record, or configuration validation.
#[derive(Debug, Error)]
pub enum Error {
    /// Schema contained no fields.
    #[error("schema must contain at least one field")]
    EmptySchema,

    /// Two fields shared the same name.
    #[error("duplicate field name `{name}`")]
    DuplicateField {
        /// Conflicting field name.
        name: String,
    },

    /// Enum type declared with zero variants.
    #[error("enum field `{field}` must declare at least one variant")]
    EmptyEnum {
        /// Field name.
        field: String,
    },

    /// Fixed decimal scale exceeded precision.
    #[error("fixed decimal field `{field}` has scale {scale} greater than precision {precision}")]
    InvalidFixedDecimal {
        /// Field name.
        field: String,
        /// Declared precision.
        precision: u8,
        /// Declared scale.
        scale: u8,
    },

    /// Record was missing a required schema field.
    #[error("missing field `{field}`")]
    MissingField {
        /// Expected field name.
        field: String,
    },

    /// Record contained a field not in the schema.
    #[error("unknown field `{field}`")]
    UnknownField {
        /// Unexpected field name.
        field: String,
    },

    /// Null supplied for a non-nullable field.
    #[error("null not allowed for field `{field}`")]
    NullNotAllowed {
        /// Field name.
        field: String,
    },

    /// Value kind did not match the field's logical type.
    #[error("type mismatch for field `{field}`: expected {expected}, got {actual}")]
    TypeMismatch {
        /// Field name.
        field: String,
        /// Expected logical type name.
        expected: String,
        /// Actual value kind name.
        actual: String,
    },

    /// Value exceeds the declared width or size limit.
    #[error("invalid value for field `{field}`: {reason}")]
    InvalidValue {
        /// Field name.
        field: String,
        /// Violated constraint.
        reason: String,
    },

    /// Enum value was not in the declared variant set.
    #[error("invalid enum variant `{variant}` for field `{field}`")]
    InvalidEnumVariant {
        /// Field name.
        field: String,
        /// Provided variant.
        variant: String,
    },

    /// Compression profile TOML could not be read from disk.
    #[error("failed to read compression profile `{}`: {source}", path.display())]
    ProfileIo {
        /// Profile path.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// Compression profile TOML failed to parse.
    #[error("invalid compression profile TOML at `{}`: {source}", path.display())]
    ProfileParse {
        /// Profile path.
        path: PathBuf,
        /// TOML error.
        source: toml::de::Error,
    },

    /// Compression profile parsed but failed semantic validation.
    #[error("invalid compression profile at `{}`: {message}", path.display())]
    ProfileInvalid {
        /// Profile path (may be a placeholder for in-memory loads).
        path: PathBuf,
        /// Human-readable explanation.
        message: String,
    },

    /// Column declaration could not be converted into a logical type.
    #[error("invalid column `{column}`: {message}")]
    InvalidColumn {
        /// Column name.
        column: String,
        /// Explanation.
        message: String,
    },
}

impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        use Error::*;
        match (self, other) {
            (EmptySchema, EmptySchema) => true,
            (
                InvalidValue {
                    field: a,
                    reason: ar,
                },
                InvalidValue {
                    field: b,
                    reason: br,
                },
            ) => a == b && ar == br,
            (DuplicateField { name: a }, DuplicateField { name: b }) => a == b,
            (EmptyEnum { field: a }, EmptyEnum { field: b }) => a == b,
            (
                InvalidFixedDecimal {
                    field: a,
                    precision: ap,
                    scale: as_,
                },
                InvalidFixedDecimal {
                    field: b,
                    precision: bp,
                    scale: bs,
                },
            ) => a == b && ap == bp && as_ == bs,
            (MissingField { field: a }, MissingField { field: b }) => a == b,
            (UnknownField { field: a }, UnknownField { field: b }) => a == b,
            (NullNotAllowed { field: a }, NullNotAllowed { field: b }) => a == b,
            (
                TypeMismatch {
                    field: a,
                    expected: ae,
                    actual: aa,
                },
                TypeMismatch {
                    field: b,
                    expected: be,
                    actual: ba,
                },
            ) => a == b && ae == be && aa == ba,
            (
                InvalidEnumVariant {
                    field: a,
                    variant: av,
                },
                InvalidEnumVariant {
                    field: b,
                    variant: bv,
                },
            ) => a == b && av == bv,
            (
                ProfileInvalid {
                    path: a,
                    message: am,
                },
                ProfileInvalid {
                    path: b,
                    message: bm,
                },
            ) => a == b && am == bm,
            (
                InvalidColumn {
                    column: a,
                    message: am,
                },
                InvalidColumn {
                    column: b,
                    message: bm,
                },
            ) => a == b && am == bm,
            (ProfileIo { path: a, .. }, ProfileIo { path: b, .. }) => a == b,
            (ProfileParse { path: a, .. }, ProfileParse { path: b, .. }) => a == b,
            _ => false,
        }
    }
}

impl Eq for Error {}
