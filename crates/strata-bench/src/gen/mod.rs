//! Synthetic dataset generators.

mod logs;
mod market;
mod rng;
mod telemetry;

pub use logs::{generate_logs_chunk, logs_schema};
pub use market::{generate_market_chunk, market_schema};
pub use telemetry::{generate_telemetry_chunk, telemetry_schema};

use strata_core::{Record, Schema};

/// Which synthetic workload to generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Workload {
    /// Equity-style market events.
    Market,
    /// Device telemetry.
    Telemetry,
    /// Structured application logs.
    Logs,
}

impl Workload {
    /// CLI / report name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Market => "market",
            Self::Telemetry => "telemetry",
            Self::Logs => "logs",
        }
    }

    /// Parse CLI name.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "market" => Some(Self::Market),
            "telemetry" => Some(Self::Telemetry),
            "logs" | "log" => Some(Self::Logs),
            _ => None,
        }
    }

    /// Schema for this workload.
    pub fn schema(self) -> Schema {
        match self {
            Self::Market => market_schema(),
            Self::Telemetry => telemetry_schema(),
            Self::Logs => logs_schema(),
        }
    }

    /// Key fields for compaction benches (market uses `order_id`).
    pub fn key_fields(self) -> Vec<String> {
        match self {
            Self::Market => vec!["order_id".into()],
            Self::Telemetry => vec!["device_id".into(), "metric".into()],
            Self::Logs => vec!["request_id".into()],
        }
    }
}

/// Fills `out` with up to `count` records starting at global index `start`.
pub fn generate_chunk(
    workload: Workload,
    seed: u64,
    start: u64,
    count: u64,
    out: &mut Vec<Record>,
) {
    out.clear();
    match workload {
        Workload::Market => generate_market_chunk(seed, start, count, out),
        Workload::Telemetry => generate_telemetry_chunk(seed, start, count, out),
        Workload::Logs => generate_logs_chunk(seed, start, count, out),
    }
}
