//! Crash-safe compaction for keyed datasets.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use strata_codec::{
    compressor_for, CompressionCodec, Compressor, EncodingPlan, IdentityCompressor,
};
use strata_core::{DurabilityMode, Record};
use strata_format::SegmentId;

use crate::dataset::{DatasetManifest, DatasetReader, DatasetWriterOptions, ManifestSegment};
use crate::durability::publish_file;
use crate::error::{Error, Result};
use crate::index::rebuild_indexes;
use crate::path::DatasetLayout;
use crate::segment::SegmentWriter;
use crate::versioning::resolve_latest;

/// Options controlling compaction rewrite.
#[derive(Clone)]
pub struct CompactOptions {
    /// Durability for publishing the new manifest.
    pub durability: DurabilityMode,
    /// Encoding plans applied when rewriting segments.
    pub encoding_plans: std::collections::HashMap<String, EncodingPlan>,
    /// Compressor for rewritten blocks.
    pub compressor: Arc<dyn Compressor>,
    /// Optional override for block row count (default: keep manifest value).
    pub block_row_count: Option<u32>,
    /// Optional override for segment target size.
    pub segment_target_size_bytes: Option<u64>,
}

impl Default for CompactOptions {
    fn default() -> Self {
        Self {
            durability: DurabilityMode::Durable,
            encoding_plans: Default::default(),
            compressor: Arc::new(IdentityCompressor),
            block_row_count: None,
            segment_target_size_bytes: None,
        }
    }
}

impl CompactOptions {
    /// Sets compression codec for rewritten segments.
    pub fn with_compression(mut self, codec: CompressionCodec, level: Option<i32>) -> Self {
        self.compressor = Arc::from(compressor_for(codec, level));
        self
    }
}

/// Outcome of a compaction run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactReport {
    /// Dataset root.
    pub root: PathBuf,
    /// Physical records before compaction.
    pub physical_records_before: u64,
    /// Live logical records written.
    pub logical_records_after: u64,
    /// Segments after compaction.
    pub segment_count: usize,
    /// New compaction generation.
    pub compaction_generation: u64,
    /// Whether indexes were rebuilt.
    pub indexes_rebuilt: bool,
}

impl CompactReport {
    /// CLI summary.
    pub fn to_cli_summary(&self) -> String {
        format!(
            "strata compact: {}\n  physical_before={}\n  logical_after={}\n  segments={}\n  generation={}\n  indexes_rebuilt={}\n",
            self.root.display(),
            self.physical_records_before,
            self.logical_records_after,
            self.segment_count,
            self.compaction_generation,
            self.indexes_rebuilt
        )
    }
}

