//! Bounded parallel ingest pipeline.
//!
//! ```text
//! Input reader / parser (caller)
//!   → bounded parse→encode queue
//!   → N encode + compress workers
//!   → bounded encode→writer queue
//!   → ordered block assembly
//!   → sequential segment writer (large writes)
//! ```
//!
//! Independent blocks encode concurrently. Append order is deterministic by
//! sequence number. Queues are bounded (backpressure via blocking send).
//! Failures propagate; records are never silently dropped.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Instant;

use crossbeam_channel::{bounded, Receiver, Sender};
use strata_codec::{Compressor, EncodingPlan};
use strata_core::{Record, Schema};
use strata_format::BlockId;
use strata_metrics::MetricsHandle;

use crate::block::{encode_block, EncodedBlock};
use crate::dataset::{DatasetManifest, DatasetWriter, DatasetWriterOptions};
use crate::error::{Error, Result};
use crate::versioning::{ensure_tombstone_field, mark_live, validate_key_fields};

/// Configuration for the parallel ingest pipeline.
#[derive(Clone)]
pub struct PipelineOptions {
    /// Encode/compress worker threads (`>= 1`).
    pub workers: usize,
    /// Capacity of the parser → encode queue (in blocks). Primary backpressure bound.
    pub parse_queue_capacity: usize,
    /// Capacity of the encode→writer channel. Global permits bound reordering too.
    pub encode_queue_capacity: usize,
    /// Dataset writer options (block size, segment target, plans, compressor).
    pub writer: DatasetWriterOptions,
}

impl Default for PipelineOptions {
    fn default() -> Self {
        Self {
            workers: 1,
            parse_queue_capacity: 4,
            encode_queue_capacity: 4,
            writer: DatasetWriterOptions::default(),
        }
    }
}

impl PipelineOptions {
    /// Builds options with an explicit worker count and proportional queue bounds.
    pub fn with_workers(workers: usize) -> Self {
        let workers = workers.clamp(1, 32);
        Self {
            workers,
            parse_queue_capacity: (workers * 2).max(2),
            encode_queue_capacity: (workers * 2).max(2),
            writer: DatasetWriterOptions::default(),
        }
    }
}

struct EncodeJob {
    seq: u64,
    records: Vec<Record>,
}

struct EncodeResult {
    seq: u64,
    row_count: u64,
    outcome: std::result::Result<EncodedBlock, String>,
}

/// Parallel ingest handle: submit block-sized chunks, finish with ordered writes.
pub struct IngestPipeline {
    job_tx: Option<Sender<EncodeJob>>,
    credit_tx: Sender<()>,
    next_seq: u64,
    failed: Arc<AtomicBool>,
    metrics: Option<MetricsHandle>,
    encode_out_len: Arc<AtomicU64>,
    writer_pending: Arc<AtomicU64>,
    parse_depth: Arc<AtomicU64>,
    busy_nanos: Arc<AtomicU64>,
    writer_handle: Option<JoinHandle<Result<DatasetManifest>>>,
    worker_handles: Vec<JoinHandle<()>>,
    keyed: bool,
}

