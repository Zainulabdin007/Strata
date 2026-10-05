//! Shared bench configuration and helpers.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use strata_codec::{compressor_for, CompressionCodec};
use strata_core::DurabilityMode;
use strata_metrics::MetricsHandle;
use strata_storage::{
    write_records_parallel, DatasetWriterOptions, PipelineOptions, SegmentReadMode,
};

use crate::gen::{generate_chunk, Workload};
use crate::meta::BenchMeta;
use crate::size::BenchSize;

/// Run configuration shared across suites.
#[derive(Debug, Clone)]
pub struct BenchConfig {
    /// Output directory for JSON/MD results.
    pub out_dir: PathBuf,
    /// Scratch directory for datasets.
    pub work_dir: PathBuf,
    /// Workload.
    pub workload: Workload,
    /// Size preset.
    pub size: BenchSize,
    /// RNG seed.
    pub seed: u64,
    /// Ingest workers.
    pub workers: usize,
    /// Block rows.
    pub block_size: u32,
    /// Segment target bytes.
    pub segment_size_bytes: u64,
    /// Compression codec.
    pub codec: CompressionCodec,
    /// Compression level.
    pub codec_level: Option<i32>,
    /// Segment read mode for decode/query.
    pub segment_read_mode: SegmentReadMode,
}

impl BenchConfig {
    /// Metadata snapshot.
    pub fn meta(&self) -> BenchMeta {
        BenchMeta::collect(
            self.seed,
            self.workload,
            self.size,
            self.workers,
            self.block_size,
            self.segment_size_bytes,
            self.codec.as_str(),
        )
    }

    /// Writer options for this run.
    pub fn writer_options(
        &self,
        metrics: Option<MetricsHandle>,
        keyed: bool,
    ) -> DatasetWriterOptions {
        DatasetWriterOptions {
            block_row_count: self.block_size,
            segment_target_size_bytes: self.segment_size_bytes,
            metrics,
            compressor: Arc::from(compressor_for(self.codec, self.codec_level)),
            durability: DurabilityMode::Fast,
            key_fields: if keyed {
                self.workload.key_fields()
            } else {
                Vec::new()
            },
            ..Default::default()
        }
    }

    /// Dataset path for a named suite.
    pub fn dataset_path(&self, suite: &str) -> PathBuf {
        self.work_dir.join(format!(
            "{}-{}-{}-{}",
            suite,
            self.workload.as_str(),
            self.size.as_str(),
            self.seed
        ))
    }
}

/// Streaming ingest of `config.size` records into `root`.
pub fn ingest_streaming(
    config: &BenchConfig,
    root: &std::path::Path,
    keyed: bool,
    metrics: Option<MetricsHandle>,
) -> anyhow::Result<Duration> {
    let schema = config.workload.schema();
    let mut opts = PipelineOptions::with_workers(config.workers);
    opts.writer = config.writer_options(metrics, keyed);

    let total = config.size.records();
    let chunk = config.block_size.max(1) as u64;
    let t0 = Instant::now();

    // For very large sizes, stream through the pipeline without materializing all records.
    use strata_storage::IngestPipeline;
    let mut pipeline = IngestPipeline::start(root, schema, opts)?;
    let mut buf = Vec::new();
    let mut start = 0u64;
    while start < total {
        let n = chunk.min(total - start);
        generate_chunk(config.workload, config.seed, start, n, &mut buf);
        pipeline.submit(std::mem::take(&mut buf))?;
        start += n;
    }
    pipeline.finish()?;
    Ok(t0.elapsed())
}

/// Convenience full materialization ingest (small sizes / tests).
#[allow(dead_code)]
pub fn ingest_all_at_once(
    config: &BenchConfig,
    root: &std::path::Path,
    keyed: bool,
) -> anyhow::Result<Duration> {
    let schema = config.workload.schema();
    let mut opts = PipelineOptions::with_workers(config.workers);
    opts.writer = config.writer_options(None, keyed);
    let total = config.size.records();
    let mut all = Vec::with_capacity(total.min(1_000_000) as usize);
    let mut buf = Vec::new();
    let mut start = 0u64;
    let chunk = 65_536u64;
    while start < total {
        let n = chunk.min(total - start);
        generate_chunk(config.workload, config.seed, start, n, &mut buf);
        all.append(&mut buf);
        start += n;
    }
    let t0 = Instant::now();
    write_records_parallel(root, schema, &all, opts)?;
    Ok(t0.elapsed())
}

/// Percentile from a sorted ascending sample (nearest-rank).
pub fn percentile(sorted_ms: &[f64], p: f64) -> Option<f64> {
    if sorted_ms.is_empty() {
        return None;
    }
    let rank = ((p / 100.0) * (sorted_ms.len() as f64 - 1.0)).round() as usize;
    Some(sorted_ms[rank.min(sorted_ms.len() - 1)])
}

/// Bytes/sec helpers.
pub fn per_sec(count: u64, elapsed: Duration) -> f64 {
    let secs = elapsed.as_secs_f64().max(1e-12);
    count as f64 / secs
}

pub fn gb_per_sec(bytes: u64, elapsed: Duration) -> f64 {
    per_sec(bytes, elapsed) / (1024.0 * 1024.0 * 1024.0)
}

pub fn mb_per_sec(bytes: u64, elapsed: Duration) -> f64 {
    per_sec(bytes, elapsed) / (1024.0 * 1024.0)
}
