//! Runtime metrics accumulation.
//!
//! The storage engine and CLI record observations here. Display concerns live
//! only in snapshot formatting — never in the collectors.

use std::time::{Duration, Instant};

use crate::snapshot::MetricsSnapshot;
use crate::units::{
    Bytes, CompressionRatio, DurationMillis, MegabytesPerSec, PhaseDuration, RecordsPerSec,
};

/// Accumulates authoritative counters and phase timers.
#[derive(Debug)]
pub struct MetricsCollector {
    started: Instant,
    finished: Option<Instant>,
    records_processed: u64,
    records_read: u64,
    logical_input_bytes: u64,
    semantic_encoded_bytes: u64,
    compressed_bytes: u64,
    physical_bytes_written: u64,
    blocks_completed: u64,
    segments_completed: u64,
    files_completed: u64,
    files_seen: u64,
    /// Declared total input files (0 until set by ingest orchestration).
    total_files: u64,
    source_input_bytes: u64,
    parse_nanos: u64,
    encode_nanos: u64,
    compression_nanos: u64,
    write_nanos: u64,
    errors: u64,
    current_input_file: Option<String>,
    queue_depth_parse: u64,
    queue_depth_encode: u64,
    queue_depth_write: u64,
    /// Cumulative worker busy nanoseconds across all encode workers.
    worker_busy_nanos: u64,
    /// Configured encode worker count (for utilization).
    worker_count: u64,
    active_workers: u64,
}

