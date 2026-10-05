//! SegmentWriter: append columnar blocks into a `.strata` file.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use strata_codec::Compressor;
use strata_codec::{EncodingPlan, IdentityCompressor};
use strata_core::{DurabilityMode, Record, RecordBatch, Schema};
use strata_format::{BlockId, SegmentId, SegmentMeta, FOOTER_MAGIC, FORMAT_VERSION, SEGMENT_MAGIC};
use strata_metrics::MetricsHandle;

use crate::block::BlockWriter;
use crate::durability::{sync_after_block, sync_commit};
use crate::error::{Error, Result};
use crate::io::{ensure_parent, write_bytes, write_string, write_u16, write_u32, write_u64};
use crate::path::SegmentPath;

/// Writes blocks into a single Strata segment file.
pub struct SegmentWriter {
    path: PathBuf,
    file: File,
    schema: Schema,
    meta: SegmentMeta,
    /// File offset where `record_count` / `block_count` live for patching.
    counts_offset: u64,
    /// Incremental CRC32 over concatenated block bytes.
    blocks_hasher: crc32fast::Hasher,
    max_block_rows: u32,
    metrics: Option<MetricsHandle>,
    encoding_plans: HashMap<String, EncodingPlan>,
    compressor: Arc<dyn Compressor>,
    durability: DurabilityMode,
    sync_every_blocks: u32,
    blocks_since_sync: u32,
    /// Records durable through the last successful sync/ack point.
    acknowledged_records: u64,
    finalized: bool,
}

impl SegmentWriter {
    /// Creates a new segment file and writes the header.
    pub fn create(
        path: SegmentPath,
        schema: Schema,
        segment_id: SegmentId,
        max_block_rows: u32,
    ) -> Result<Self> {
        Self::create_with_metrics(
            path,
            schema,
            segment_id,
            max_block_rows,
            None,
            HashMap::new(),
            Arc::new(IdentityCompressor),
            DurabilityMode::Fast,
            16,
        )
    }

    /// Creates a segment writer that emits encode/write observations to `metrics`.
    #[allow(clippy::too_many_arguments)]
    pub fn create_with_metrics(
        path: SegmentPath,
        schema: Schema,
        segment_id: SegmentId,
        max_block_rows: u32,
        metrics: Option<MetricsHandle>,
        encoding_plans: HashMap<String, EncodingPlan>,
        compressor: Arc<dyn Compressor>,
        durability: DurabilityMode,
        sync_every_blocks: u32,
    ) -> Result<Self> {
        if max_block_rows == 0 {
            return Err(Error::InvalidState {
                message: "max_block_rows must be > 0".into(),
            });
        }
        ensure_parent(&path.path)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path.path)
            .map_err(|source| Error::Io {
                path: path.path.clone(),
                source,
            })?;

        let schema_id = schema.id.clone();
        let t0 = Instant::now();
        let counts_offset = write_header(&mut file, &path.path, &schema, &segment_id)?;
        let header_end = file.stream_position().map_err(|source| Error::Io {
            path: path.path.clone(),
            source,
        })?;
        if let Some(m) = &metrics {
            m.record_write(header_end, t0.elapsed());
        }

        let meta = SegmentMeta::empty(schema_id, segment_id);

