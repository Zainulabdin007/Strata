//! Authoritative size accounting for persisted storage observations.
use crate::{Bytes, CompressionRatio};
use serde::{Deserialize, Serialize};
/// Sizes supplied by storage metadata and filesystem observations, never estimates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageMetrics {
    /// Physical record count.
    pub records: u64,
    /// Decoded payload bytes.
    pub logical_bytes: u64,
    /// Actual segment + manifest + index file lengths.
    pub physical_bytes: u64,
    /// Logical divided by physical, absent for empty storage.
    pub compression_ratio: Option<f64>,
    /// Signed savings (negative when framing exceeds logical size).
    pub storage_saved_bytes: i128,
    /// Number of sealed segments.
    pub segments: u64,
    /// Number of stored blocks.
    pub blocks: u64,
}
impl StorageMetrics {
    /// Construct from observations made by the storage reader.
    pub fn observed(
        records: u64,
        logical_bytes: u64,
        physical_bytes: u64,
        segments: u64,
        blocks: u64,
    ) -> Self {
        Self {
            records,
            logical_bytes,
            physical_bytes,
            segments,
            blocks,
            compression_ratio: (physical_bytes > 0).then(|| {
                CompressionRatio::from_sizes(Bytes(logical_bytes), Bytes(physical_bytes)).get()
            }),
            storage_saved_bytes: i128::from(logical_bytes) - i128::from(physical_bytes),
        }
    }
}
