//! DatasetReader: reopen a dataset and decode all records.

use std::path::{Path, PathBuf};

use strata_core::{Record, RecordBatch, Schema};

use super::manifest::DatasetManifest;
use crate::error::{Error, Result};
use crate::mmap::SegmentReadMode;
use crate::path::{DatasetLayout, SegmentPath};
use crate::segment::SegmentReader;

/// Reads a Strata dataset directory.
#[derive(Debug)]
pub struct DatasetReader {
    layout: DatasetLayout,
    manifest: DatasetManifest,
    segment_read_mode: SegmentReadMode,
}

impl DatasetReader {
    /// Opens a dataset by reading `manifest.json` (mmap auto for sealed segments).
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        Self::open_with_mode(root, SegmentReadMode::Auto)
    }

    /// Opens a dataset with an explicit sealed-segment read mode.
    pub fn open_with_mode(
        root: impl Into<PathBuf>,
        segment_read_mode: SegmentReadMode,
    ) -> Result<Self> {
        let layout = DatasetLayout::new(root);
        let manifest_path = layout.manifest_path();
        let bytes = crate::io::read_file_with_limit(&manifest_path, 16 * 1024 * 1024)?;
        let manifest: DatasetManifest = serde_json::from_slice(&bytes).map_err(|e| {
            Error::corrupt(
                &manifest_path,
                format!("invalid dataset manifest JSON: {e}"),
            )
        })?;
        if manifest.manifest_version == 0
            || manifest.manifest_version > DatasetManifest::VERSION
            || !strata_format::FORMAT_VERSION.is_compatible_with(manifest.format_version)
        {
            return Err(Error::corrupt(
                &manifest_path,
                "unsupported manifest version",
            ));
        }
        let mut names = std::collections::HashSet::new();
        for segment in &manifest.segments {
            if segment.file.is_empty()
                || Path::new(&segment.file).components().count() != 1
                || !matches!(
                    Path::new(&segment.file).components().next(),
                    Some(std::path::Component::Normal(_))
                )
                || !names.insert(&segment.file)
            {
                return Err(Error::corrupt(
                    &manifest_path,
                    "invalid or duplicate segment filename",
                ));
            }
        }
        let total = manifest
            .segments
            .iter()
            .try_fold(0u64, |sum, segment| sum.checked_add(segment.record_count));
        if total != Some(manifest.record_count) {
            return Err(Error::corrupt(
                &manifest_path,
                "manifest record count mismatch",
            ));
        }
        manifest.schema.validate()?;
        Ok(Self {
            layout,
            manifest,
            segment_read_mode,
        })
    }

    /// Dataset root.
    pub fn root(&self) -> &Path {
        &self.layout.root
    }

    /// Loaded manifest.
    pub fn manifest(&self) -> &DatasetManifest {
        &self.manifest
    }

    /// Dataset schema.
    pub fn schema(&self) -> &Schema {
        &self.manifest.schema
    }

    /// Sealed-segment byte source mode.
    pub fn segment_read_mode(&self) -> SegmentReadMode {
        self.segment_read_mode
    }

    /// Number of segments.
    pub fn segment_count(&self) -> usize {
        self.manifest.segments.len()
    }

    /// Opens a segment reader by index using this reader's read mode.
    pub fn open_segment(&self, index: usize) -> Result<SegmentReader> {
        self.open_segment_with_mode(index, self.segment_read_mode)
    }

    /// Opens a segment reader by index with an explicit read mode.
    pub fn open_segment_with_mode(
        &self,
        index: usize,
        mode: SegmentReadMode,
    ) -> Result<SegmentReader> {
        let entry = self
            .manifest
            .segments
            .get(index)
            .ok_or_else(|| Error::InvalidPath {
                message: format!(
                    "segment index {index} out of range (count={})",
                    self.manifest.segments.len()
                ),
            })?;
        let path = SegmentPath::new(self.layout.segments_dir().join(&entry.file));
        let canonical = path.path.canonicalize().map_err(|source| Error::Io {
            path: path.path.clone(),
            source,
        })?;
        let root = self
            .layout
            .segments_dir()
            .canonicalize()
            .map_err(|source| Error::Io {
                path: self.layout.segments_dir(),
                source,
            })?;
        if !canonical.starts_with(&root) {
            return Err(Error::corrupt(
                &path.path,
                "segment symlink escapes dataset",
            ));
        }
        let segment = SegmentReader::open_with_mode(path, mode)?;
        if segment.schema() != self.schema()
            || segment.meta().header.record_count != entry.record_count
            || segment.block_count() != entry.block_count as usize
        {
            return Err(Error::corrupt(
                segment.path(),
                "segment does not match manifest",
            ));
        }
        Ok(segment)
    }

    /// Visit one decoded columnar block at a time in source order. Memory is
    /// bounded by a segment and a block; CRC and schema checks remain enabled.
    pub fn scan_columns(
        &self,
        mut visit: impl FnMut(&strata_format::BlockMeta, &[Vec<strata_core::FieldValue>]) -> Result<()>,
    ) -> Result<u64> {
        let mut rows = 0;
        for i in 0..self.segment_count() {
            let seg = self.open_segment(i)?;
            for b in 0..seg.block_count() {
                let (meta, columns) = seg.read_block_columns(b)?;
                visit(&meta, &columns)?;
                rows += u64::from(meta.row_count);
            }
        }
        Ok(rows)
    }

    /// Decode blocks with bounded parallel workers and visit them in source order.
    /// Each worker holds at most one queued block and one decoding block. Errors
    /// cancel delivery and close all channels before scoped threads are joined.
    pub fn scan_columns_parallel(
        &self,
        workers: usize,
        mut visit: impl FnMut(&strata_format::BlockMeta, &[Vec<strata_core::FieldValue>]) -> Result<()>,
    ) -> Result<u64> {
        if !(1..=32).contains(&workers) {
            return Err(Error::InvalidState {
                message: "decode workers must be 1..32".into(),
            });
        }
        if workers == 1 {
            return self.scan_columns(visit);
        }
        let mut rows = 0u64;
        for i in 0..self.segment_count() {
            let seg = self.open_segment(i)?;
            let count = workers.min(seg.block_count()).max(1);
            std::thread::scope(|scope| -> Result<()> {
                let mut receivers = Vec::with_capacity(count);
                for worker in 0..count {
                    let (tx, rx) = crossbeam_channel::bounded(1);
                    receivers.push(rx);
                    let seg = &seg;
                    scope.spawn(move || {
                        for b in (worker..seg.block_count()).step_by(count) {
                            if tx.send(seg.read_block_columns(b)).is_err() {
                                break;
                            }
                        }
                    });
                }
                let result = (|| {
                    for b in 0..seg.block_count() {
                        let (meta, columns) =
                            receivers[b % count]
                                .recv()
                                .map_err(|_| Error::InvalidState {
                                    message: "decode worker disconnected".into(),
                                })??;
                        visit(&meta, &columns)?;
                        rows += u64::from(meta.row_count);
                    }
                    Ok(())
                })();
                drop(receivers);
                result
            })?;
        }
        Ok(rows)
    }

    /// Reads all batches from all segments in order.
    pub fn read_all_batches(&self) -> Result<Vec<RecordBatch>> {
        let mut out = Vec::new();
        for i in 0..self.segment_count() {
            let seg = self.open_segment(i)?;
            out.extend(seg.read_all_batches()?);
        }
        Ok(out)
    }

    /// Reads all records from all segments in order.
    pub fn read_all_records(&self) -> Result<Vec<Record>> {
        let mut records = Vec::new();
        for i in 0..self.segment_count() {
            let seg = self.open_segment(i)?;
            records.extend(seg.read_all_records()?);
        }
        Ok(records)
    }

    /// Key fields from the manifest (empty = append-only).
    pub fn key_fields(&self) -> &[String] {
        &self.manifest.key_fields
    }

    /// True when updates/tombstones are supported.
    pub fn is_keyed(&self) -> bool {
        self.manifest.is_keyed()
    }

    /// Resolves the logical latest state (keyed datasets).
    ///
    /// Append-only datasets return all physical records.
    pub fn read_latest_records(&self) -> Result<Vec<Record>> {
        let physical = self.read_all_records()?;
        crate::versioning::resolve_latest(&self.manifest.key_fields, &physical)
    }
}
