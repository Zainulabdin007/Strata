//! SegmentReader: open and decode sealed `.strata` segment files.

use std::path::{Path, PathBuf};

use strata_core::{RecordBatch, Schema};
use strata_format::{
    BlockEncoding, BlockId, BlockMeta, ColumnBlockMeta, FormatVersion, SegmentFooter,
    SegmentHeader, SegmentId, SegmentMeta, FOOTER_MAGIC, FORMAT_VERSION, SEGMENT_MAGIC,
};

use crate::block::BlockReader;
use crate::error::{Error, Result};
use crate::io::BufCursor;
use crate::mmap::{SealedSegmentBytes, SegmentBytesBackend, SegmentReadMode};
use crate::path::SegmentPath;

/// Reads a sealed Strata segment file (valid `SFTR` footer).
///
/// Bytes may be buffered or memory-mapped. Block accessors return borrowed
/// views into that storage without copying the frame.
#[derive(Debug)]
pub struct SegmentReader {
    path: PathBuf,
    bytes: SealedSegmentBytes,
    schema: Schema,
    meta: SegmentMeta,
}

impl SegmentReader {
    /// Opens a sealed segment, preferring mmap with buffered fallback ([`SegmentReadMode::Auto`]).
    pub fn open(path: SegmentPath) -> Result<Self> {
        Self::open_with_mode(path, SegmentReadMode::Auto)
    }

    /// Opens a sealed segment using an explicit read mode.
    pub fn open_with_mode(path: SegmentPath, mode: SegmentReadMode) -> Result<Self> {
        let path = path.path;
        let bytes = SealedSegmentBytes::open(&path, mode)?;
        let (schema, meta) = parse_segment(bytes.as_slice(), &path)?;
        Ok(Self {
            path,
            bytes,
            schema,
            meta,
        })
    }

    /// Opens with a full-file buffered read (never mmap).
    pub fn open_buffered(path: SegmentPath) -> Result<Self> {
        Self::open_with_mode(path, SegmentReadMode::Buffered)
    }

    /// Opens with a mandatory memory map (Unix sealed segments).
    pub fn open_mmap(path: SegmentPath) -> Result<Self> {
        Self::open_with_mode(path, SegmentReadMode::Mmap)
    }

    /// Segment filesystem path.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Schema embedded in the segment header.
    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Parsed segment metadata.
    pub fn meta(&self) -> &SegmentMeta {
        &self.meta
    }

    /// Whether this reader is backed by mmap or a heap buffer.
    pub fn bytes_backend(&self) -> SegmentBytesBackend {
        self.bytes.backend()
    }

    /// Full sealed segment bytes (zero-copy when mmap-backed).
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_slice()
    }

    /// Number of blocks.
    pub fn block_count(&self) -> usize {
        self.meta.blocks.len()
    }

    /// Reads one block as a [`RecordBatch`].
    pub fn read_block(&self, index: usize) -> Result<RecordBatch> {
        let bytes = self.block_bytes(index)?;
        BlockReader::new().read_batch(&self.schema, bytes)
    }

    /// Decode column arrays directly, without rebuilding row records.
    pub fn read_block_columns(
        &self,
        index: usize,
    ) -> Result<(BlockMeta, Vec<Vec<strata_core::FieldValue>>)> {
        BlockReader::new().read_columns(&self.schema, self.block_bytes(index)?, &self.path)
    }

    /// Decode selected column arrays in projection order.
    pub fn read_block_columns_projected(
        &self,
        index: usize,
        project: &[String],
    ) -> Result<(BlockMeta, Vec<Vec<strata_core::FieldValue>>)> {
        BlockReader::new().read_columns_projected(
            &self.schema,
            self.block_bytes(index)?,
            &self.path,
            project,
        )
    }

    /// Reads one block decoding only `project` columns.
    ///
    /// The returned batch schema contains only the projected fields.
    pub fn read_block_projected(&self, index: usize, project: &[String]) -> Result<RecordBatch> {
        let bytes = self.block_bytes(index)?;
        BlockReader::new().read_batch_projected(&self.schema, bytes, project)
    }

    /// Resolves a block by its on-disk block id.
    pub fn block_index_for_id(&self, block_id: u32) -> Option<usize> {
        self.meta.blocks.iter().position(|b| b.id.0 == block_id)
    }

    /// Borrowed on-disk bytes for a block by directory index (no frame copy).
    pub fn block_bytes(&self, index: usize) -> Result<&[u8]> {
        let block = self.meta.blocks.get(index).ok_or_else(|| {
            Error::corrupt(
                &self.path,
                format!(
                    "block index {index} out of range (block_count={})",
                    self.meta.blocks.len()
                ),
            )
        })?;
        let start = block.file_offset as usize;
        let end = start
            .checked_add(block.block_length as usize)
            .ok_or_else(|| Error::corrupt(&self.path, "block length overflow"))?;
        let slice = self.bytes.as_slice();
        if end > slice.len() {
            return Err(Error::corrupt(
                &self.path,
                format!("block {index} extends past end of file"),
            ));
        }
        Ok(&slice[start..end])
    }

    /// Reads all blocks in order.
    pub fn read_all_batches(&self) -> Result<Vec<RecordBatch>> {
        let mut out = Vec::with_capacity(self.block_count());
        for i in 0..self.block_count() {
            out.push(self.read_block(i)?);
        }
        Ok(out)
    }

    /// Reads all records across all blocks.
    pub fn read_all_records(&self) -> Result<Vec<strata_core::Record>> {
        let mut records = Vec::new();
        for i in 0..self.block_count() {
            records.extend(self.read_block(i)?.records);
        }
        Ok(records)
    }
}

