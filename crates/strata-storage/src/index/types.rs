//! Index value types and on-disk structures.

use serde::{Deserialize, Serialize};

/// Stable reference to a columnar block inside a dataset.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockRef {
    /// Zero-based segment ordinal in the dataset manifest order.
    pub segment_ordinal: u32,
    /// Segment file name (e.g. `000018.strata`).
    pub segment_file: String,
    /// Block id within the segment (matches [`strata_format::BlockId`]).
    pub block_id: u32,
    /// Rows stored in this block.
    pub row_count: u32,
}

impl BlockRef {
    /// Human-readable location, e.g. `segment 18 / block 921`.
    pub fn display_location(&self) -> String {
        format!(
            "segment {} / block {} ({})",
            self.segment_ordinal, self.block_id, self.segment_file
        )
    }
}

/// Kind of index artifact produced by rebuild.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexKind {
    /// Monotonic / identity unsigned integer zone map.
    Sequence,
    /// Timestamp zone map.
    Time,
    /// Categorical / enum inverted block postings.
    Categorical,
    /// Per-block min/max (and null counts) for comparable fields.
    BlockMinMax,
    /// Latest live version location per key (keyed datasets only).
    LatestVersion,
}

impl IndexKind {
    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sequence => "sequence",
            Self::Time => "time",
            Self::Categorical => "categorical",
            Self::BlockMinMax => "block_min_max",
            Self::LatestVersion => "latest_version",
        }
    }
}

/// Comparable scalar used in min/max and sequence/time zone maps.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum IndexValue {
    /// Unsigned integer.
    U64(u64),
    /// Signed integer / timestamp / fixed-decimal scaled integer.
    I64(i64),
    /// Lexicographic string bound (categorical / enum).
    String(String),
}

impl IndexValue {
    /// Returns the unsigned payload when this is [`IndexValue::U64`].
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Self::U64(v) => Some(*v),
            _ => None,
        }
    }

    /// Returns the signed payload when this is [`IndexValue::I64`].
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::I64(v) => Some(*v),
            _ => None,
        }
    }

    /// Returns the string payload when this is [`IndexValue::String`].
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(v) => Some(v.as_str()),
            _ => None,
        }
    }
}

/// Per-block zone map entry for one field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldMinMax {
    /// Field name.
    pub field: String,
    /// Minimum non-null value in the block (absent if all null).
    pub min: Option<IndexValue>,
    /// Maximum non-null value in the block (absent if all null).
    pub max: Option<IndexValue>,
    /// Null row count in this block for the field.
    pub null_count: u32,
}

/// Statistics for a single block across indexed fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockStats {
    /// Block location.
    pub block: BlockRef,
    /// Per-field min/max / nulls.
    pub fields: Vec<FieldMinMax>,
}

/// Dataset-wide block min/max statistics index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockMinMaxIndex {
    /// Format version.
    pub version: u32,
    /// All blocks in segment/block order.
    pub blocks: Vec<BlockStats>,
}

/// Sequence (unsigned integer) zone-map index for one field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceIndex {
    /// Format version.
    pub version: u32,
    /// Indexed field name.
    pub field: String,
    /// Sorted by `min` ascending (non-overlapping preferred but not required).
    pub entries: Vec<SequenceEntry>,
}

/// One sequence zone covering a block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SequenceEntry {
    /// Minimum sequence value in the block.
    pub min: u64,
    /// Maximum sequence value in the block.
    pub max: u64,
    /// Target block.
    pub block: BlockRef,
}

/// Timestamp zone-map index for one field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeIndex {
    /// Format version.
    pub version: u32,
    /// Indexed field name.
    pub field: String,
    /// Timestamp unit label from the schema (informational).
    pub unit: String,
    /// Sorted by `min` ascending.
    pub entries: Vec<TimeEntry>,
}

/// One timestamp zone covering a block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeEntry {
    /// Minimum timestamp (field unit).
    pub min: i64,
    /// Maximum timestamp (field unit).
    pub max: i64,
    /// Target block.
    pub block: BlockRef,
}

/// Categorical inverted index: value → blocks that contain it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoricalIndex {
    /// Format version.
    pub version: u32,
    /// Indexed field name.
    pub field: String,
    /// Whether the field is an enum (vs categorical string).
    pub is_enum: bool,
    /// Posting lists sorted by value.
    pub postings: Vec<CategoricalPosting>,
}

/// Blocks containing a single categorical / enum value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoricalPosting {
    /// Dictionary / enum value.
    pub value: String,
    /// Blocks that contain at least one occurrence (sorted by segment, block).
    pub blocks: Vec<BlockRef>,
}

/// Optional latest-version index for keyed datasets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestVersionIndex {
    /// Format version.
    pub version: u32,
    /// Key field names.
    pub key_fields: Vec<String>,
    /// One entry per live key (tombstoned keys omitted).
    pub entries: Vec<LatestVersionEntry>,
}

/// Location of the latest live row for one key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatestVersionEntry {
    /// Display form of key parts (stable, human-readable).
    pub key: Vec<String>,
    /// Segment ordinal in the manifest.
    pub segment_ordinal: u32,
    /// Block id within the segment.
    pub block_id: u32,
    /// Row index within the block.
    pub row_in_block: u32,
}
