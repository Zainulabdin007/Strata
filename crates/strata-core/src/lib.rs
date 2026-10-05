//! Core types for the Strata storage engine.
//!
//! This crate defines schemas, logical types, record batches, configuration,
//! and shared error types. Profile loading reads TOML from disk; parsers live
//! in `strata-input`.

#![deny(missing_docs)]

pub mod config;
pub mod error;
pub mod record;
pub mod schema;
pub mod types;

pub use config::{
    load_profile, parse_profile_str, BlockConfig, ColumnConfig, ColumnEncodingSpec,
    CompressionCodecName, CompressionConfig, CompressionProfile, DurabilityConfig, DurabilityMode,
    FieldEncodingHint, IngestConfig, InputFilterConfig, InputFormat, NumericBound, SegmentConfig,
    SemanticHint,
};
pub use error::{Error, Result};
pub use record::{validate_field_value, FieldValue, Record, RecordBatch, RecordValues};
pub use schema::{Field, Schema, SchemaId};
pub use types::{
    BinaryType, BooleanType, CategoricalStringType, EnumType, FixedDecimalType, IntegerWidth,
    LogicalType, SignedIntegerType, TimestampType, TimestampUnit, UnsignedIntegerType,
};

pub use record::logical_value_bytes;