fn parse_segment(bytes: &[u8], path: &Path) -> Result<(Schema, SegmentMeta)> {
    if bytes.len() < SEGMENT_MAGIC.len() + FOOTER_MAGIC.len() + 32 {
        return Err(Error::corrupt(path, "segment file too short"));
    }

    // Footer is fixed size at end: dir_off(8)+logical(8)+encoded(8)+crc(4)+magic(4) = 32
    let footer_len = 32;
    let footer_start = bytes.len() - footer_len;
    let footer_bytes = &bytes[footer_start..];
    if &footer_bytes[28..32] != FOOTER_MAGIC {
        return Err(Error::corrupt(path, "missing or invalid footer magic"));
    }

    let mut fcur = BufCursor::new(footer_bytes, path);
    let directory_offset = fcur.read_u64()?;
    let logical_bytes = fcur.read_u64()?;
    let encoded_bytes = fcur.read_u64()?;
    let blocks_crc = fcur.read_u32()?;
    let _ = fcur.read_exact(4)?;

    let mut cur = BufCursor::new(bytes, path);
    let magic = cur.read_exact(8)?;
    if magic != SEGMENT_MAGIC {
        return Err(Error::corrupt(path, "invalid segment magic"));
    }
    let major = cur.read_u16()?;
    let minor = cur.read_u16()?;
    let version = FormatVersion { major, minor };
    if !FORMAT_VERSION.is_compatible_with(version) {
        // Reader supports same major and minor >= file minor
        // is_compatible_with(self, other): self can read other if same major && self.minor >= other.minor
        // So FORMAT_VERSION.is_compatible_with(version) means we can read `version`.
        return Err(Error::UnsupportedVersion {
            found_major: major,
            found_minor: minor,
            supported_major: FORMAT_VERSION.major,
            supported_minor: FORMAT_VERSION.minor,
        });
    }
    let _flags = cur.read_u32()?;
    let schema_len = cur.read_u32()? as usize;
    let schema_bytes = cur.read_exact(schema_len)?;
    let schema: Schema = serde_json::from_slice(schema_bytes)
        .map_err(|e| Error::corrupt(path, format!("invalid schema JSON in segment header: {e}")))?;
    schema.validate()?;
    let segment_id = SegmentId::new(cur.read_string()?);
    let record_count = cur.read_u64()?;
    let block_count = cur.read_u32()?;
    let _reserved = cur.read_u32()?;

    let header = SegmentHeader {
        version,
        schema_id: schema.id.clone(),
        segment_id,
        record_count,
        block_count,
    };

    if directory_offset > footer_start as u64 {
        return Err(Error::corrupt(
            path,
            "block directory offset points past footer",
        ));
    }

    // Verify blocks CRC over [header_end .. directory_offset)
    let header_end = cur.position();
    if directory_offset < header_end as u64 {
        return Err(Error::corrupt(path, "directory overlaps header"));
    }
    let blocks_region = &bytes[header_end..directory_offset as usize];
    let actual_crc = strata_format::crc32(if minor >= 2 {
        &bytes[..bytes.len() - 8]
    } else {
        blocks_region
    });
    if actual_crc != blocks_crc {
        return Err(Error::corrupt(
            path,
            format!("segment blocks CRC32 mismatch: expected {blocks_crc:#x}, got {actual_crc:#x}"),
        ));
    }

    let mut dcur = BufCursor::new(&bytes[directory_offset as usize..footer_start], path);
    if block_count as usize > dcur.remaining() / 28 {
        return Err(Error::corrupt(path, "block count exceeds directory"));
    }
    let mut blocks = Vec::with_capacity(block_count as usize);
    let mut expected_offset = header_end as u64;
    let mut total_rows = 0u64;
    for _ in 0..block_count {
        let id = BlockId(dcur.read_u32()?);
        if id.0 as usize != blocks.len() {
            return Err(Error::corrupt(path, "duplicate or nonsequential block id"));
        }
        let file_offset = dcur.read_u64()?;
        let block_length = dcur.read_u64()?;
        let row_count = dcur.read_u32()?;
        let column_count = dcur.read_u32()? as usize;
        if column_count != schema.len() || column_count > dcur.remaining() / 21 {
            return Err(Error::corrupt(path, "invalid column count"));
        }
        if file_offset != expected_offset
            || block_length < 14
            || file_offset
                .checked_add(block_length)
                .filter(|e| *e <= directory_offset)
                .is_none()
        {
            return Err(Error::corrupt(path, "invalid block range"));
        }
        expected_offset = file_offset + block_length;
        total_rows += u64::from(row_count);
        let mut columns = Vec::with_capacity(column_count);
        let mut logical = 0u64;
        let mut encoded = 0u64;
        for col_index in 0..column_count {
            let field_name = dcur.read_string()?;
            if field_name != schema.fields[col_index].name {
                return Err(Error::corrupt(path, "directory column name mismatch"));
            }
            let encoding_tag = dcur.read_u8()?;
            let encoding = BlockEncoding::from_u8(encoding_tag).ok_or_else(|| {
                Error::corrupt(path, format!("unknown encoding tag {encoding_tag}"))
            })?;
            let data_rel_offset = dcur.read_u32()?;
            let data_len = dcur.read_u32()?;
            let logical_bytes = dcur.read_u64()?;
            logical = logical
                .checked_add(logical_bytes)
                .ok_or_else(|| Error::corrupt(path, "logical byte overflow"))?;
            encoded += u64::from(data_len);
            columns.push(ColumnBlockMeta {
                field_name,
                encoding,
                data_rel_offset,
                data_len,
                logical_bytes,
                encoded_bytes: u64::from(data_len),
            });
        }
        blocks.push(BlockMeta {
            id,
            file_offset,
            block_length,
            row_count,
            logical_bytes: logical,
            encoded_bytes: encoded,
            semantic_encoded_bytes: u64::from(u32::from_le_bytes(
                bytes[file_offset as usize + 2..file_offset as usize + 6]
                    .try_into()
                    .unwrap(),
            )),
            compressed_bytes: u64::from(u32::from_le_bytes(
                bytes[file_offset as usize + 6..file_offset as usize + 10]
                    .try_into()
                    .unwrap(),
            )),
            compression_codec: bytes[file_offset as usize + 1],
            columns,
        });
    }

    if dcur.remaining() != 0 || expected_offset != directory_offset || total_rows != record_count {
        return Err(Error::corrupt(
            path,
            "inconsistent segment directory totals",
        ));
    }
    if blocks.len() as u32 != block_count {
        return Err(Error::corrupt(
            path,
            format!(
                "directory block entries {} != header block_count {block_count}",
                blocks.len()
            ),
        ));
    }

    let footer = SegmentFooter {
        block_directory_offset: directory_offset,
        logical_bytes,
        encoded_bytes,
        checksum: Some(u64::from(blocks_crc)),
    };

    Ok((
        schema,
        SegmentMeta {
            header,
            blocks,
            footer,
        },
    ))
}
