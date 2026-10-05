//! Canonical runtime and final metrics for Strata.
//!
//! This crate is the **only** authoritative source for Strata performance and
//! storage metrics. The CLI and future Lab dashboard must both consume
//! [`MetricsSnapshot`] values produced here.
//!
//! The storage engine emits observations through [`MetricsHandle`] and never
//! formats display output itself.

#![deny(missing_docs)]

pub mod handle;
pub mod runtime;
pub mod snapshot;
pub mod storage;
pub use storage::StorageMetrics;
pub mod units;

pub use handle::MetricsHandle;
pub use runtime::MetricsCollector;
pub use snapshot::{FinalMetrics, MetricsReport, MetricsSnapshot};
pub use units::{
    Bytes, CompressionRatio, DurationMillis, MegabytesPerSec, PhaseDuration, RecordsPerSec,
};
