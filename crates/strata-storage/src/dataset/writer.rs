//! DatasetWriter: spill record batches across segments.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use strata_codec::Compressor;
use strata_codec::{compressor_for, CompressionCodec, EncodingPlan, IdentityCompressor};
use strata_core::{DurabilityMode, Record, RecordBatch, Schema};
use strata_format::SegmentId;
use strata_metrics::MetricsHandle;

use super::manifest::{DatasetManifest, ManifestSegment};
use crate::durability::publish_file;
use crate::error::{Error, Result};
use crate::path::DatasetLayout;
use crate::segment::SegmentWriter;
use crate::versioning::{ensure_tombstone_field, mark_live, validate_key_fields};

/// Options controlling dataset writing.
#[derive(Clone)]
pub struct DatasetWriterOptions {
    /// Max rows per columnar block.
    pub block_row_count: u32,
    /// Soft target for encoded bytes per segment before rotating.
    pub segment_target_size_bytes: u64,
    /// Optional shared metrics handle (CLI / dashboard).
    pub metrics: Option<MetricsHandle>,
    /// Per-field encoding plans (from the compression profile).
    pub encoding_plans: HashMap<String, EncodingPlan>,
    /// General block compressor applied after semantic encoding.
    pub compressor: Arc<dyn Compressor>,
    /// Durability / fsync policy.
    pub durability: DurabilityMode,
    /// For [`DurabilityMode::Durable`]: sync every N blocks.
    pub sync_every_blocks: u32,
    /// Primary key fields for keyed datasets (empty = append-only).
    pub key_fields: Vec<String>,
}

impl Default for DatasetWriterOptions {
    fn default() -> Self {
        Self {
            block_row_count: 65_536,
            segment_target_size_bytes: 256 * 1024 * 1024,
            metrics: None,
            encoding_plans: HashMap::new(),
            compressor: Arc::new(IdentityCompressor),
            durability: DurabilityMode::Fast,
            sync_every_blocks: 16,
            key_fields: Vec::new(),
        }
    }
}

impl DatasetWriterOptions {
    /// Sets general compression from a codec name + optional level.
    pub fn with_compression(mut self, codec: CompressionCodec, level: Option<i32>) -> Self {
        self.compressor = Arc::from(compressor_for(codec, level));
        self
    }
}

/// Writes a Strata dataset directory (segments + manifest).
pub struct DatasetWriter {
    _lock: std::fs::File,
    layout: DatasetLayout,
    schema: Schema,
    options: DatasetWriterOptions,
    current: Option<SegmentWriter>,
    next_ordinal: u32,
    finished_segments: Vec<ManifestSegment>,
    total_records: u64,
    acknowledged_records: u64,
    finalized: bool,
}

impl DatasetWriter {
    /// Creates a new dataset at `root`.
    pub fn create(
        root: impl Into<PathBuf>,
        schema: Schema,
        options: DatasetWriterOptions,
    ) -> Result<Self> {
        if options.block_row_count == 0 {
            return Err(Error::InvalidState {
                message: "block_row_count must be > 0".into(),
            });
        }
        if options.segment_target_size_bytes == 0 {
            return Err(Error::InvalidState {
                message: "segment_target_size_bytes must be > 0".into(),
            });
        }

        let layout = DatasetLayout::new(root);
        layout.ensure_dirs()?;
        let lock = crate::writer_lock::lock_dataset(&layout.root)?;
        if layout.manifest_path().exists()
            || std::fs::read_dir(layout.segments_dir())
                .map_err(|source| Error::Io {
                    path: layout.segments_dir(),
                    source,
                })?
                .next()
                .is_some()
        {
            return Err(Error::InvalidState {
                message: "dataset already contains storage; refusing to overwrite".into(),
            });
        }

        let schema = if options.key_fields.is_empty() {
            schema
        } else {
            validate_key_fields(&schema, &options.key_fields)?;
            ensure_tombstone_field(schema)?
        };

        Ok(Self {
            _lock: lock,
            layout,
            schema,
            options,
            current: None,
            next_ordinal: 0,
            finished_segments: Vec::new(),
            total_records: 0,
            acknowledged_records: 0,
            finalized: false,
        })
    }

    /// Dataset root path.
    pub fn root(&self) -> &Path {
        &self.layout.root
    }

    /// Schema being written.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Records acknowledged under the configured durability mode.
    pub fn acknowledged_records(&self) -> u64 {
        let current = self
            .current
            .as_ref()
            .map(|s| s.acknowledged_records())
            .unwrap_or(0);
        self.acknowledged_records.saturating_add(current)
    }