impl IngestPipeline {
    /// Starts encode workers and the ordered writer thread.
    pub fn start(
        root: impl Into<PathBuf>,
        schema: Schema,
        mut options: PipelineOptions,
    ) -> Result<Self> {
        if options.workers > 32
            || options.parse_queue_capacity > 128
            || options.encode_queue_capacity > 128
        {
            return Err(Error::InvalidState {
                message: "pipeline limits: 32 workers, 128 blocks per queue".into(),
            });
        }
        let workers = options.workers.max(1);
        options.workers = workers;
        options.parse_queue_capacity = options.parse_queue_capacity.max(1);
        options.encode_queue_capacity = options.encode_queue_capacity.max(1);

        let metrics = options.writer.metrics.clone();
        if let Some(m) = &metrics {
            m.set_worker_count(workers as u64);
            m.set_queue_depths(0, 0, 0);
        }

        let schema = if options.writer.key_fields.is_empty() {
            schema
        } else {
            validate_key_fields(&schema, &options.writer.key_fields)?;
            ensure_tombstone_field(schema)?
        };
        let keyed = !options.writer.key_fields.is_empty();

        let (job_tx, job_rx) = bounded::<EncodeJob>(options.parse_queue_capacity);
        // A permit covers submission through ordered write, including reorder buffers.
        // The writer always drains results, so a late low-sequence result cannot deadlock.
        let (enc_tx, enc_rx) = bounded::<EncodeResult>(options.encode_queue_capacity);
        let (credit_tx, credit_rx) =
            bounded::<()>(options.parse_queue_capacity + options.encode_queue_capacity + workers);
        let failed = Arc::new(AtomicBool::new(false));
        let encode_out_len = Arc::new(AtomicU64::new(0));
        let writer_pending = Arc::new(AtomicU64::new(0));
        let busy_nanos = Arc::new(AtomicU64::new(0));

        let plans = Arc::new(options.writer.encoding_plans.clone());
        let compressor = Arc::clone(&options.writer.compressor);

        let mut worker_handles = Vec::with_capacity(workers);
        for _ in 0..workers {
            let job_rx = job_rx.clone();
            let enc_tx = enc_tx.clone();
            let schema = schema.clone();
            let plans = Arc::clone(&plans);
            let compressor = Arc::clone(&compressor);
            let failed = Arc::clone(&failed);
            let busy_nanos = Arc::clone(&busy_nanos);
            let metrics = metrics.clone();
            let encode_out_len = Arc::clone(&encode_out_len);
            worker_handles.push(thread::spawn(move || {
                worker_loop(
                    job_rx,
                    enc_tx,
                    schema,
                    plans,
                    compressor,
                    failed,
                    busy_nanos,
                    metrics,
                    encode_out_len,
                );
            }));
        }
        drop(job_rx);
        drop(enc_tx);

        let writer_options = options.writer.clone();
        let writer_failed = Arc::clone(&failed);
        let writer_metrics = metrics.clone();
        let writer_pending_gauge = Arc::clone(&writer_pending);
        let encode_out_len_w = Arc::clone(&encode_out_len);
        let parse_depth = Arc::new(AtomicU64::new(0));
        let parse_depth_w = Arc::clone(&parse_depth);
        let root = root.into();
        let writer_handle = thread::spawn(move || {
            let failure = Arc::clone(&writer_failed);
            let result = writer_loop(
                root,
                schema,
                writer_options,
                enc_rx,
                writer_failed,
                writer_metrics,
                writer_pending_gauge,
                parse_depth_w,
                encode_out_len_w,
                credit_rx,
            );
            if result.is_err() {
                failure.store(true, Ordering::Release);
            }
            result
        });

        Ok(Self {
            job_tx: Some(job_tx),
            credit_tx,
            next_seq: 0,
            failed,
            metrics,
            encode_out_len,
            writer_pending,
            parse_depth,
            busy_nanos,
            writer_handle: Some(writer_handle),
            worker_handles,
            keyed,
        })
    }

    /// Submits one block of records for concurrent encode/compress.
    ///
    /// Blocks when the parse→encode queue is full (backpressure).
    pub fn submit(&mut self, records: Vec<Record>) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        if self.failed.load(Ordering::Acquire) {
            return Err(Error::InvalidState {
                message: "ingest pipeline failed; cannot submit more records".into(),
            });
        }
        let records = if self.keyed {
            records.into_iter().map(mark_live).collect()
        } else {
            records
        };
        let tx = self.job_tx.as_ref().ok_or_else(|| Error::InvalidState {
            message: "ingest pipeline already finished".into(),
        })?;
        self.credit_tx.send(()).map_err(|_| Error::InvalidState {
            message: "writer disconnected".into(),
        })?;
        let seq = self.next_seq;
        self.next_seq += 1;
        self.parse_depth.store(tx.len() as u64, Ordering::Relaxed);
        self.publish_queue_depths();
        tx.send(EncodeJob { seq, records })
            .map_err(|_| Error::InvalidState {
                message: "encode workers disconnected (pipeline failure)".into(),
            })?;
        self.parse_depth.store(tx.len() as u64, Ordering::Relaxed);
        self.publish_queue_depths();
        self.flush_busy_time();
        Ok(())
    }

    /// Closes input, drains workers, and finalizes the dataset.
    pub fn finish(mut self) -> Result<DatasetManifest> {
        // Close job channel so workers exit after draining.
        drop(self.job_tx.take());
        for handle in self.worker_handles.drain(..) {
            if handle.join().is_err() {
                self.failed.store(true, Ordering::Release);
            }
        }
        self.flush_busy_time();
        if self.failed.load(Ordering::Acquire) {
            if let Some(handle) = self.writer_handle.take() {
                let _ = handle.join();
            }
            return Err(Error::InvalidState {
                message: "ingest worker or writer failed".into(),
            });
        }
        self.writer_handle
            .take()
            .expect("writer handle")
            .join()
            .map_err(|_| Error::InvalidState {
                message: "writer thread panicked".into(),
            })?
    }

    fn publish_queue_depths(&self) {
        if let Some(m) = &self.metrics {
            m.set_queue_depths(
                self.parse_depth.load(Ordering::Relaxed),
                self.encode_out_len.load(Ordering::Relaxed),
                self.writer_pending.load(Ordering::Relaxed),
            );
        }
    }

    fn flush_busy_time(&self) {
        if let Some(m) = &self.metrics {
            let nanos = self.busy_nanos.swap(0, Ordering::Relaxed);
            if nanos > 0 {
                m.add_worker_busy_time(std::time::Duration::from_nanos(nanos));
            }
        }
    }
}

