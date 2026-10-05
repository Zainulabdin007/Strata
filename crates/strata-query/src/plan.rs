//! Typed query AST.

use serde::{Deserialize, Serialize};

/// A complete typed query against a Strata dataset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Query {
    /// Columns to return. Empty means all schema columns (`select *`).
    pub select: Vec<String>,
    /// Conjunctive predicates (`AND`). Empty means no row filter.
    pub predicates: Vec<Predicate>,
    /// Optional maximum rows to return.
    pub limit: Option<u64>,
}

impl Query {
    /// Full-scan query returning all columns.
    pub fn scan_all() -> Self {
        Self {
            select: Vec::new(),
            predicates: Vec::new(),
            limit: None,
        }
    }
}

/// One filter clause (combined with others via AND).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Predicate {
    /// `field == value`
    Eq {
        /// Field name.
        field: String,
        /// Comparison value.
        value: Literal,
    },
    /// `field != value`
    Ne {
        /// Field name.
        field: String,
        /// Comparison value.
        value: Literal,
    },
    /// `field < value`
    Lt {
        /// Field name.
        field: String,
        /// Comparison value.
        value: Literal,
    },
    /// `field <= value`
    Le {
        /// Field name.
        field: String,
        /// Comparison value.
        value: Literal,
    },
    /// `field > value`
    Gt {
        /// Field name.
        field: String,
        /// Comparison value.
        value: Literal,
    },
    /// `field >= value`
    Ge {
        /// Field name.
        field: String,
        /// Comparison value.
        value: Literal,
    },
    /// `field between lo and hi` (inclusive).
    Between {
        /// Field name.
        field: String,
        /// Lower bound (inclusive).
        low: Literal,
        /// Upper bound (inclusive).
        high: Literal,
    },
    /// `field in (a, b, …)`
    In {
        /// Field name.
        field: String,
        /// Allowed values.
        values: Vec<Literal>,
    },
}

impl Predicate {
    /// Field referenced by this predicate.
    pub fn field(&self) -> &str {
        match self {
            Self::Eq { field, .. }
            | Self::Ne { field, .. }
            | Self::Lt { field, .. }
            | Self::Le { field, .. }
            | Self::Gt { field, .. }
            | Self::Ge { field, .. }
            | Self::Between { field, .. }
            | Self::In { field, .. } => field.as_str(),
        }
    }
}

/// Literal value appearing in a predicate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Literal {
    /// Null.
    Null,
    /// Boolean.
    Bool(bool),
    /// Unsigned integer.
    U64(u64),
    /// Signed integer / timestamp / fixed-decimal scaled value.
    I64(i64),
    /// String (categorical / enum).
    String(String),
}

impl Literal {
    /// Display for explain / errors.
    pub fn display(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(b) => b.to_string(),
            Self::U64(n) => n.to_string(),
            Self::I64(n) => n.to_string(),
            Self::String(s) => format!("\"{s}\""),
        }
    }
}