        Ok(Self {
            path: path.path,
            file,
            schema,
            meta,
            counts_offset,
            blocks_hasher: crc32fast::Hasher::new(),
            max_block_rows,
            metrics,
            encoding_plans,
            compressor,
            durability,
            sync_every_blocks: sync_every_blocks.max(1),
            blocks_since_sync: 0,
            acknowledged_records: 0,
            finalized: false,
        })
    }

    /// Returns the segment path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns in-progress metadata.
    pub fn meta(&self) -> &SegmentMeta {
        &self.meta
    }

    /// Approximate encoded block payload bytes written so far.
    pub fn encoded_bytes_so_far(&self) -> u64 {
        self.meta.footer.encoded_bytes
    }

    /// Records that have been acknowledged under the active durability policy.
    pub fn acknowledged_records(&self) -> u64 {
        self.acknowledged_records
    }

    /// Durability mode for this segment.
    pub fn durability(&self) -> DurabilityMode {
        self.durability
    }

    /// Writes a pre-encoded block.
    pub fn write_encoded_block(&mut self, mut encoded: crate::block::EncodedBlock) -> Result<()> {
        self.ensure_open()?;
        let offset = self.file.stream_position().map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })?;
        encoded.meta.file_offset = offset;
        encoded.meta.id = BlockId(self.meta.header.block_count);

        let nbytes = encoded.bytes.len() as u64;
        let t0 = Instant::now();
        write_bytes(&mut self.file, &self.path, &encoded.bytes)?;
        let write_dur = t0.elapsed();
        self.blocks_hasher.update(&encoded.bytes);
        self.meta.push_block(encoded.meta);

        let synced = sync_after_block(
            &mut self.file,
            &self.path,
            self.durability,
            &mut self.blocks_since_sync,
            self.sync_every_blocks,
        )?;
        // FAST acknowledges at write time (lossy by contract).
        // DURABLE/STRICT acknowledge only when a sync barrier succeeds.
        if self.durability == DurabilityMode::Fast || synced {
            self.acknowledged_records = self.meta.header.record_count;
        }

        if let Some(m) = &self.metrics {
            m.record_write(nbytes, write_dur);
            m.record_block_completed();
        }
        Ok(())
    }

    /// Encodes and writes records, splitting into blocks of `max_block_rows`.
    pub fn write_records(&mut self, records: &[Record]) -> Result<()> {
        self.ensure_open()?;
        if records.is_empty() {
            return Ok(());
        }
        for chunk in records.chunks(self.max_block_rows as usize) {
            let mut bw = BlockWriter::with_plans_and_compressor(
                self.schema.clone(),
                self.encoding_plans.clone(),
                Arc::clone(&self.compressor),
            );
            bw.extend(chunk.iter().cloned())?;
            let encoded = bw.finish(BlockId(self.meta.header.block_count))?;
            if let Some(m) = &self.metrics {
                m.record_block_sizes(
                    encoded.meta.logical_bytes,
                    encoded.meta.semantic_encoded_bytes,
                    encoded.meta.compressed_bytes,
                    encoded.semantic_encode_duration,
                    encoded.compress_duration,
                );
            }
            self.write_encoded_block(encoded)?;
        }
        Ok(())
    }

    /// Writes a record batch (must match this segment's schema).
    pub fn write_batch(&mut self, batch: &RecordBatch) -> Result<()> {
        if batch.schema != self.schema {
            return Err(Error::InvalidState {
                message: "record batch schema does not match segment schema".into(),
            });
        }
        self.write_records(&batch.records)
    }

    /// Finalizes the segment: directory, patched counts, then footer (commit).
    ///
    /// Safe commit order:
    /// 1. append block directory
    /// 2. patch header record/block counts
    /// 3. append footer ending in `SFTR` (commit marker)
    /// 4. flush / fsync per durability mode
    ///
    /// A crash before step 3 leaves an incomplete tail that `recover` can
    /// truncate without corrupting earlier valid blocks.
    pub fn finish(mut self) -> Result<SegmentMeta> {
        self.ensure_open()?;
        self.finalized = true;

        let before = self.file.stream_position().map_err(|source| Error::Io {
            path: self.path.clone(),
            source,
        })?;
        let t0 = Instant::now();

        let directory_offset = before;
        write_block_directory(&mut self.file, &self.path, &self.meta)?;

        let mut blocks_crc = self.blocks_hasher.clone().finalize();
        self.meta.footer.block_directory_offset = directory_offset;
        self.meta.footer.checksum = Some(u64::from(blocks_crc));

        // Patch counts BEFORE the footer so any file with SFTR has consistent counts.
        self.file
            .seek(SeekFrom::Start(self.counts_offset))
            .map_err(|source| Error::Io {
                path: self.path.clone(),
                source,
            })?;
        write_u64(&mut self.file, &self.path, self.meta.header.record_count)?;
        write_u32(&mut self.file, &self.path, self.meta.header.block_count)?;

        self.file
            .seek(SeekFrom::End(0))
            .map_err(|source| Error::Io {
                path: self.path.clone(),
                source,
            })?;

        write_u64(
            &mut self.file,
            &self.path,
            self.meta.footer.block_directory_offset,
        )?;
        write_u64(&mut self.file, &self.path, self.meta.footer.logical_bytes)?;
        write_u64(&mut self.file, &self.path, self.meta.footer.encoded_bytes)?;
        blocks_crc = crate::io::checksum_prefix(&mut self.file, &self.path)?;
        self.meta.footer.checksum = Some(u64::from(blocks_crc));
        write_u32(&mut self.file, &self.path, blocks_crc)?;
        write_bytes(&mut self.file, &self.path, FOOTER_MAGIC)?;

        sync_commit(&mut self.file, &self.path, self.durability)?;
        self.acknowledged_records = self.meta.header.record_count;

        let after_append = self
            .file
            .seek(SeekFrom::End(0))
            .map_err(|source| Error::Io {
                path: self.path.clone(),
                source,
            })?;
        let appended = after_append.saturating_sub(before);
        if let Some(m) = &self.metrics {
            m.record_write(appended, t0.elapsed());
            m.record_segment_completed();
        }

        Ok(self.meta)
    }

    fn ensure_open(&self) -> Result<()> {
        if self.finalized {
            Err(Error::InvalidState {
                message: "segment writer already finalized".into(),
            })
        } else {
            Ok(())
        }
    }
}

fn write_header(
    file: &mut File,
    path: &Path,
    schema: &Schema,
    segment_id: &SegmentId,
) -> Result<u64> {
    write_bytes(file, path, SEGMENT_MAGIC)?;
    write_u16(file, path, FORMAT_VERSION.major)?;
    write_u16(file, path, FORMAT_VERSION.minor)?;
    write_u32(file, path, 0)?; // flags

    let schema_json = serde_json::to_vec(schema).map_err(|e| Error::InvalidState {
        message: format!("failed to serialize schema: {e}"),
    })?;
    let schema_len = u32::try_from(schema_json.len()).map_err(|_| Error::InvalidState {
        message: "schema JSON too large".into(),
    })?;
    write_u32(file, path, schema_len)?;
    write_bytes(file, path, &schema_json)?;
    write_string(file, path, segment_id.as_str())?;

    let counts_offset = file.stream_position().map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    write_u64(file, path, 0)?; // record_count placeholder
    write_u32(file, path, 0)?; // block_count placeholder
    write_u32(file, path, 0)?; // reserved
    Ok(counts_offset)
}

fn write_block_directory(file: &mut File, path: &Path, meta: &SegmentMeta) -> Result<()> {
    for block in &meta.blocks {
        write_u32(file, path, block.id.0)?;
        write_u64(file, path, block.file_offset)?;
        write_u64(file, path, block.block_length)?;
        write_u32(file, path, block.row_count)?;
        write_u32(file, path, block.columns.len() as u32)?;
        for col in &block.columns {
            write_string(file, path, &col.field_name)?;
            file.write_all(&[col.encoding.as_u8()])
                .map_err(|source| Error::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
            write_u32(file, path, col.data_rel_offset)?;
            write_u32(file, path, col.data_len)?;
            write_u64(file, path, col.logical_bytes)?;
        }
    }
    Ok(())
}
