//! Append updates and tombstones onto an existing keyed dataset.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use strata_codec::Compressor;
use strata_codec::{EncodingPlan, IdentityCompressor};
use strata_core::{DurabilityMode, FieldValue, Record, Schema};
use strata_format::SegmentId;
use strata_metrics::MetricsHandle;

use super::manifest::{DatasetManifest, ManifestSegment};
use super::reader::DatasetReader;
use crate::durability::publish_file;
use crate::error::{Error, Result};
use crate::path::DatasetLayout;
use crate::segment::SegmentWriter;
use crate::versioning::{
    is_tombstone, make_tombstone, mark_live, validate_key_fields, RecordKey, TOMBSTONE_FIELD,
};

/// Options when appending to an existing dataset.
#[derive(Clone)]
pub struct DatasetAppenderOptions {
    /// Optional metrics.
    pub metrics: Option<MetricsHandle>,
    /// Encoding plans for new segments.
    pub encoding_plans: HashMap<String, EncodingPlan>,
    /// Compressor for new segments.
    pub compressor: Arc<dyn Compressor>,
    /// Durability policy for new writes + manifest publish.
    pub durability: DurabilityMode,
    /// Sync every N blocks for durable mode.
    pub sync_every_blocks: u32,
}

impl Default for DatasetAppenderOptions {
    fn default() -> Self {
        Self {
            metrics: None,
            encoding_plans: HashMap::new(),
            compressor: Arc::new(IdentityCompressor),
            durability: DurabilityMode::Durable,
            sync_every_blocks: 16,
        }
    }
}

/// Appends new versions / tombstones without rewriting sealed segments.
pub struct DatasetAppender {
    _lock: std::fs::File,
    layout: DatasetLayout,
    manifest: DatasetManifest,
    options: DatasetAppenderOptions,
    current: Option<SegmentWriter>,
    next_ordinal: u32,
    new_segments: Vec<ManifestSegment>,
    appended_records: u64,
    finalized: bool,
}

impl DatasetAppender {
    /// Opens an existing dataset for append-only mutation.
    pub fn open(root: impl Into<PathBuf>, options: DatasetAppenderOptions) -> Result<Self> {
        let layout = DatasetLayout::new(root);
        let lock = crate::writer_lock::lock_dataset(&layout.root)?;
        let reader = DatasetReader::open(&layout.root)?;
        let manifest = reader.manifest().clone();
        if !manifest.is_keyed() {
            return Err(Error::InvalidState {
                message: "dataset is append-only (no key_fields); updates/deletes require keys"
                    .into(),
            });
        }
        validate_key_fields(&manifest.schema, &manifest.key_fields)?;
        if manifest.schema.field(TOMBSTONE_FIELD).is_none() {
            return Err(Error::InvalidState {
                message: format!(
                    "keyed dataset schema missing required tombstone field `{TOMBSTONE_FIELD}`"
                ),
            });
        }
        let next_ordinal = manifest.segments.len() as u32;
        Ok(Self {
            _lock: lock,
            layout,
            manifest,
            options,
            current: None,
            next_ordinal,
            new_segments: Vec::new(),
            appended_records: 0,
            finalized: false,
        })
    }

    /// Dataset root.
    pub fn root(&self) -> &Path {
        &self.layout.root
    }

    /// Schema (includes tombstone field).
    pub fn schema(&self) -> &Schema {
        &self.manifest.schema
    }

    /// Key fields.
    pub fn key_fields(&self) -> &[String] {
        &self.manifest.key_fields
    }

    /// Appends raw records (must already include `__deleted` when intended).
    pub fn append_records(&mut self, records: &[Record]) -> Result<()> {
        self.ensure_open()?;
        if records.is_empty() {
            return Ok(());
        }
        for chunk in records.chunks(self.manifest.block_row_count as usize) {
            self.ensure_segment()?;
            {
                let seg = self.current.as_mut().expect("segment");
                seg.write_records(chunk)?;
            }
            self.appended_records += chunk.len() as u64;
            self.maybe_rotate()?;
        }
        Ok(())
    }

    /// Appends a new live version of a keyed record (logical update / upsert).
    pub fn update(&mut self, record: Record) -> Result<()> {
        let _ = RecordKey::from_record(&record, &self.manifest.key_fields)?;
        let live = mark_live(record);
        self.append_records(&[live])
    }

    /// Appends a tombstone for the key carried by `key_record`.
    pub fn delete(&mut self, key_record: &Record) -> Result<()> {
        let tomb = make_tombstone(key_record, &self.manifest.key_fields, &self.manifest.schema)?;
        debug_assert!(is_tombstone(&tomb));
        self.append_records(&[tomb])
    }

    /// Appends a tombstone constructed from explicit key field values.
    pub fn delete_key(&mut self, key_values: &[(impl AsRef<str>, FieldValue)]) -> Result<()> {
        let mut key_rec = Record::new();
        for (name, value) in key_values {
            key_rec.insert(name.as_ref().to_string(), value.clone());
        }
        self.delete(&key_rec)
    }

    /// Seals new segments and publishes an updated manifest.
    pub fn finish(mut self) -> Result<DatasetManifest> {
        self.ensure_open()?;
        self.finalized = true;
        self.close_current()?;

        let mut segments = self.manifest.segments.clone();
        segments.extend(self.new_segments);
        let record_count = self.manifest.record_count + self.appended_records;
        let manifest = DatasetManifest {
            manifest_version: DatasetManifest::VERSION,
            format_version: self.manifest.format_version,
            schema: self.manifest.schema.clone(),
            block_row_count: self.manifest.block_row_count,
            segment_target_size_bytes: self.manifest.segment_target_size_bytes,
            segments,
            record_count,
            key_fields: self.manifest.key_fields.clone(),
            compaction_generation: self.manifest.compaction_generation,
        };
        let text = serde_json::to_string_pretty(&manifest).map_err(|e| Error::InvalidState {
            message: format!("failed to serialize manifest: {e}"),
        })?;
        publish_file(
            &self.layout.manifest_path(),
            text.as_bytes(),
            self.options.durability,
        )?;
        Ok(manifest)
    }

    fn ensure_segment(&mut self) -> Result<()> {
        if self.current.is_some() {
            return Ok(());
        }
        let ordinal = self.next_ordinal;
        self.next_ordinal += 1;
        // Manifest length is not a free filename: compaction and interrupted
        // appends leave gaps. Allocate a unique immutable segment name.
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| Error::InvalidState {
                message: e.to_string(),
            })?
            .as_nanos();
        let path = crate::path::SegmentPath::new(
            self.layout
                .segments_dir()
                .join(format!("a{nonce:x}-{ordinal:06}.strata")),
        );
        let segment_id = SegmentId::new(format!("seg-{ordinal}"));
        let writer = SegmentWriter::create_with_metrics(
            path,
            self.manifest.schema.clone(),
            segment_id,
            self.manifest.block_row_count,
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
        let should = self
            .current
            .as_ref()
            .map(|s| s.encoded_bytes_so_far() >= self.manifest.segment_target_size_bytes)
            .unwrap_or(false);
        if should {
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
            self.new_segments.push(ManifestSegment {
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
                message: "dataset appender already finalized".into(),
            })
        } else {
            Ok(())
        }
    }
}