    /// Writes records, creating/rotating segments as needed.
    pub fn write_records(&mut self, records: &[Record]) -> Result<()> {
        self.ensure_open()?;
        if records.is_empty() {
            return Ok(());
        }

        let keyed = !self.options.key_fields.is_empty();
        for chunk in records.chunks(self.options.block_row_count as usize) {
            self.ensure_segment()?;
            {
                let seg = self.current.as_mut().expect("segment opened");
                if keyed {
                    let live: Vec<Record> = chunk.iter().cloned().map(mark_live).collect();
                    seg.write_records(&live)?;
                } else {
                    seg.write_records(chunk)?;
                }
            }
            self.total_records += chunk.len() as u64;
            if let Some(m) = &self.options.metrics {
                m.add_records_processed(chunk.len() as u64);
            }
            self.maybe_rotate()?;
        }
        Ok(())
    }

    /// Writes a record batch.
    pub fn write_batch(&mut self, batch: &RecordBatch) -> Result<()> {
        if batch.schema != self.schema {
            return Err(Error::InvalidState {
                message: "batch schema does not match dataset schema".into(),
            });
        }
        self.write_records(&batch.records)
    }

    /// Appends a pre-encoded block (large sequential write) and may rotate segments.
    ///
    /// Used by the parallel ingest pipeline after concurrent encode/compress.
    /// `record_count` must match the number of logical rows in the block.
    pub fn write_encoded_block(
        &mut self,
        encoded: crate::block::EncodedBlock,
        record_count: u64,
    ) -> Result<()> {
        self.ensure_open()?;
        if record_count == 0 {
            return Ok(());
        }
        self.ensure_segment()?;
        {
            let seg = self.current.as_mut().expect("segment opened");
            seg.write_encoded_block(encoded)?;
        }
        self.total_records += record_count;
        if let Some(m) = &self.options.metrics {
            m.add_records_processed(record_count);
        }
        self.maybe_rotate()?;
        Ok(())
    }

    /// Finalizes open segments and writes `manifest.json`.
    pub fn finish(mut self) -> Result<DatasetManifest> {
        self.ensure_open()?;
        self.finalized = true;
        self.close_current()?;

        let manifest = DatasetManifest {
            manifest_version: DatasetManifest::VERSION,
            format_version: strata_format::FORMAT_VERSION,
            schema: self.schema.clone(),
            block_row_count: self.options.block_row_count,
            segment_target_size_bytes: self.options.segment_target_size_bytes,
            segments: self.finished_segments.clone(),
            record_count: self.total_records,
            key_fields: self.options.key_fields.clone(),
            compaction_generation: 0,
        };

        let text = serde_json::to_string_pretty(&manifest).map_err(|e| Error::InvalidState {
            message: format!("failed to serialize manifest: {e}"),
        })?;
        let manifest_path = self.layout.manifest_path();
        let bytes = text.as_bytes();
        let t0 = std::time::Instant::now();
        publish_file(&manifest_path, bytes, self.options.durability)?;
        if let Some(m) = &self.options.metrics {
            m.record_write(bytes.len() as u64, t0.elapsed());
        }
        self.acknowledged_records = self.total_records;

        Ok(manifest)
    }

    fn ensure_segment(&mut self) -> Result<()> {
        if self.current.is_some() {
            return Ok(());
        }
        let ordinal = self.next_ordinal;
        self.next_ordinal += 1;
        let path = self.layout.segment_path(ordinal);
        let segment_id = SegmentId::new(format!("seg-{ordinal}"));
        let writer = SegmentWriter::create_with_metrics(
            path,
            self.schema.clone(),
            segment_id,
            self.options.block_row_count,
            self.options.metrics.clone(),
            self.options.encoding_plans.clone(),
            Arc::clone(&self.options.compressor),
            self.options.durability,
            self.options.sync_every_blocks,
        )?;
        self.current = Some(writer);
        Ok(())
    }

    fn maybe_rotate(&mut self) -> Result<()> {
        let should_rotate = self
            .current
            .as_ref()
            .map(|s| s.encoded_bytes_so_far() >= self.options.segment_target_size_bytes)
            .unwrap_or(false);
        if should_rotate {
            self.close_current()?;
        }
        Ok(())
    }

    fn close_current(&mut self) -> Result<()> {
        if let Some(writer) = self.current.take() {
            let file_name = writer
                .path()
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown.strata")
                .to_string();
            let meta = writer.finish()?;
            self.acknowledged_records += meta.header.record_count;
            self.finished_segments.push(ManifestSegment {
                file: file_name,
                segment_id: meta.header.segment_id.as_str().to_string(),
                record_count: meta.header.record_count,
                block_count: meta.header.block_count,
            });
        }
        Ok(())
    }

    fn ensure_open(&self) -> Result<()> {
        if self.finalized {
            Err(Error::InvalidState {
                message: "dataset writer already finalized".into(),
            })
        } else {
            Ok(())
        }
    }
}