impl MetricsCollector {
    /// Starts a new collection window.
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
            finished: None,
            records_processed: 0,
            records_read: 0,
            logical_input_bytes: 0,
            semantic_encoded_bytes: 0,
            compressed_bytes: 0,
            physical_bytes_written: 0,
            blocks_completed: 0,
            segments_completed: 0,
            files_completed: 0,
            files_seen: 0,
            total_files: 0,
            source_input_bytes: 0,
            parse_nanos: 0,
            encode_nanos: 0,
            compression_nanos: 0,
            write_nanos: 0,
            errors: 0,
            current_input_file: None,
            queue_depth_parse: 0,
            queue_depth_encode: 0,
            queue_depth_write: 0,
            worker_busy_nanos: 0,
            worker_count: 0,
            active_workers: 0,
        }
    }

    /// Replace with the actual size of committed dataset files.
    pub fn set_physical_bytes(&mut self, bytes: u64) {
        self.physical_bytes_written = bytes;
    }
    /// Restore committed-file counters, excluding restart downtime.
    pub fn restore(&mut self, s: &MetricsSnapshot) {
        self.started = Instant::now()
            .checked_sub(Duration::from_millis(s.elapsed.get()))
            .unwrap_or_else(Instant::now);
        self.finished = None;
        self.records_processed = s.records_processed;
        self.records_read = s.records_read;
        self.logical_input_bytes = s.logical_input_bytes.get();
        self.semantic_encoded_bytes = s.semantic_encoded_bytes.get();
        self.compressed_bytes = s.compressed_bytes.get();
        self.physical_bytes_written = s.physical_bytes_written.get();
        self.blocks_completed = s.blocks_completed;
        self.segments_completed = s.segments_completed;
        self.files_completed = s.files_completed;
        self.files_seen = s.files_seen;
        self.total_files = s.total_files;
        self.source_input_bytes = s.source_input_bytes.get();
        self.parse_nanos = s.parse_time.nanos;
        self.encode_nanos = s.encode_time.nanos;
        self.compression_nanos = s.compression_time.nanos;
        self.write_nanos = s.write_time.nanos;
        self.current_input_file = None;
        self.queue_depth_parse = 0;
        self.queue_depth_encode = 0;
        self.queue_depth_write = 0;
        self.active_workers = 0;
    }

    /// Marks an input file as opened and records its source size.
    pub fn begin_input_file(&mut self, path: impl Into<String>, source_bytes: u64) {
        self.files_seen += 1;
        self.source_input_bytes += source_bytes;
        self.current_input_file = Some(path.into());
    }

    /// Marks the current input file as completed.
    pub fn finish_input_file(&mut self) {
        self.files_completed += 1;
        self.current_input_file = None;
    }

    /// Adds records observed at parse time.
    pub fn add_records_read(&mut self, count: u64) {
        self.records_read += count;
    }

    /// Adds records successfully written/processed into storage.
    pub fn add_records_processed(&mut self, count: u64) {
        self.records_processed += count;
    }

    /// Alias retained for older call sites.
    pub fn add_records_written(&mut self, count: u64) {
        self.add_records_processed(count);
    }

    /// Records that an input file of `bytes` was opened (without setting current path).
    pub fn add_input_file(&mut self, bytes: u64) {
        self.files_seen += 1;
        self.source_input_bytes += bytes;
    }

    /// Adds completed columnar blocks.
    pub fn add_blocks_completed(&mut self, count: u64) {
        self.blocks_completed += count;
    }

    /// Alias for [`add_blocks_completed`].
    pub fn add_blocks_written(&mut self, count: u64) {
        self.add_blocks_completed(count);
    }

    /// Adds completed segments.
    pub fn add_segments_completed(&mut self, count: u64) {
        self.segments_completed += count;
    }

    /// Alias for [`add_segments_completed`].
    pub fn add_segments_written(&mut self, count: u64) {
        self.add_segments_completed(count);
    }

    /// Adds logical payload bytes and physical on-disk bytes.
    pub fn add_bytes(&mut self, logical: u64, physical: u64) {
        self.logical_input_bytes += logical;
        self.physical_bytes_written += physical;
    }

    /// Adds only logical columnar bytes (pre-semantic-encode estimate).
    pub fn add_logical_bytes(&mut self, logical: u64) {
        self.logical_input_bytes += logical;
    }

    /// Adds semantic-encoded bytes (after Strata codecs, before general compression).
    pub fn add_semantic_encoded_bytes(&mut self, bytes: u64) {
        self.semantic_encoded_bytes += bytes;
    }

    /// Adds generally-compressed payload bytes (after lz4/zstd; equals semantic when none).
    pub fn add_compressed_bytes(&mut self, bytes: u64) {
        self.compressed_bytes += bytes;
    }

    /// Adds only physical disk bytes (write path).
    pub fn add_physical_bytes(&mut self, physical: u64) {
        self.physical_bytes_written += physical;
    }

    /// Accumulates measured parse duration.
    pub fn add_parse_time(&mut self, duration: Duration) {
        self.parse_nanos = self.parse_nanos.saturating_add(duration_as_nanos(duration));
    }

    /// Accumulates measured encode duration.
    pub fn add_encode_time(&mut self, duration: Duration) {
        self.encode_nanos = self
            .encode_nanos
            .saturating_add(duration_as_nanos(duration));
    }

    /// Accumulates measured compression duration.
    ///
    /// Callers must only record time actually spent in a compression stage.
    /// When compression is disabled, leave this at zero.
    pub fn add_compression_time(&mut self, duration: Duration) {
        self.compression_nanos = self
            .compression_nanos
            .saturating_add(duration_as_nanos(duration));
    }

    /// Accumulates measured write duration.
    pub fn add_write_time(&mut self, duration: Duration) {
        self.write_nanos = self.write_nanos.saturating_add(duration_as_nanos(duration));
    }

    /// Records a block encode observation (logical size + duration).
    pub fn record_encode(&mut self, logical_bytes: u64, duration: Duration) {
        self.add_logical_bytes(logical_bytes);
        self.add_encode_time(duration);
    }

    /// Records sizes for one block after semantic encode + optional general compression.
    pub fn record_block_sizes(
        &mut self,
        logical_bytes: u64,
        semantic_encoded_bytes: u64,
        compressed_bytes: u64,
        encode_duration: Duration,
        compress_duration: Duration,
    ) {
        self.add_logical_bytes(logical_bytes);
        self.add_semantic_encoded_bytes(semantic_encoded_bytes);
        self.add_compressed_bytes(compressed_bytes);
        self.add_encode_time(encode_duration);
        self.add_compression_time(compress_duration);
    }

    /// Records a disk write observation (physical size + duration).
    pub fn record_write(&mut self, physical_bytes: u64, duration: Duration) {
        self.add_physical_bytes(physical_bytes);
        self.add_write_time(duration);
    }

    /// Records one completed block after it has been encoded and written.
    pub fn record_block_completed(&mut self) {
        self.blocks_completed += 1;
    }

    /// Records one completed segment.
    pub fn record_segment_completed(&mut self) {
        self.segments_completed += 1;
    }

    /// Increments the error counter.
    pub fn add_error(&mut self) {
        self.errors += 1;
    }

    /// Reserved queue-depth setters for future pipelines.
    pub fn set_queue_depths(&mut self, parse: u64, encode: u64, write: u64) {
        self.queue_depth_parse = parse;
        self.queue_depth_encode = encode;
        self.queue_depth_write = write;
    }

    /// Sets the configured encode worker count used for utilization.
    pub fn set_worker_count(&mut self, workers: u64) {
        self.worker_count = workers;
    }

    /// Declares how many input files the current ingest will process.
    pub fn set_total_files(&mut self, total: u64) {
        self.total_files = total;
    }

    /// Track actual workers currently encoding a block.
    pub fn worker_active(&mut self, active: bool) {
        if active {
            self.active_workers += 1;
        } else {
            self.active_workers = self.active_workers.saturating_sub(1);
        }
    }

    /// Adds busy time observed by encode workers.
    pub fn add_worker_busy_time(&mut self, duration: Duration) {
        self.worker_busy_nanos = self
            .worker_busy_nanos
            .saturating_add(duration_as_nanos(duration));
    }

    /// Elapsed wall time since start.
    pub fn elapsed_millis(&self) -> DurationMillis {
        DurationMillis(
            u64::try_from(
                self.finished
                    .unwrap_or_else(Instant::now)
                    .duration_since(self.started)
                    .as_millis(),
            )
            .unwrap_or(u64::MAX),
        )
    }

    /// Builds a live or final snapshot from current counters.
    pub fn snapshot(&self, live: bool) -> MetricsSnapshot {
        let elapsed = self.elapsed_millis();
        let logical = Bytes(self.logical_input_bytes);
        let semantic = Bytes(self.semantic_encoded_bytes);
        let compressed = Bytes(self.compressed_bytes);
        let physical = Bytes(self.physical_bytes_written);
        let wall_nanos = self
            .finished
            .unwrap_or_else(Instant::now)
            .duration_since(self.started)
            .as_nanos() as u64;
        let worker_utilization = if self.worker_count == 0 || wall_nanos == 0 {
            0.0
        } else {
            let capacity = wall_nanos.saturating_mul(self.worker_count);
            (self.worker_busy_nanos as f64 / capacity as f64).clamp(0.0, 1.0)
        };
        MetricsSnapshot {
            live,
            elapsed,
            records_processed: self.records_processed,
            records_read: self.records_read,
            logical_input_bytes: logical,
            semantic_encoded_bytes: semantic,
            compressed_bytes: compressed,
            physical_bytes_written: physical,
            compression_ratio: CompressionRatio::from_sizes(logical, physical),
            records_per_sec: RecordsPerSec::from_nanos(self.records_processed, wall_nanos),
            blocks_per_sec: RecordsPerSec::from_nanos(self.blocks_completed, wall_nanos),
            logical_mb_per_sec: MegabytesPerSec::from_bytes_and_nanos(logical, wall_nanos),
            physical_mb_per_sec: MegabytesPerSec::from_bytes_and_nanos(physical, wall_nanos),
            encode_mb_per_sec: MegabytesPerSec::from_bytes_and_nanos(logical, self.encode_nanos),
            compress_mb_per_sec: MegabytesPerSec::from_bytes_and_nanos(
                semantic,
                self.compression_nanos,
            ),
            write_mb_per_sec: MegabytesPerSec::from_bytes_and_nanos(physical, self.write_nanos),
            blocks_completed: self.blocks_completed,
            segments_completed: self.segments_completed,
            files_completed: self.files_completed,
            files_seen: self.files_seen,
            total_files: self.total_files,
            source_input_bytes: Bytes(self.source_input_bytes),
            parse_time: PhaseDuration::from_nanos(self.parse_nanos),
            encode_time: PhaseDuration::from_nanos(self.encode_nanos),
            compression_time: PhaseDuration::from_nanos(self.compression_nanos),
            write_time: PhaseDuration::from_nanos(self.write_nanos),
            current_input_file: self.current_input_file.clone(),
            errors: self.errors,
            queue_depth_parse: self.queue_depth_parse,
            queue_depth_encode: self.queue_depth_encode,
            queue_depth_write: self.queue_depth_write,
            worker_count: self.worker_count,
            active_workers: self.active_workers,
            worker_utilization,
        }
    }

    /// Freezes the collector into a final snapshot.
    pub fn finish(mut self) -> MetricsSnapshot {
        self.freeze()
    }

    /// Freeze wall time so later observers see the same final snapshot.
    pub fn freeze(&mut self) -> MetricsSnapshot {
        self.finished.get_or_insert_with(Instant::now);
        self.snapshot(false)
    }
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::start()
    }
}

fn duration_as_nanos(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_computes_ratio_and_phases() {
        let mut c = MetricsCollector::start();
        c.begin_input_file("a.csv", 100);
        c.add_records_read(10);
        c.add_records_processed(10);
        c.record_encode(1000, Duration::from_millis(5));
        c.record_write(250, Duration::from_millis(2));
        c.record_block_completed();
        c.record_segment_completed();
        c.finish_input_file();
        let m = c.finish();
        assert_eq!(m.records_processed, 10);
        // No compressed_bytes recorded via record_encode alone → ratio uses physical.
        assert!((m.compression_ratio.get() - 4.0).abs() < f64::EPSILON);
        assert_eq!(m.encode_time.as_millis(), 5);
        assert_eq!(m.write_time.as_millis(), 2);
        assert_eq!(m.compression_time.nanos, 0);
        assert_eq!(m.files_completed, 1);
        assert!(!m.live);
    }
}
