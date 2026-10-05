//! Shared metrics handle for engine + CLI/dashboard consumers.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::runtime::MetricsCollector;
use crate::snapshot::MetricsSnapshot;

/// Thread-safe handle to the canonical metrics collector.
///
/// The storage engine updates metrics through this handle without knowing
/// whether a CLI or dashboard is observing them.
#[derive(Clone, Debug)]
pub struct MetricsHandle {
    inner: Arc<Mutex<MetricsCollector>>,
}

impl MetricsHandle {
    /// Creates a new metrics session.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(MetricsCollector::start())),
        }
    }

    /// Runs `f` with exclusive access to the collector.
    pub fn with_mut<R>(&self, f: impl FnOnce(&mut MetricsCollector) -> R) -> R {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut guard)
    }

    /// Live snapshot for progress displays / dashboard polling.
    pub fn snapshot(&self) -> MetricsSnapshot {
        self.with_mut(|m| m.snapshot(true))
    }

    /// Final snapshot (consumes this handle's collector state via clone semantics:
    /// subsequent snapshots remain available; `live` is forced false).
    pub fn finish(&self) -> MetricsSnapshot {
        self.with_mut(|m| m.freeze())
    }

    /// Convenience: begin an input file.
    pub fn begin_input_file(&self, path: impl Into<String>, source_bytes: u64) {
        self.with_mut(|m| m.begin_input_file(path, source_bytes));
    }

    /// Convenience: finish the current input file.
    pub fn finish_input_file(&self) {
        self.with_mut(|m| m.finish_input_file());
    }

    /// Convenience: add parse duration.
    pub fn add_parse_time(&self, duration: Duration) {
        self.with_mut(|m| m.add_parse_time(duration));
    }

    /// Convenience: add records read.
    pub fn add_records_read(&self, count: u64) {
        self.with_mut(|m| m.add_records_read(count));
    }

    /// Convenience: add records processed.
    pub fn add_records_processed(&self, count: u64) {
        self.with_mut(|m| m.add_records_processed(count));
    }

    /// Convenience: record an error.
    pub fn add_error(&self) {
        self.with_mut(|m| m.add_error());
    }

    /// Encode observation from the storage engine.
    pub fn record_encode(&self, logical_bytes: u64, duration: Duration) {
        self.with_mut(|m| m.record_encode(logical_bytes, duration));
    }

    /// Block size breakdown after semantic encode + optional general compression.
    pub fn record_block_sizes(
        &self,
        logical_bytes: u64,
        semantic_encoded_bytes: u64,
        compressed_bytes: u64,
        encode_duration: Duration,
        compress_duration: Duration,
    ) {
        self.with_mut(|m| {
            m.record_block_sizes(
                logical_bytes,
                semantic_encoded_bytes,
                compressed_bytes,
                encode_duration,
                compress_duration,
            )
        });
    }

    /// Write observation from the storage engine.
    pub fn record_write(&self, physical_bytes: u64, duration: Duration) {
        self.with_mut(|m| m.record_write(physical_bytes, duration));
    }

    /// Block completion from the storage engine.
    pub fn record_block_completed(&self) {
        self.with_mut(|m| m.record_block_completed());
    }

    /// Segment completion from the storage engine.
    pub fn record_segment_completed(&self) {
        self.with_mut(|m| m.record_segment_completed());
    }

    /// Updates bounded-pipeline queue depth gauges.
    pub fn set_queue_depths(&self, parse: u64, encode: u64, write: u64) {
        self.with_mut(|m| m.set_queue_depths(parse, encode, write));
    }

    /// Records the configured encode worker count.
    pub fn set_worker_count(&self, workers: u64) {
        self.with_mut(|m| m.set_worker_count(workers));
    }

    /// Declares total input files for progress (files_completed / total_files).
    pub fn set_total_files(&self, total: u64) {
        self.with_mut(|m| m.set_total_files(total));
    }

    /// Accumulates encode-worker busy time.
    pub fn add_worker_busy_time(&self, duration: Duration) {
        self.with_mut(|m| m.add_worker_busy_time(duration));
    }
}

impl Default for MetricsHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_handle_updates_visible_in_snapshot() {
        let handle = MetricsHandle::new();
        handle.add_records_processed(5);
        handle.record_encode(100, Duration::from_millis(1));
        handle.record_write(50, Duration::from_millis(1));
        let snap = handle.snapshot();
        assert!(snap.live);
        assert_eq!(snap.records_processed, 5);
        assert_eq!(snap.logical_input_bytes.get(), 100);
        assert_eq!(snap.physical_bytes_written.get(), 50);
    }
}
