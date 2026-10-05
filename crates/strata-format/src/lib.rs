//! Versioned on-disk format definitions for Strata.
//!
//! This crate describes segment/block metadata and binary constants.
//! Readers and writers live in `strata-storage`.

#![deny(missing_docs)]

pub mod binary;
pub mod block;
pub mod segment;
pub mod version;

pub use binary::{
    crc32, BLOCK_COMPRESS_LZ4, BLOCK_COMPRESS_NONE, BLOCK_COMPRESS_ZSTD, BLOCK_FRAME_VERSION,
    ENCODING_PIPELINE, ENCODING_PLAIN, FOOTER_MAGIC, SEGMENT_MAGIC,
};
pub use block::{BlockEncoding, BlockId, BlockMeta, ColumnBlockMeta};
pub use segment::{SegmentFooter, SegmentHeader, SegmentId, SegmentMeta};
pub use version::{FormatVersion, FORMAT_VERSION};
