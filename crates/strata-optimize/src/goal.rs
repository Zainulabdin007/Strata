//! Optimization goals and constraints.

use serde::{Deserialize, Serialize};

/// What the optimizer should prioritize when selecting a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OptimizationGoal {
    /// Prefer the smallest encoded representation (among feasible plans).
    MaxCompression,
    /// Prefer the fastest encode throughput.
    MaxIngest,
    /// Prefer the fastest decode throughput.
    MaxDecode,
    /// Trade size vs encode/decode speed — not simply the smallest.
    Balanced,
}

impl OptimizationGoal {
    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaxCompression => "max_compression",
            Self::MaxIngest => "max_ingest",
            Self::MaxDecode => "max_decode",
            Self::Balanced => "balanced",
        }
    }

    /// Parse a CLI / TOML goal name.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "max_compression" | "compression" | "size" => Some(Self::MaxCompression),
            "max_ingest" | "ingest" | "encode" => Some(Self::MaxIngest),
            "max_decode" | "decode" => Some(Self::MaxDecode),
            "balanced" | "balance" => Some(Self::Balanced),
            _ => None,
        }
    }
}

/// Hard constraints applied before goal scoring.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OptimizationConstraints {
    /// Minimum encode throughput in megabytes/second (logical bytes / encode time).
    pub min_ingest_mb_s: Option<f64>,
    /// Minimum decode throughput in megabytes/second.
    pub min_decode_mb_s: Option<f64>,
    /// Maximum estimated temporary memory in bytes during encode.
    pub max_memory_bytes: Option<u64>,
    /// Optional target compression ratio `logical / encoded` (must be ≥ this).
    pub target_compression_ratio: Option<f64>,
}

/// Options controlling an optimize run.
#[derive(Debug, Clone)]
pub struct OptimizeOptions {
    /// Selection goal.
    pub goal: OptimizationGoal,
    /// Hard constraints.
    pub constraints: OptimizationConstraints,
    /// Rows used for sample blocks (defaults to profile block size, capped).
    pub sample_rows: Option<u32>,
    /// Block window for profiling local ranges.
    pub block_rows: Option<u32>,
    /// Minimum wall time to spend timing each encode/decode (nanos).
    pub min_bench_nanos: u64,
    /// Maximum encode/decode iterations when timing.
    pub max_bench_iters: u32,
}

impl Default for OptimizeOptions {
    fn default() -> Self {
        Self {
            goal: OptimizationGoal::Balanced,
            constraints: OptimizationConstraints::default(),
            sample_rows: None,
            block_rows: None,
            min_bench_nanos: 2_000_000, // 2ms
            max_bench_iters: 256,
        }
    }
}
