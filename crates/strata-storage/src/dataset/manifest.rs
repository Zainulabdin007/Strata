//! Dataset manifest (JSON) stored at `<dataset>/manifest.json`.

use serde::{Deserialize, Serialize};
use strata_core::Schema;
use strata_format::FormatVersion;

/// One segment entry in the dataset manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestSegment {
    /// Relative file name under `segments/` (e.g. `000000.strata`).
    pub file: String,
    /// Segment id string.
    pub segment_id: String,
    /// Records in the segment.
    pub record_count: u64,
    /// Blocks in the segment.
    pub block_count: u32,
}

/// Dataset-level manifest written on finish.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatasetManifest {
    /// Manifest document version.
    pub manifest_version: u32,
    /// On-disk format version used by segments.
    pub format_version: FormatVersion,
    /// Dataset schema.
    pub schema: Schema,
    /// Block row target used when writing.
    pub block_row_count: u32,
    /// Soft segment size target in bytes.
    pub segment_target_size_bytes: u64,
    /// Ordered segment entries.
    pub segments: Vec<ManifestSegment>,
    /// Total physical records across all segments (includes obsolete versions).
    pub record_count: u64,
    /// Primary key field names (empty = append-only, no updates/tombstones).
    #[serde(default)]
    pub key_fields: Vec<String>,
    /// Number of successful compactions applied to this dataset.
    #[serde(default)]
    pub compaction_generation: u64,
}

impl DatasetManifest {
    /// Current manifest document version.
    pub const VERSION: u32 = 2;

    /// True when this dataset supports keyed updates / deletes.
    pub fn is_keyed(&self) -> bool {
        !self.key_fields.is_empty()
    }
}
