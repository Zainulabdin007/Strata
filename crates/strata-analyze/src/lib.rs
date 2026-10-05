//! Dataset profiler for Strata.
//!
//! Computes per-column statistics used later for automatic compression-plan
//! generation. Analysis is read-only and does not modify source data.

#![deny(missing_docs)]

pub mod error;
pub mod format;
pub mod report;
pub mod stats;

pub use error::{Error, Result};
pub use format::{format_human, format_json};
pub use report::{AnalysisReport, AnalyzeOptions, Profiler};
pub use stats::{
    ColumnStats, DeltaVarianceLabel, LocalRangeStats, NumericBound, RepetitionStats,
    RunLengthStats, StatScalar,
};
