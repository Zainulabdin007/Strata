//! Top-level compression profile types.

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::schema::{Schema, SchemaId};

use super::column::ColumnConfig;
use super::durability::DurabilityConfig;
use super::encoding::ColumnEncodingSpec;

fn default_true() -> bool {
    true
}

fn default_block_rows() -> u32 {
    65_536
}

fn default_segment_target_bytes() -> u64 {
    256 * 1024 * 1024
}

/// Supported (and reserved) input formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputFormat {
    /// Databento DBN MBO, optionally wrapped in Zstandard.
    Dbn,
    /// Delimited CSV with a header row.
    Csv,
    /// Newline-delimited JSON objects.
    Jsonl,
    /// Apache Parquet (reserved; not implemented yet).
    Parquet,
    /// Opaque binary records (reserved; not implemented yet).
    Binary,
}

impl InputFormat {
    /// Stable name for diagnostics.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dbn => "dbn",
            Self::Csv => "csv",
            Self::Jsonl => "jsonl",
            Self::Parquet => "parquet",
            Self::Binary => "binary",
        }
    }

    /// Default file extensions associated with this format.
    pub fn default_extensions(self) -> &'static [&'static str] {
        match self {
            Self::Dbn => &["dbn", "dbn.zst"],
            Self::Csv => &["csv"],
            Self::Jsonl => &["jsonl", "ndjson"],
            Self::Parquet => &["parquet"],
            Self::Binary => &["bin", "strata-bin"],
        }
    }

    /// Returns true when a parser implementation exists today.
    pub const fn is_supported(self) -> bool {
        matches!(self, Self::Csv | Self::Jsonl | Self::Dbn)
    }
}

/// Hint for how a field should be encoded when an explicit pipeline is omitted.
///
/// Prefer `encoding = ["delta", "bitpack"]` pipelines when you need precise control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldEncodingHint {
    /// Let Strata choose a default plan from the logical type.
    #[default]
    Auto,
    /// Prefer dictionary / categorical encoding.
    Dictionary,
    /// Prefer run-length encoding.
    RunLength,
    /// Prefer delta encoding of integers/timestamps.
    Delta,
    /// Prefer bit-packing of small integer domains.
    BitPack,
    /// Store values largely as-is (still subject to block framing).
    Plain,
}

/// Directory / file discovery filters.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct InputFilterConfig {
    /// Recurse into subdirectories when the ingest path is a directory.
    #[serde(default = "default_true")]
    pub recursive: bool,
    /// Allowed extensions (without dots). Empty means use format defaults.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Optional include globs relative to the ingest root (`*` and `**` supported).
    /// Empty means include all extension-matching files.
    #[serde(default)]
    pub include_globs: Vec<String>,
    /// Optional exclude globs relative to the ingest root (applied after includes).
    #[serde(default)]
    pub exclude_globs: Vec<String>,
}

impl Default for InputFilterConfig {
    fn default() -> Self {
        Self {
            recursive: true,
            extensions: Vec::new(),
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
        }
    }
}

impl InputFilterConfig {
    /// Effective extension list for discovery.
    pub fn effective_extensions(&self, format: InputFormat) -> Vec<String> {
        if self.extensions.is_empty() {
            format
                .default_extensions()
                .iter()
                .map(|s| (*s).to_string())
                .collect()
        } else {
            self.extensions
                .iter()
                .map(|e| e.trim_start_matches('.').to_ascii_lowercase())
                .collect()
        }
    }
}

/// Columnar block sizing.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct BlockConfig {
    /// Target number of records per columnar block.
    #[serde(default = "default_block_rows")]
    pub row_count: u32,
}

impl Default for BlockConfig {
    fn default() -> Self {
        Self {
            row_count: default_block_rows(),
        }
    }
}

/// Segment sizing targets.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SegmentConfig {
    /// Soft target for encoded segment size in bytes.
    #[serde(default = "default_segment_target_bytes")]
    pub target_size_bytes: u64,
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self {
            target_size_bytes: default_segment_target_bytes(),
        }
    }
}

/// General compression configuration applied after semantic encoding.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CompressionConfig {
    /// Block-level general compressor: `none`, `lz4`, or `zstd`.
    #[serde(default)]
    pub codec: CompressionCodecName,
    /// Optional compressor level (codec-specific; defaults apply when omitted).
    #[serde(default)]
    pub level: Option<i32>,
    /// Legacy flag; prefer `codec` instead.
    ///
    /// When `codec` is `none` and this is `true`, `general_compressor` is used.
    #[serde(default)]
    pub enable_general: bool,
    /// Legacy compressor name; prefer `codec` instead.
    #[serde(default)]
    pub general_compressor: Option<String>,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            codec: CompressionCodecName::None,
            level: None,
            enable_general: false,
            general_compressor: None,
        }
    }
}

impl CompressionConfig {
    /// Resolves the effective codec, honoring legacy `enable_general` fields.
    pub fn effective_codec(&self) -> std::result::Result<CompressionCodecName, String> {
        if self.codec != CompressionCodecName::None {
            return Ok(self.codec);
        }
        if !self.enable_general {
            return Ok(CompressionCodecName::None);
        }
        match self.general_compressor.as_deref() {
            Some(name) => CompressionCodecName::parse_name(name).ok_or_else(|| {
                format!("unknown compression.general_compressor `{name}`; expected none|lz4|zstd")
            }),
            None => Err(
                "compression.enable_general=true requires compression.general_compressor or codec"
                    .into(),
            ),
        }
    }

    /// Effective compression level for the resolved codec.
    pub fn effective_level(&self) -> i32 {
        self.level
            .unwrap_or_else(|| self.effective_codec().unwrap_or_default().default_level())
    }
}

