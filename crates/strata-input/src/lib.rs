//! Input discovery and parsers for Strata ingest.
//!
//! CSV and JSONL are implemented. Parquet and binary formats are reserved via
//! the [`RecordParser`] interface for later phases.

#![deny(missing_docs)]

pub mod dbn;
pub mod discover;
pub mod error;
pub mod glob;
pub mod parse;
pub mod value;

pub use discover::{discover_inputs, discover_inputs_with, DiscoverOptions, DiscoveredInputs};
pub use error::{Error, Result};
pub use parse::{open_parser, CsvParser, JsonlParser, ParseOptions, RecordParser};

mod bounded;
