//! Segment-level metadata.

use serde::{Deserialize, Serialize};
use strata_core::SchemaId;

use crate::block::BlockMeta;
use crate::version::{FormatVersion, FORMAT_VERSION};

/// Opaque segment identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SegmentId(pub String);

impl SegmentId {
    /// Creates a new segment id.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the underlying string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Fixed-size-ish header conceptually placed at the start of a segment.
///
/// Binary layout is specified in `FORMAT.md`; this struct is the logical form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentHeader {
    /// Format version of this segment.
    pub version: FormatVersion,
    /// Schema id referenced by this segment.
    pub schema_id: SchemaId,
    /// Segment identity.
    pub segment_id: SegmentId,
    /// Total number of records stored in the segment.
    pub record_count: u64,
    /// Number of columnar blocks in the segment.
    pub block_count: u32,
}

impl SegmentHeader {
    /// Creates a header for a new empty segment.
    pub fn new(schema_id: SchemaId, segment_id: SegmentId) -> Self {
        Self {
            version: FORMAT_VERSION,
            schema_id,
            segment_id,
            record_count: 0,
            block_count: 0,
        }
    }
}

/// Trailer metadata summarizing the segment after all blocks are written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentFooter {
    /// Byte offset from the start of the file to the block directory.
    pub block_directory_offset: u64,
    /// Uncompressed logical payload size in bytes.
    pub logical_bytes: u64,
    /// On-disk byte size of encoded block payloads.
    pub encoded_bytes: u64,
    /// CRC32 of all block bytes (concatenated in file order).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checksum: Option<u64>,
}

/// Full segment metadata: header + block list + footer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentMeta {
    /// Segment header.
    pub header: SegmentHeader,
    /// Ordered block metadata.
    pub blocks: Vec<BlockMeta>,
    /// Segment footer.
    pub footer: SegmentFooter,
}

impl SegmentMeta {
    /// Creates metadata for a segment with no blocks yet.
    pub fn empty(schema_id: SchemaId, segment_id: SegmentId) -> Self {
        Self {
            header: SegmentHeader::new(schema_id, segment_id),
            blocks: Vec::new(),
            footer: SegmentFooter {
                block_directory_offset: 0,
                logical_bytes: 0,
                encoded_bytes: 0,
                checksum: None,
            },
        }
    }

    /// Appends block metadata and updates header counters.
    pub fn push_block(&mut self, block: BlockMeta) {
        self.header.record_count += u64::from(block.row_count);
        self.header.block_count = self.header.block_count.saturating_add(1);
        self.footer.logical_bytes += block.logical_bytes;
        // Track on-disk block contribution for segment sizing / physical stats.
        self.footer.encoded_bytes += block.block_length;
        self.blocks.push(block);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::{BlockEncoding, BlockId, BlockMeta, ColumnBlockMeta};

    #[test]
    fn push_block_updates_counts() {
        let mut meta = SegmentMeta::empty(SchemaId::new("s"), SegmentId::new("seg-0"));
        meta.push_block(BlockMeta {
            id: BlockId(0),
            file_offset: 100,
            block_length: 250,
            row_count: 100,
            logical_bytes: 800,
            encoded_bytes: 200,
            semantic_encoded_bytes: 210,
            compressed_bytes: 180,
            compression_codec: 0,
            columns: vec![ColumnBlockMeta {
                field_name: "id".into(),
                encoding: BlockEncoding::Plain,
                data_rel_offset: 20,
                data_len: 200,
                logical_bytes: 800,
                encoded_bytes: 200,
            }],
        });
        assert_eq!(meta.header.record_count, 100);
        assert_eq!(meta.header.block_count, 1);
        assert_eq!(meta.footer.encoded_bytes, 250);
    }
}