impl Drop for IngestPipeline {
    fn drop(&mut self) {
        if self.writer_handle.is_some() {
            self.failed.store(true, Ordering::Release);
        }
        drop(self.job_tx.take());
        for handle in self.worker_handles.drain(..) {
            let _ = handle.join();
        }
        if let Some(handle) = self.writer_handle.take() {
            let _ = handle.join();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn worker_loop(
    job_rx: Receiver<EncodeJob>,
    enc_tx: Sender<EncodeResult>,
    schema: Schema,
    plans: Arc<HashMap<String, EncodingPlan>>,
    compressor: Arc<dyn Compressor>,
    failed: Arc<AtomicBool>,
    busy_nanos: Arc<AtomicU64>,
    metrics: Option<MetricsHandle>,
    encode_out_len: Arc<AtomicU64>,
) {
    while let Ok(job) = job_rx.recv() {
        if failed.load(Ordering::Acquire) {
            break;
        }
        let row_count = job.records.len() as u64;
        if let Some(m) = &metrics {
            m.with_mut(|c| c.worker_active(true));
        }
        let t0 = Instant::now();
        let outcome = encode_block(
            &schema,
            &job.records,
            BlockId(0),
            plans.as_ref(),
            compressor.as_ref(),
        )
        .map_err(|e| e.to_string());
        let elapsed = t0.elapsed();
        if let Some(m) = &metrics {
            m.with_mut(|c| c.worker_active(false));
        }
        busy_nanos.fetch_add(elapsed.as_nanos() as u64, Ordering::Relaxed);

        if outcome.is_err() {
            failed.store(true, Ordering::Release);
            if let Some(m) = &metrics {
                m.add_error();
            }
        }

        encode_out_len.fetch_add(1, Ordering::Relaxed);
        if enc_tx
            .send(EncodeResult {
                seq: job.seq,
                row_count,
                outcome,
            })
            .is_err()
        {
            failed.store(true, Ordering::Release);
            break;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn writer_loop(
    root: PathBuf,
    schema: Schema,
    options: DatasetWriterOptions,
    enc_rx: Receiver<EncodeResult>,
    failed: Arc<AtomicBool>,
    metrics: Option<MetricsHandle>,
    writer_pending: Arc<AtomicU64>,
    parse_depth: Arc<AtomicU64>,
    encode_out_len: Arc<AtomicU64>,
    credit_rx: Receiver<()>,
) -> Result<DatasetManifest> {
    let mut writer = DatasetWriter::create(root, schema, options)?;
    let mut pending: BTreeMap<u64, EncodeResult> = BTreeMap::new();
    let mut next_seq = 0u64;

    for msg in &enc_rx {
        encode_out_len.fetch_sub(1, Ordering::Relaxed);
        pending.insert(msg.seq, msg);
        writer_pending.store(pending.len() as u64, Ordering::Relaxed);
        if let Some(m) = &metrics {
            m.set_queue_depths(
                parse_depth.load(Ordering::Relaxed),
                encode_out_len.load(Ordering::Relaxed),
                pending.len() as u64,
            );
        }

        while let Some(msg) = pending.remove(&next_seq) {
            writer_pending.store(pending.len() as u64, Ordering::Relaxed);
            match msg.outcome {
                Ok(encoded) => {
                    if let Some(m) = &metrics {
                        m.record_block_sizes(
                            encoded.meta.logical_bytes,
                            encoded.meta.semantic_encoded_bytes,
                            encoded.meta.compressed_bytes,
                            encoded.semantic_encode_duration,
                            encoded.compress_duration,
                        );
                    }
                    writer.write_encoded_block(encoded, msg.row_count)?;
                    let _ = credit_rx.recv();
                    next_seq += 1;
                }
                Err(err) => {
                    failed.store(true, Ordering::Release);
                    return Err(Error::InvalidState {
                        message: format!("encode worker failed on block {next_seq}: {err}"),
                    });
                }
            }
        }
    }

    if !pending.is_empty() {
        return Err(Error::InvalidState {
            message: format!(
                "writer shut down with {} out-of-order blocks still pending (next_seq={next_seq})",
                pending.len()
            ),
        });
    }

    if failed.load(Ordering::Acquire) {
        return Err(Error::InvalidState {
            message: "pipeline failed before completion".into(),
        });
    }
    if let Some(m) = &metrics {
        m.set_queue_depths(0, 0, 0);
    }
    writer.finish()
}

/// Convenience: encode+write records with the parallel pipeline in one call.
pub fn write_records_parallel(
    root: impl Into<PathBuf>,
    schema: Schema,
    records: &[Record],
    options: PipelineOptions,
) -> Result<DatasetManifest> {
    let block_rows = options.writer.block_row_count.max(1) as usize;
    let mut pipeline = IngestPipeline::start(root, schema, options)?;
    for chunk in records.chunks(block_rows) {
        pipeline.submit(chunk.to_vec())?;
    }
    pipeline.finish()
}
