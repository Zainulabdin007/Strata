//! Top-level analysis report assembly.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use strata_core::{FieldValue, Record, Schema};

use crate::error::Result;
use crate::stats::{profile_columns, ColumnAccumulator, ColumnStats};

/// Options controlling analysis.
#[derive(Debug, Clone)]
pub struct AnalyzeOptions {
    /// Row window size for local block range statistics.
    pub block_rows: u32,
}

impl Default for AnalyzeOptions {
    fn default() -> Self {
        Self { block_rows: 65_536 }
    }
}

/// Streaming profiler over records.
#[derive(Debug)]
pub struct Profiler {
    source: String,
    schema_id: String,
    block_rows: u32,
    file_count: u64,
    record_count: u64,
    columns: Vec<ColumnAccumulator>,
}

impl Profiler {
    /// Creates a profiler for `schema`.
    pub fn new(source: impl Into<String>, schema: &Schema, options: &AnalyzeOptions) -> Self {
        let columns = schema
            .fields
            .iter()
            .map(|f| {
                ColumnAccumulator::new(f.name.clone(), f.logical_type.clone(), options.block_rows)
            })
            .collect();
        Self {
            source: source.into(),
            schema_id: schema.id.as_str().to_string(),
            block_rows: options.block_rows,
            file_count: 0,
            record_count: 0,
            columns,
        }
    }

    /// Records how many input files were scanned.
    pub fn set_file_count(&mut self, file_count: u64) {
        self.file_count = file_count;
    }

    /// Observes one record.
    pub fn push_record(&mut self, schema: &Schema, record: &Record) {
        self.record_count += 1;
        for (i, field) in schema.fields.iter().enumerate() {
            let value = record.get(&field.name).cloned().unwrap_or(FieldValue::Null);
            self.columns[i].push(&value);
        }
    }

    /// Observes many records.
    pub fn push_records(&mut self, schema: &Schema, records: &[Record]) {
        for record in records {
            self.push_record(schema, record);
        }
    }

    /// Finalizes the report.
    pub fn finish(self) -> AnalysisReport {
        AnalysisReport {
            source: self.source,
            schema_id: self.schema_id,
            record_count: self.record_count,
            file_count: self.file_count,
            block_rows: self.block_rows,
            columns: self
                .columns
                .into_iter()
                .map(ColumnAccumulator::finish)
                .collect(),
        }
    }
}

/// Full profiler report for a dataset (read-only analysis).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisReport {
    /// Optional dataset / input label.
    pub source: String,
    /// Schema id when known.
    pub schema_id: String,
    /// Total records observed.
    pub record_count: u64,
    /// Files analyzed (when discovery was used).
    pub file_count: u64,
    /// Block window used for local ranges.
    pub block_rows: u32,
    /// Per-column statistics in schema order.
    pub columns: Vec<ColumnStats>,
}

impl AnalysisReport {
    /// Builds a report from an in-memory record stream and schema.
    pub fn from_records(
        source: impl Into<String>,
        schema: &Schema,
        records: &[Record],
        options: &AnalyzeOptions,
        file_count: u64,
    ) -> Result<Self> {
        let mut profiler = Profiler::new(source, schema, options);
        profiler.set_file_count(file_count);
        profiler.push_records(schema, records);
        Ok(profiler.finish())
    }

    /// Builds a report from row maps (tests / custom loaders).
    pub fn from_rows(
        source: impl Into<String>,
        schema: &Schema,
        rows: Vec<IndexMap<String, FieldValue>>,
        options: &AnalyzeOptions,
        file_count: u64,
    ) -> Result<Self> {
        let field_order: Vec<_> = schema
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.logical_type.clone()))
            .collect();
        let record_count = rows.len() as u64;
        let columns = profile_columns(&field_order, rows, options.block_rows);
        Ok(Self {
            source: source.into(),
            schema_id: schema.id.as_str().to_string(),
            record_count,
            file_count,
            block_rows: options.block_rows,
            columns,
        })
    }
}
