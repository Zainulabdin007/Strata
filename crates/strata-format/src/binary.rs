//! Binary constants and helpers for the Strata on-disk format.

/// Magic bytes at the start of every segment file.
pub const SEGMENT_MAGIC: &[u8; 8] = b"STRATA01";

/// Magic bytes at the end of every segment footer.
pub const FOOTER_MAGIC: &[u8; 4] = b"SFTR";

/// Encoding tag written in block/column directories.
pub const ENCODING_PLAIN: u8 = 1;

/// Encoding tag for self-describing composable semantic pipelines.
pub const ENCODING_PIPELINE: u8 = 6;

/// Block frame version (semantic body + optional general compression).
pub const BLOCK_FRAME_VERSION: u8 = 1;

/// No general compression on the block body.
pub const BLOCK_COMPRESS_NONE: u8 = 0;
/// LZ4 general compression on the block body.
pub const BLOCK_COMPRESS_LZ4: u8 = 1;
/// Zstandard general compression on the block body.
pub const BLOCK_COMPRESS_ZSTD: u8 = 2;

/// CRC32 castagnoli/ieee via `crc32fast` (IEEE).
pub fn crc32(data: &[u8]) -> u32 {
    crc32fast::hash(data)
}