/// Compacts a keyed dataset into new sealed segments.
///
/// Append-only datasets (empty `key_fields`) return an error — compaction is
/// not required and would be a no-op rewrite.
///
/// Crash safety:
/// 1. Write new segments + staging manifest under `compaction.building/`
/// 2. Move `segments/` → `segments.old/`
/// 3. Move `compaction.building/segments/` → `segments/`
/// 4. Atomically publish the new `manifest.json`
/// 5. Delete `segments.old/` and staging leftovers
/// 6. Rebuild indexes (including latest-version index)
///
/// If the process crashes before step 4, the previous manifest still points at
/// `segments.old` or the original `segments/` and remains readable after recovery
/// cleanup of incomplete staging dirs.
pub fn compact_dataset(root: impl AsRef<Path>, options: CompactOptions) -> Result<CompactReport> {
    let layout = DatasetLayout::new(root.as_ref());
    cleanup_incomplete_compaction(&layout)?;

    let reader = DatasetReader::open(&layout.root)?;
    let manifest = reader.manifest().clone();
    if !manifest.is_keyed() {
        return Err(Error::InvalidState {
            message:
                "compaction requires a keyed dataset; append-only datasets do not need compaction"
                    .into(),
        });
    }

    let physical_before = manifest.record_count;
    let all = reader.read_all_records()?;
    let live = resolve_latest(&manifest.key_fields, &all)?;

    let building = layout.compaction_building_dir();
    if building.exists() {
        fs::remove_dir_all(&building).map_err(|source| Error::Io {
            path: building.clone(),
            source,
        })?;
    }
    let building_segments = building.join("segments");
    fs::create_dir_all(&building_segments).map_err(|source| Error::Io {
        path: building_segments.clone(),
        source,
    })?;

    let block_rows = options
        .block_row_count
        .unwrap_or(manifest.block_row_count)
        .max(1);
    let seg_target = options
        .segment_target_size_bytes
        .unwrap_or(manifest.segment_target_size_bytes)
        .max(1);

    let mut next_ordinal = 0u32;
    let mut finished = Vec::new();
    let mut current: Option<SegmentWriter> = None;
    let mut written = 0u64;

    for rec in &live {
        if current.is_none() {
            let path = building_segments.join(format!("{next_ordinal:06}.strata"));
            let segment_id = SegmentId::new(format!("seg-{next_ordinal}"));
            current = Some(SegmentWriter::create_with_metrics(
                crate::path::SegmentPath::new(path),
                manifest.schema.clone(),
                segment_id,
                block_rows,
                None,
                options.encoding_plans.clone(),
                Arc::clone(&options.compressor),
                options.durability,
                16,
            )?);
            next_ordinal += 1;
        }
        let should_rotate = {
            let seg = current.as_mut().expect("segment");
            seg.write_records(std::slice::from_ref(rec))?;
            written += 1;
            seg.encoded_bytes_so_far() >= seg_target
        };
        if should_rotate {
            finished.push(finish_seg(current.take().expect("segment"))?);
        }
    }
    if let Some(writer) = current.take() {
        finished.push(finish_seg(writer)?);
    }

    // Empty live set: still write an empty valid dataset (zero segments).
    let new_manifest = DatasetManifest {
        manifest_version: DatasetManifest::VERSION,
        format_version: manifest.format_version,
        schema: manifest.schema.clone(),
        block_row_count: block_rows,
        segment_target_size_bytes: seg_target,
        segments: finished,
        record_count: written,
        key_fields: manifest.key_fields.clone(),
        compaction_generation: manifest.compaction_generation.saturating_add(1),
    };
    let manifest_text =
        serde_json::to_string_pretty(&new_manifest).map_err(|e| Error::InvalidState {
            message: format!("failed to serialize compacted manifest: {e}"),
        })?;
    let staging_manifest = building.join("manifest.json");
    publish_file(
        &staging_manifest,
        manifest_text.as_bytes(),
        options.durability,
    )?;

    // Cutover.
    let segments = layout.segments_dir();
    let segments_old = layout.segments_old_dir();
    if segments_old.exists() {
        fs::remove_dir_all(&segments_old).map_err(|source| Error::Io {
            path: segments_old.clone(),
            source,
        })?;
    }
    fs::rename(&segments, &segments_old).map_err(|source| Error::Io {
        path: segments.clone(),
        source,
    })?;
    fs::rename(&building_segments, &segments).map_err(|source| Error::Io {
        path: segments.clone(),
        source,
    })?;
    publish_file(
        &layout.manifest_path(),
        manifest_text.as_bytes(),
        options.durability,
    )?;

    let _ = fs::remove_dir_all(&segments_old);
    let _ = fs::remove_dir_all(&building);

    // Rebuild indexes (block indexes + latest-version index).
    rebuild_indexes(&layout.root)?;

    Ok(CompactReport {
        root: layout.root,
        physical_records_before: physical_before,
        logical_records_after: written,
        segment_count: new_manifest.segments.len(),
        compaction_generation: new_manifest.compaction_generation,
        indexes_rebuilt: true,
    })
}

fn finish_seg(writer: SegmentWriter) -> Result<ManifestSegment> {
    let file_name = writer
        .path()
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown.strata")
        .to_string();
    let meta = writer.finish()?;
    Ok(ManifestSegment {
        file: file_name,
        segment_id: meta.header.segment_id.as_str().to_string(),
        record_count: meta.header.record_count,
        block_count: meta.header.block_count,
    })
}

fn cleanup_incomplete_compaction(layout: &DatasetLayout) -> Result<()> {
    // If a previous crash left segments.old and no usable segments/, restore.
    let segments = layout.segments_dir();
    let segments_old = layout.segments_old_dir();
    if segments_old.exists() && !segments.exists() {
        fs::rename(&segments_old, &segments).map_err(|source| Error::Io {
            path: segments.clone(),
            source,
        })?;
    }
    let building = layout.compaction_building_dir();
    if building.exists() {
        let _ = fs::remove_dir_all(&building);
    }
    if segments_old.exists() && segments.exists() {
        // Cutover completed far enough that both exist; prefer live segments/,
        // drop the backup.
        let _ = fs::remove_dir_all(&segments_old);
    }
    Ok(())
}

/// Convenience: compact using the same writer options defaults as ingest.
pub fn compact_dataset_simple(root: impl AsRef<Path>) -> Result<CompactReport> {
    compact_dataset(root, CompactOptions::default())
}

/// Helper used by tests: create a writer options bundle for compaction rewrite.
pub fn compact_options_from_writer(opts: &DatasetWriterOptions) -> CompactOptions {
    CompactOptions {
        durability: opts.durability,
        encoding_plans: opts.encoding_plans.clone(),
        compressor: Arc::clone(&opts.compressor),
        block_row_count: Some(opts.block_row_count),
        segment_target_size_bytes: Some(opts.segment_target_size_bytes),
    }
}

/// Expose resolve for callers packing records.
pub fn logical_records(key_fields: &[String], physical: &[Record]) -> Result<Vec<Record>> {
    resolve_latest(key_fields, physical)
}
