//! Index catalog and on-disk paths.

use serde::{Deserialize, Serialize};

use super::types::IndexKind;

/// On-disk index format version written by rebuild.
pub const INDEX_FORMAT_VERSION: u32 = 1;

/// Catalog describing which index artifacts exist for a dataset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexCatalog {
    /// Index format version.
    pub version: u32,
    /// CRC of the exact dataset manifest from which indexes were built.
    #[serde(default)]
    pub dataset_crc: Option<u32>,
    /// Artifact checksums; missing checksums require a rebuild before pruning.
    #[serde(default)]
    pub checksums: std::collections::BTreeMap<String, u32>,
    /// Schema id copied from the dataset manifest at rebuild time.
    pub schema_id: String,
    /// Total records observed while building.
    pub record_count: u64,
    /// Total blocks indexed.
    pub block_count: u64,
    /// Artifacts present under `indexes/`.
    pub artifacts: Vec<IndexArtifact>,
}

/// One rebuilt index file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexArtifact {
    /// Index kind.
    pub kind: IndexKind,
    /// Field name (empty for dataset-wide block min/max).
    pub field: String,
    /// Relative path from the dataset `indexes/` directory.
    pub path: String,
}

impl IndexCatalog {
    /// Relative catalog file name.
    pub const FILE_NAME: &'static str = "manifest.json";
}
