//! Encode/decode micro-benchmarks for candidate plans.

use std::time::Instant;

use strata_codec::{decode_column, encode_column, EncodingPlan};
use strata_core::{FieldValue, LogicalType};

use crate::error::{Error, Result};
use crate::goal::OptimizeOptions;

/// Options for a single plan bench (usually from [`OptimizeOptions`]).
#[derive(Debug, Clone, Copy)]
pub struct BenchOptions {
    /// Minimum wall time spent in encode timing loop.
    pub min_bench_nanos: u64,
    /// Cap on encode/decode iterations.
    pub max_bench_iters: u32,
}

impl From<&OptimizeOptions> for BenchOptions {
    fn from(o: &OptimizeOptions) -> Self {
        Self {
            min_bench_nanos: o.min_bench_nanos,
            max_bench_iters: o.max_bench_iters,
        }
    }
}

/// Measured result for one encoding pipeline on a sample column.
#[derive(Debug, Clone, PartialEq)]
pub struct BenchResult {
    /// Plan under test.
    pub plan: EncodingPlan,
    /// Encoded payload size in bytes.
    pub encoded_bytes: u64,
    /// Estimated logical size of present values.
    pub logical_bytes: u64,
    /// Encode throughput (logical MB / encode second).
    pub encode_mb_s: f64,
    /// Decode throughput (logical MB / decode second).
    pub decode_mb_s: f64,
    /// Estimated peak temporary memory during encode.
    pub peak_memory_bytes: u64,
    /// Encode iterations performed for timing.
    pub encode_iters: u32,
    /// Decode iterations performed for timing.
    pub decode_iters: u32,
}

impl BenchResult {
    /// Compression ratio `logical / encoded` (higher is better).
    pub fn compression_ratio(&self) -> f64 {
        if self.encoded_bytes == 0 {
            return f64::INFINITY;
        }
        self.logical_bytes as f64 / self.encoded_bytes as f64
    }
}

/// Encodes and decodes `values` with `plan`, measuring size and throughput.
pub fn bench_plan(
    logical_type: &LogicalType,
    values: &[FieldValue],
    plan: &EncodingPlan,
    options: BenchOptions,
) -> Result<BenchResult> {
    if values.is_empty() {
        return Err(Error::Invalid {
            message: "cannot bench an empty sample".into(),
        });
    }

    let first = encode_column(logical_type, values, plan)?;
    let decoded = decode_column(logical_type, values.len() as u32, &first.bytes)?;
    if decoded != values {
        return Err(Error::Invalid {
            message: format!(
                "plan `{}` failed round-trip on sample ({} rows)",
                plan.steps
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join("→"),
                values.len()
            ),
        });
    }

    let encoded_bytes = first.encoded_size() as u64;
    let logical_bytes = first.logical_bytes;
    let peak_memory_bytes = estimate_peak_memory(logical_bytes, encoded_bytes, values.len());

    let (encode_iters, encode_nanos) = time_loop(options, || {
        encode_column(logical_type, values, plan).map(|v| {
            std::hint::black_box(v);
        })
    })?;
    let payload = first.bytes.clone();
    let row_count = values.len() as u32;
    let (decode_iters, decode_nanos) = time_loop(options, || {
        decode_column(logical_type, row_count, &payload).map(|v| {
            std::hint::black_box(v);
        })
    })?;

    let encode_mb_s = throughput_mb_s(logical_bytes, encode_iters, encode_nanos);
    let decode_mb_s = throughput_mb_s(logical_bytes, decode_iters, decode_nanos);

    Ok(BenchResult {
        plan: plan.clone(),
        encoded_bytes,
        logical_bytes,
        encode_mb_s,
        decode_mb_s,
        peak_memory_bytes,
        encode_iters,
        decode_iters,
    })
}

fn estimate_peak_memory(logical_bytes: u64, encoded_bytes: u64, row_count: usize) -> u64 {
    // Practical estimate: input values + working buffers + encoded output.
    let value_overhead = (row_count as u64).saturating_mul(24);
    logical_bytes
        .saturating_mul(2)
        .saturating_add(encoded_bytes)
        .saturating_add(value_overhead)
}

fn throughput_mb_s(logical_bytes: u64, iters: u32, nanos: u64) -> f64 {
    if nanos == 0 || iters == 0 {
        return f64::INFINITY;
    }
    let total_bytes = logical_bytes as f64 * iters as f64;
    let seconds = nanos as f64 / 1_000_000_000.0;
    (total_bytes / seconds) / (1024.0 * 1024.0)
}

fn time_loop<F>(options: BenchOptions, mut f: F) -> Result<(u32, u64)>
where
    F: FnMut() -> strata_codec::Result<()>,
{
    let started = Instant::now();
    let mut iters = 0u32;
    loop {
        f()?;
        iters += 1;
        let elapsed = started.elapsed().as_nanos() as u64;
        if elapsed >= options.min_bench_nanos || iters >= options.max_bench_iters {
            return Ok((iters, elapsed.max(1)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_codec::plan_examples;

    #[test]
    fn benches_timestamp_plan() {
        let ty = LogicalType::timestamp(strata_core::TimestampUnit::Nanoseconds);
        let values: Vec<_> = (0..512)
            .map(|i| FieldValue::Timestamp(1_000_000 + i * 382))
            .collect();
        let plan = plan_examples::timestamp();
        let result = bench_plan(
            &ty,
            &values,
            &plan,
            BenchOptions {
                min_bench_nanos: 100_000,
                max_bench_iters: 8,
            },
        )
        .unwrap();
        assert!(result.encoded_bytes > 0);
        assert!(result.encode_mb_s > 0.0);
        assert!(result.decode_mb_s > 0.0);
        assert!(result.peak_memory_bytes >= result.encoded_bytes);
    }
}
