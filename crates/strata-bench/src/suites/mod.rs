//! Benchmark suites.

mod compaction;
mod compare;
mod compression;
mod decode;
mod ingest;
mod query;
mod recovery;

pub use compaction::run_compaction;
pub use compare::run_compare;
pub use compression::run_compression;
pub use decode::run_decode;
pub use ingest::run_ingest;
pub use query::run_query;
pub use recovery::run_recovery;

use crate::config::BenchConfig;
use crate::report::BenchReport;

/// Suite selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suite {
    /// Ingest throughput.
    Ingest,
    /// Compression size metrics (runs ingest first if needed).
    Compression,
    /// Full decode scan.
    Decode,
    /// Indexed / range queries with latency percentiles.
    Query,
    /// Crash recovery.
    Recovery,
    /// Keyed compaction.
    Compaction,
    /// Fair multi-format storage comparison.
    Compare,
}

impl Suite {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ingest => "ingest",
            Self::Compression => "compression",
            Self::Decode => "decode",
            Self::Query => "query",
            Self::Recovery => "recovery",
            Self::Compaction => "compaction",
            Self::Compare => "compare",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ingest" => Some(Self::Ingest),
            "compression" | "compress" => Some(Self::Compression),
            "decode" => Some(Self::Decode),
            "query" => Some(Self::Query),
            "recovery" | "recover" => Some(Self::Recovery),
            "compaction" | "compact" => Some(Self::Compaction),
            "compare" | "comparison" | "baseline" | "baselines" => Some(Self::Compare),
            _ => None,
        }
    }

    pub fn all() -> &'static [Suite] {
        &[
            Self::Ingest,
            Self::Compression,
            Self::Decode,
            Self::Query,
            Self::Recovery,
            Self::Compaction,
            Self::Compare,
        ]
    }
}

/// Runs one suite and returns a measured report.
pub fn run_suite(suite: Suite, config: &BenchConfig) -> anyhow::Result<BenchReport> {
    match suite {
        Suite::Ingest => run_ingest(config),
        Suite::Compression => run_compression(config),
        Suite::Decode => run_decode(config),
        Suite::Query => run_query(config),
        Suite::Recovery => run_recovery(config),
        Suite::Compaction => run_compaction(config),
        Suite::Compare => run_compare(config),
    }
}
