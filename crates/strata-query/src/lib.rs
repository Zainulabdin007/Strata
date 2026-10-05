//! Typed query engine for Strata datasets.
//!
//! This is **not** SQL. See repository root `QUERY.md` for the Strata Query
//! Language (SQL-inspired but intentionally smaller).
//!
//! Features:
//! - column projection
//! - timestamp / sequence / numeric ranges
//! - categorical equality and `IN`
//! - `AND` combinations + `LIMIT`
//! - predicate pushdown, block skipping, selective column decode
//! - streaming [`RecordBatch`] results

#![deny(missing_docs)]

mod error;
mod exec;
mod parse;
mod plan;
mod predicate;

pub use error::{Error, Result};
pub use exec::{
    execute, execute_with_options, format_explain, ExplainPlan, QueryOptions, QueryStats,
    QueryStream,
};
pub use parse::parse_query;
pub use plan::{Literal, Predicate, Query};
pub use predicate::record_matches;
