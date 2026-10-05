//! Live and final metrics snapshots (canonical wire format).

use serde::{Deserialize, Serialize};

use crate::units::{
    Bytes, CompressionRatio, DurationMillis, MegabytesPerSec, PhaseDuration, RecordsPerSec,
};

/// Point-in-time metrics view shared by CLI and dashboard.
///
/// Produced by [`crate::MetricsHandle::snapshot`] (live) or
/// [`crate::MetricsHandle::finish`] (final).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricsSnapshot {
    /// `true` while ingest is still running; `false` for the final summary.
    pub live: bool,
    /// Wall-clock elapsed since metrics collection started.
    pub elapsed: DurationMillis,
    /// Records successfully processed into Strata storage.
    pub records_processed: u64,
    /// Records parsed from input (may equal processed on success).
    pub records_read: u64,
    /// Logical columnar payload bytes (raw / decoded estimate).
    pub logical_input_bytes: Bytes,
    /// Bytes after semantic encoding, before general compression.
    pub semantic_encoded_bytes: Bytes,
    /// Bytes after general compression (lz4/zstd); equals semantic when codec is none.
    pub compressed_bytes: Bytes,
    /// Actual bytes written to disk (headers, blocks, directories, footers).
    pub physical_bytes_written: Bytes,
    /// `logical_input_bytes / physical_bytes_written`, including storage framing.
    pub compression_ratio: CompressionRatio,
    /// Derived records/sec over elapsed wall time.
    pub records_per_sec: RecordsPerSec,
    /// Derived blocks/sec over elapsed wall time.
    pub blocks_per_sec: RecordsPerSec,
    /// Derived logical MB/s over elapsed wall time.
    pub logical_mb_per_sec: MegabytesPerSec,
    /// Derived physical disk MB/s over elapsed wall time.
    pub physical_mb_per_sec: MegabytesPerSec,
    /// Encode phase throughput: logical bytes / encode time.
    pub encode_mb_per_sec: MegabytesPerSec,
    /// Compress phase throughput: semantic bytes / compression time.
    pub compress_mb_per_sec: MegabytesPerSec,
    /// Write phase throughput: physical bytes / write time.
    pub write_mb_per_sec: MegabytesPerSec,
    /// Columnar blocks completed.
    pub blocks_completed: u64,
    /// Segments completed.
    pub segments_completed: u64,
    /// Input files fully completed.
    pub files_completed: u64,
    /// Input files opened/seen.
    pub files_seen: u64,
    /// Total files scheduled for this ingest (`0` if unknown).
    #[serde(default)]
    pub total_files: u64,
    /// Sum of source input file sizes (filesystem metadata).
    pub source_input_bytes: Bytes,
    /// Cumulative parse time.
    pub parse_time: PhaseDuration,
    /// Cumulative encode (PLAIN) time.
    pub encode_time: PhaseDuration,
    /// Cumulative general-compression time (`0` when compression is not run).
    pub compression_time: PhaseDuration,
    /// Cumulative disk write time.
    pub write_time: PhaseDuration,
    /// Path of the input file currently being processed, if any.
    pub current_input_file: Option<String>,
    /// Error count observed during the operation.
    pub errors: u64,
    /// Depth of the parser → encode queue (bounded pipeline).
    pub queue_depth_parse: u64,
    /// Depth of the encode → writer queue (bounded pipeline).
    pub queue_depth_encode: u64,
    /// Depth of the ordered writer reorder buffer.
    pub queue_depth_write: u64,
    /// Configured encode worker count (`0` when unused).
    pub worker_count: u64,
    /// Number of workers currently encoding a block.
    #[serde(default)]
    pub active_workers: u64,
    /// Encode-worker busy fraction over wall×workers (`0.0`–`1.0`).
    pub worker_utilization: f64,
}

impl MetricsSnapshot {
    /// Alias used in older call sites / docs for the frozen end-of-run view.
    pub fn into_final(mut self) -> Self {
        self.live = false;
        self
    }

    /// Compact live progress line for CLI stderr/stdout.
    pub fn to_cli_live(&self) -> String {
        let file = self.current_input_file.as_deref().unwrap_or("-");
        format!(
            "[live] records={recs} logical={logical} semantic={semantic} compressed={compressed} \
             ratio={ratio} elapsed={elapsed} rec/s={rps} blocks/s={bps} files={done}/{seen} \
             blocks={blocks} segs={segs} q(p/e/w)={qp}/{qe}/{qw} workers={workers} util={util:.0}% \
             file={file}",
            recs = self.records_processed,
            logical = self.logical_input_bytes.display_mib(),
            semantic = self.semantic_encoded_bytes.display_mib(),
            compressed = self.compressed_bytes.display_mib(),
            ratio = self.compression_ratio.display(),
            elapsed = self.elapsed.display(),
            rps = self.records_per_sec.display(),
            bps = self.blocks_per_sec.display(),
            done = self.files_completed,
            seen = if self.total_files > 0 {
                self.total_files
            } else {
                self.files_seen
            },
            blocks = self.blocks_completed,
            segs = self.segments_completed,
            qp = self.queue_depth_parse,
            qe = self.queue_depth_encode,
            qw = self.queue_depth_write,
            workers = self.worker_count,
            util = self.worker_utilization * 100.0,
            file = file,
        )
    }