/// Named general compression codec in a compression profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompressionCodecName {
    /// No general compression.
    #[default]
    None,
    /// LZ4.
    Lz4,
    /// Zstandard.
    Zstd,
}

impl CompressionCodecName {
    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Lz4 => "lz4",
            Self::Zstd => "zstd",
        }
    }

    /// Parse a config name.
    pub fn parse_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "none" | "identity" | "off" => Some(Self::None),
            "lz4" => Some(Self::Lz4),
            "zstd" | "zstandard" => Some(Self::Zstd),
            _ => None,
        }
    }

    /// Default level for this codec.
    pub const fn default_level(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Lz4 => 4,
            Self::Zstd => 3,
        }
    }
}

/// Schema identity section inside a profile.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SchemaSection {
    /// Schema id string.
    pub id: String,
    /// Optional description.
    #[serde(default)]
    pub description: Option<String>,
    /// Primary key column names for keyed datasets (updates / tombstones).
    ///
    /// Empty (default) means append-only — compaction is not required.
    #[serde(default)]
    pub key: Vec<String>,
}

/// Canonical Strata compression / ingest profile (TOML document root).
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CompressionProfile {
    /// Profile name for CLI / dashboard display.
    pub name: String,
    /// Declared input format.
    pub input_format: InputFormat,
    /// File discovery filters.
    #[serde(default)]
    pub input: InputFilterConfig,
    /// Schema identity section.
    pub schema: SchemaSection,
    /// Ordered column definitions (names + logical types + hints).
    pub columns: Vec<ColumnConfig>,
    /// Block sizing.
    #[serde(default)]
    pub block: BlockConfig,
    /// Segment sizing.
    #[serde(default)]
    pub segment: SegmentConfig,
    /// Compression configuration (declarative).
    #[serde(default)]
    pub compression: CompressionConfig,
    /// Durability / fsync policy.
    #[serde(default)]
    pub durability: DurabilityConfig,
}

impl CompressionProfile {
    /// Target block row count (alias used by older call sites).
    pub fn target_block_rows(&self) -> u32 {
        self.block.row_count
    }

    /// Builds a validated [`Schema`] from `columns`.
    pub fn schema(&self) -> Result<Schema> {
        if self.columns.is_empty() {
            return Err(Error::ProfileInvalid {
                path: std::path::PathBuf::from("<profile>"),
                message: "profile must define at least one column under `[[columns]]`".into(),
            });
        }

        let mut fields = Vec::with_capacity(self.columns.len());
        for column in &self.columns {
            fields.push(column.to_field()?);
        }

        let mut schema = Schema::new(SchemaId::new(self.schema.id.clone()), fields)?;
        if let Some(description) = &self.schema.description {
            schema = schema.with_description(description.clone());
        }
        Ok(schema)
    }

    /// Declared primary key fields (empty = append-only).
    pub fn key_fields(&self) -> &[String] {
        &self.schema.key
    }

    /// Validates profile semantics beyond TOML parsing.
    pub fn validate(&self) -> Result<()> {
        if self.name.trim().is_empty() {
            return Err(Error::ProfileInvalid {
                path: std::path::PathBuf::from("<profile>"),
                message: "profile `name` must not be empty".into(),
            });
        }
        if self.schema.id.trim().is_empty() {
            return Err(Error::ProfileInvalid {
                path: std::path::PathBuf::from("<profile>"),
                message: "schema.id must not be empty".into(),
            });
        }
        if self.block.row_count == 0 || self.block.row_count > 1_048_576 {
            return Err(Error::ProfileInvalid {
                path: std::path::PathBuf::from("<profile>"),
                message: "block.row_count must be 1..1048576".into(),
            });
        }
        if self.segment.target_size_bytes == 0 {
            return Err(Error::ProfileInvalid {
                path: std::path::PathBuf::from("<profile>"),
                message: "segment.target_size_bytes must be > 0".into(),
            });
        }
        if let Err(message) = self.compression.effective_codec() {
            return Err(Error::ProfileInvalid {
                path: std::path::PathBuf::from("<profile>"),
                message,
            });
        }
        if let Some(level) = self.compression.level {
            if !(0..=22).contains(&level) {
                return Err(Error::ProfileInvalid {
                    path: std::path::PathBuf::from("<profile>"),
                    message: format!("compression.level={level} is out of range (expected 0..=22)"),
                });
            }
        }

        // Ensure columns / types are valid.
        let schema = self.schema()?;
        for key in &self.schema.key {
            if schema.field(key).is_none() {
                return Err(Error::ProfileInvalid {
                    path: std::path::PathBuf::from("<profile>"),
                    message: format!("schema.key references unknown column `{key}`"),
                });
            }
        }

        for column in &self.columns {
            if let ColumnEncodingSpec::Pipeline(steps) = &column.encoding {
                if steps.is_empty() {
                    return Err(Error::ProfileInvalid {
                        path: std::path::PathBuf::from("<profile>"),
                        message: format!(
                            "column `{}`: encoding pipeline must not be empty",
                            column.name
                        ),
                    });
                }
            }
            if let (Some(min), Some(max)) = (column.expected_min, column.expected_max) {
                if min.as_f64() > max.as_f64() {
                    return Err(Error::ProfileInvalid {
                        path: std::path::PathBuf::from("<profile>"),
                        message: format!(
                            "column `{}`: expected_min ({}) is greater than expected_max ({})",
                            column.name,
                            min.as_f64(),
                            max.as_f64()
                        ),
                    });
                }
            }
        }

        Ok(())
    }
}
