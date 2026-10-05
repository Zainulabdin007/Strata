//! Block-level metadata within a segment.

use serde::{Deserialize, Serialize};

/// Identifier of a block within a segment (0-based).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockId(pub u32);

/// Declared encoding applied to a column block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum BlockEncoding {
    /// Encoding not yet chosen / written as a placeholder.
    Unspecified = 0,
    /// Values stored without semantic encoding (legacy PLAIN column body).
    Plain = 1,
    /// Dictionary encoding (legacy tag; prefer [`Self::Pipeline`]).
    Dictionary = 2,
    /// Run-length encoding (legacy tag; prefer [`Self::Pipeline`]).
    RunLength = 3,
    /// Delta encoding (legacy tag; prefer [`Self::Pipeline`]).
    Delta = 4,
    /// Bit-packed integers (legacy tag; prefer [`Self::Pipeline`]).
    BitPack = 5,
    /// Self-describing composable semantic encoding pipeline.
    Pipeline = 6,
}

impl BlockEncoding {
    /// Converts a wire tag into an encoding.
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Unspecified),
            1 => Some(Self::Plain),
            2 => Some(Self::Dictionary),
            3 => Some(Self::RunLength),
            4 => Some(Self::Delta),
            5 => Some(Self::BitPack),
            6 => Some(Self::Pipeline),
            _ => None,
        }
    }

    /// Wire tag for this encoding.
    pub const fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Metadata for a single column inside a block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnBlockMeta {
    /// Schema field name.
    pub field_name: String,
    /// Encoding applied to this column in the block.
    pub encoding: BlockEncoding,
    /// Byte offset of this column's payload relative to the block start.
    pub data_rel_offset: u32,
    /// Encoded payload length in bytes.
    pub data_len: u32,
    /// Logical (decoded) size estimate in bytes.
    pub logical_bytes: u64,
    /// Encoded size on disk in bytes (same as `data_len` for PLAIN).
    pub encoded_bytes: u64,
}

/// Metadata for one columnar block (a row-group of N records).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockMeta {
    /// Block id within the parent segment.
    pub id: BlockId,
    /// Absolute file offset of the block start within the segment.
    pub file_offset: u64,
    /// Total on-disk length of the block (including frame header and checksum).
    pub block_length: u64,
    /// Number of rows in this block.
    pub row_count: u32,
    /// Sum of column logical sizes (raw / decoded estimate).
    pub logical_bytes: u64,
    /// Sum of column semantic-encoded sizes (pre-general-compression).
    pub encoded_bytes: u64,
    /// Size of the uncompressed semantic block body (directory + column payloads).
    pub semantic_encoded_bytes: u64,
    /// Size of the generally-compressed payload (equals semantic when codec is none).
    pub compressed_bytes: u64,
    /// General compression codec tag (`0=none`, `1=lz4`, `2=zstd`).
    pub compression_codec: u8,
    /// Per-column metadata, in schema field order.
    pub columns: Vec<ColumnBlockMeta>,
}

impl BlockMeta {
    /// Creates an empty block meta shell.
    pub fn new(id: BlockId, row_count: u32) -> Self {
        Self {
            id,
            file_offset: 0,
            block_length: 0,
            row_count,
            logical_bytes: 0,
            encoded_bytes: 0,
            semantic_encoded_bytes: 0,
            compressed_bytes: 0,
            compression_codec: 0,
            columns: Vec::new(),
        }
    }

    /// Adds column metadata and accumulates logical/semantic sizes.
    pub fn push_column(&mut self, column: ColumnBlockMeta) {
        self.logical_bytes += column.logical_bytes;
        self.encoded_bytes += column.encoded_bytes;
        self.columns.push(column);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulates_column_sizes() {
        let mut block = BlockMeta::new(BlockId(0), 10);
        block.push_column(ColumnBlockMeta {
            field_name: "a".into(),
            encoding: BlockEncoding::Plain,
            data_rel_offset: 0,
            data_len: 40,
            logical_bytes: 40,
            encoded_bytes: 40,
        });
        block.push_column(ColumnBlockMeta {
            field_name: "b".into(),
            encoding: BlockEncoding::Plain,
            data_rel_offset: 40,
            data_len: 8,
            logical_bytes: 20,
            encoded_bytes: 8,
        });
        assert_eq!(block.logical_bytes, 60);
        assert_eq!(block.encoded_bytes, 48);
    }
}
