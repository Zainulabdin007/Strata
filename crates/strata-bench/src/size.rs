//! Benchmark size presets.

/// Supported record counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchSize {
    /// 100_000 records.
    N100k,
    /// 1_000_000 records.
    N1m,
    /// 10_000_000 records.
    N10m,
    /// 100_000_000 records.
    N100m,
}

impl BenchSize {
    /// Record count.
    pub const fn records(self) -> u64 {
        match self {
            Self::N100k => 100_000,
            Self::N1m => 1_000_000,
            Self::N10m => 10_000_000,
            Self::N100m => 100_000_000,
        }
    }

    /// CLI label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::N100k => "100k",
            Self::N1m => "1m",
            Self::N10m => "10m",
            Self::N100m => "100m",
        }
    }

    /// Parse CLI size.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "100k" | "100000" => Some(Self::N100k),
            "1m" | "1000000" => Some(Self::N1m),
            "10m" | "10000000" => Some(Self::N10m),
            "100m" | "100000000" => Some(Self::N100m),
            _ => None,
        }
    }
}
