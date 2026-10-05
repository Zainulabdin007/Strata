//! Compression profiles and ingest configuration loaded from TOML.
//!
//! A profile describes how Strata should read input and encode columns.
//! Explicit `encoding = ["…"]` pipelines are resolved at ingest time.

mod column;
mod durability;
mod encoding;
mod hints;
mod load;
mod profile;

pub use column::ColumnConfig;
pub use durability::{DurabilityConfig, DurabilityMode};
pub use encoding::ColumnEncodingSpec;
pub use hints::{NumericBound, SemanticHint};
pub use load::{load_profile, parse_profile_str};
pub use profile::{
    BlockConfig, CompressionCodecName, CompressionConfig, CompressionProfile, FieldEncodingHint,
    InputFilterConfig, InputFormat, SchemaSection, SegmentConfig,
};

use crate::schema::Schema;

/// Runtime ingest bundle: a loaded profile plus an optional dataset label.
#[derive(Debug, Clone, PartialEq)]
pub struct IngestConfig {
    /// Loaded compression / ingest profile.
    pub profile: CompressionProfile,
    /// Optional dataset name for metrics and dashboard grouping.
    pub dataset_name: Option<String>,
}

impl IngestConfig {
    /// Creates an ingest configuration from a profile.
    pub fn new(profile: CompressionProfile) -> Self {
        let dataset_name = Some(profile.name.clone());
        Self {
            profile,
            dataset_name,
        }
    }

    /// Sets the dataset name.
    pub fn with_dataset_name(mut self, name: impl Into<String>) -> Self {
        self.dataset_name = Some(name.into());
        self
    }

    /// Schema derived from the profile columns.
    pub fn schema(&self) -> crate::error::Result<Schema> {
        self.profile.schema()
    }
}
