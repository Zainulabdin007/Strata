//! Common metric unit newtypes and derived rates.

use serde::{Deserialize, Serialize};

/// Byte count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Bytes(pub u64);

impl Bytes {
    /// Zero bytes.
    pub const ZERO: Self = Self(0);

    /// Returns the inner count.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Formats as mebibytes (binary) with 2 decimal places for display.
    pub fn display_mib(self) -> String {
        format!("{:.2} MiB", self.0 as f64 / (1024.0 * 1024.0))
    }

    /// Formats as megabytes (decimal) with 2 decimal places for throughput labels.
    pub fn as_mb(self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }
}

/// Duration stored as whole milliseconds for stable reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DurationMillis(pub u64);

impl DurationMillis {
    /// Zero duration.
    pub const ZERO: Self = Self(0);

    /// Returns the inner millisecond count.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Creates from nanoseconds (rounded down to whole milliseconds).
    pub fn from_nanos(nanos: u64) -> Self {
        Self(nanos / 1_000_000)
    }

    /// Seconds as `f64` for rate math.
    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / 1000.0
    }

    /// Human-readable duration.
    pub fn display(self) -> String {
        if self.0 < 1000 {
            format!("{} ms", self.0)
        } else {
            format!("{:.3} s", self.as_secs_f64())
        }
    }
}

/// Phase timer that keeps nanosecond precision for accurate reporting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PhaseDuration {
    /// Exact accumulated nanoseconds.
    pub nanos: u64,
}

impl PhaseDuration {
    /// Zero duration.
    pub const ZERO: Self = Self { nanos: 0 };

    /// Builds from raw nanoseconds.
    pub fn from_nanos(nanos: u64) -> Self {
        Self { nanos }
    }

    /// Whole milliseconds (floored).
    pub fn as_millis(self) -> u64 {
        self.nanos / 1_000_000
    }

    /// Display with us/ms/s as appropriate — never invents time, only formats
    /// measured nanos.
    pub fn display(self) -> String {
        if self.nanos == 0 {
            "0 ms".into()
        } else if self.nanos < 1_000_000 {
            format!("{} µs", self.nanos.div_ceil(1_000))
        } else if self.nanos < 1_000_000_000 {
            format!("{:.3} ms", self.nanos as f64 / 1_000_000.0)
        } else {
            format!("{:.3} s", self.nanos as f64 / 1_000_000_000.0)
        }
    }
}

/// Compression ratio as `logical / physical` when physical > 0.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CompressionRatio(pub f64);

impl CompressionRatio {
    /// Computes `logical / physical`.
    ///
    /// - both zero → `1.0`
    /// - physical zero, logical > 0 → `f64::INFINITY` (should not occur after a successful write)
    pub fn from_sizes(logical: Bytes, physical: Bytes) -> Self {
        if physical.0 == 0 {
            if logical.0 == 0 {
                Self(1.0)
            } else {
                Self(f64::INFINITY)
            }
        } else {
            Self(logical.0 as f64 / physical.0 as f64)
        }
    }

    /// Returns the inner ratio.
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Display with an `x` suffix.
    pub fn display(self) -> String {
        if self.0.is_infinite() {
            "inf".into()
        } else {
            format!("{:.4}x", self.0)
        }
    }
}

/// Throughput helper: bytes per second → MB/s.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MegabytesPerSec(pub f64);

impl MegabytesPerSec {
    /// Computes MB/s from byte count and elapsed duration.
    ///
    /// Returns `0.0` when elapsed is zero (avoids fabricating infinite rates).
    pub fn from_bytes(bytes: Bytes, elapsed: DurationMillis) -> Self {
        let secs = elapsed.as_secs_f64();
        if secs <= 0.0 {
            Self(0.0)
        } else {
            Self(bytes.as_mb() / secs)
        }
    }

    /// Computes MB/s from byte count and a phase timer in nanoseconds.
    pub fn from_bytes_and_nanos(bytes: Bytes, nanos: u64) -> Self {
        if nanos == 0 {
            Self(0.0)
        } else {
            let secs = nanos as f64 / 1_000_000_000.0;
            Self(bytes.as_mb() / secs)
        }
    }

    /// Returns the inner MB/s value.
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Display string.
    pub fn display(self) -> String {
        format!("{:.2} MB/s", self.0)
    }
}

/// Records processed per second.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct RecordsPerSec(pub f64);

impl RecordsPerSec {
    /// Computes records/sec. Returns `0.0` when elapsed is zero.
    pub fn from_counts(records: u64, elapsed: DurationMillis) -> Self {
        let secs = elapsed.as_secs_f64();
        if secs <= 0.0 {
            Self(0.0)
        } else {
            Self(records as f64 / secs)
        }
    }

    /// Rate from the measured nanosecond duration, including sub-millisecond jobs.
    pub fn from_nanos(records: u64, nanos: u64) -> Self {
        if nanos == 0 {
            Self(0.0)
        } else {
            Self(records as f64 * 1_000_000_000.0 / nanos as f64)
        }
    }

    /// Alias for block throughput (same unit math).
    pub fn from_blocks(blocks: u64, elapsed: DurationMillis) -> Self {
        Self::from_counts(blocks, elapsed)
    }

    /// Returns the inner value.
    pub const fn get(self) -> f64 {
        self.0
    }

    /// Display string.
    pub fn display(self) -> String {
        format!("{:.2}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratio_basic() {
        let r = CompressionRatio::from_sizes(Bytes(1000), Bytes(250));
        assert!((r.get() - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn throughput_zero_elapsed() {
        assert_eq!(
            MegabytesPerSec::from_bytes(Bytes(1_000_000), DurationMillis::ZERO).get(),
            0.0
        );
    }
}
