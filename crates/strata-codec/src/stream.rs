//! Intermediate streams flowing through encoding pipelines.

use crate::error::{Error, Result};

/// In-memory stream between composable encoding steps.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stream {
    /// Signed integers (timestamps, signed ints, scaled decimals, deltas, …).
    I64(Vec<i64>),
    /// Unsigned integers (zigzag output, dictionary indices, FOR offsets, …).
    U64(Vec<u64>),
    /// Variable-length byte values (strings, binary, enum names before pack).
    Bytes(Vec<Vec<u8>>),
    /// Booleans.
    Bool(Vec<bool>),
    /// Final serialized bytes.
    Encoded(Vec<u8>),
}

impl Stream {
    /// Number of logical present values (or byte length for [`Stream::Encoded`]).
    pub fn len(&self) -> usize {
        match self {
            Self::I64(v) => v.len(),
            Self::U64(v) => v.len(),
            Self::Bytes(v) => v.len(),
            Self::Bool(v) => v.len(),
            Self::Encoded(v) => v.len(),
        }
    }

    /// Returns true when the stream has no present values (or empty bytes).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Expects an i64 stream.
    pub fn into_i64(self) -> Result<Vec<i64>> {
        match self {
            Self::I64(v) => Ok(v),
            other => Err(Error::InvalidPlan {
                message: format!("expected i64 stream, found {}", other.kind()),
            }),
        }
    }

    /// Expects a u64 stream.
    pub fn into_u64(self) -> Result<Vec<u64>> {
        match self {
            Self::U64(v) => Ok(v),
            other => Err(Error::InvalidPlan {
                message: format!("expected u64 stream, found {}", other.kind()),
            }),
        }
    }

    /// Expects a bytes stream.
    pub fn into_bytes_list(self) -> Result<Vec<Vec<u8>>> {
        match self {
            Self::Bytes(v) => Ok(v),
            other => Err(Error::InvalidPlan {
                message: format!("expected bytes stream, found {}", other.kind()),
            }),
        }
    }

    /// Expects a bool stream.
    pub fn into_bool(self) -> Result<Vec<bool>> {
        match self {
            Self::Bool(v) => Ok(v),
            other => Err(Error::InvalidPlan {
                message: format!("expected bool stream, found {}", other.kind()),
            }),
        }
    }

    /// Expects encoded bytes.
    pub fn into_encoded(self) -> Result<Vec<u8>> {
        match self {
            Self::Encoded(v) => Ok(v),
            other => Err(Error::InvalidPlan {
                message: format!("expected encoded stream, found {}", other.kind()),
            }),
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Self::I64(_) => "i64",
            Self::U64(_) => "u64",
            Self::Bytes(_) => "bytes",
            Self::Bool(_) => "bool",
            Self::Encoded(_) => "encoded",
        }
    }
}