    /// Final human-readable summary matching the canonical CLI layout.
    pub fn to_cli_summary(&self) -> String {
        format!(
            "Records:              {records}\n\
             Raw logical:          {logical} ({logical_bytes} bytes)\n\
             Semantic encoded:     {semantic} ({semantic_bytes} bytes)\n\
             Final compressed:     {compressed} ({compressed_bytes} bytes)\n\
             Physical on disk:     {physical} ({physical_bytes} bytes)\n\
             Compression ratio:    {ratio}\n\
             Elapsed:              {elapsed}\n\
             Records/sec:          {rps}\n\
             Blocks/sec:           {bps}\n\
             Logical throughput:   {logical_tp}\n\
             Encode throughput:    {encode_tp}\n\
             Compress throughput:  {compress_tp}\n\
             Write throughput:     {write_tp}\n\
             Disk throughput:      {disk_tp}\n\
             \n\
             Pipeline:\n\
               Workers:            {workers}\n\
               Worker utilization: {util:.1}%\n\
               Queue depths:  parse={qp} encode={qe} write={qw}\n\
             \n\
             Phase times:\n\
               Parse:              {parse}\n\
               Encode:             {encode}\n\
               Compression:        {compress}\n\
               Write:              {write}\n\
             \n\
             Files completed:      {files}\n\
             Blocks completed:     {blocks}\n\
             Segments completed:   {segments}\n\
             Errors:               {errors}",
            records = self.records_processed,
            logical = self.logical_input_bytes.display_mib(),
            logical_bytes = self.logical_input_bytes.get(),
            semantic = self.semantic_encoded_bytes.display_mib(),
            semantic_bytes = self.semantic_encoded_bytes.get(),
            compressed = self.compressed_bytes.display_mib(),
            compressed_bytes = self.compressed_bytes.get(),
            physical = self.physical_bytes_written.display_mib(),
            physical_bytes = self.physical_bytes_written.get(),
            ratio = self.compression_ratio.display(),
            elapsed = self.elapsed.display(),
            rps = self.records_per_sec.display(),
            bps = self.blocks_per_sec.display(),
            logical_tp = self.logical_mb_per_sec.display(),
            encode_tp = self.encode_mb_per_sec.display(),
            compress_tp = if self.compression_time.nanos == 0 {
                "n/a".to_string()
            } else {
                self.compress_mb_per_sec.display()
            },
            write_tp = self.write_mb_per_sec.display(),
            disk_tp = self.physical_mb_per_sec.display(),
            workers = self.worker_count,
            util = self.worker_utilization * 100.0,
            qp = self.queue_depth_parse,
            qe = self.queue_depth_encode,
            qw = self.queue_depth_write,
            parse = self.parse_time.display(),
            encode = self.encode_time.display(),
            compress = if self.compression_time.nanos == 0 {
                "0 ms (not enabled)".to_string()
            } else {
                self.compression_time.display()
            },
            write = self.write_time.display(),
            files = self.files_completed,
            blocks = self.blocks_completed,
            segments = self.segments_completed,
            errors = self.errors,
        )
    }
}

/// Versioned envelope for dashboard / API transport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricsReport {
    /// Schema version of this report document.
    pub report_version: u32,
    /// Operation that produced the metrics (e.g. `"ingest"`).
    pub operation: String,
    /// Snapshot payload.
    pub metrics: MetricsSnapshot,
}

impl MetricsReport {
    /// Current report document version.
    /// VERSION 5: physical-byte ratios, stable finals, file totals and active workers.
    pub const VERSION: u32 = 5;

    /// Creates a report for the given operation.
    pub fn new(operation: impl Into<String>, metrics: MetricsSnapshot) -> Self {
        Self {
            report_version: Self::VERSION,
            operation: operation.into(),
            metrics,
        }
    }

    /// CLI final summary.
    pub fn to_cli_summary(&self) -> String {
        format!(
            "operation={}\n{}",
            self.operation,
            self.metrics.to_cli_summary()
        )
    }
}

/// Back-compat alias: final metrics are snapshots with `live = false`.
pub type FinalMetrics = MetricsSnapshot;
