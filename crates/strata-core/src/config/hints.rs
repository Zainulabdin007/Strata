//! Semantic hints attached to profile columns.

use serde::{Deserialize, Serialize};

/// Flag-style semantic hints that guide future encoding choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticHint {
    /// Values tend to increase over time within a block/segment.
    Monotonic,
    /// Values are sorted (non-decreasing) in ingest order.
    Sorted,
    /// Domain is small relative to row count (dictionary-friendly).
    LowCardinality,
    /// Adjacent values are close; delta encoding is likely profitable.
    DeltaFriendly,
    /// Values use a fixed decimal scale / precision.
    FixedPrecision,
}

/// Numeric bound used by `expected_min` / `expected_max` hints.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NumericBound {
    /// Signed integer bound.
    Integer(i64),
    /// Floating bound (accepted from TOML floats).
    Float(f64),
}

impl NumericBound {
    /// Returns the value as `f64` for comparisons / display.
    pub fn as_f64(self) -> f64 {
        match self {
            Self::Integer(v) => v as f64,
            Self::Float(v) => v,
        }
    }
}
