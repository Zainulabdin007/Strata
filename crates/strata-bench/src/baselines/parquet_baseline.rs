//! Parquet + Zstd baseline (Arrow-backed).
//!
//! Same logical columns as Strata generators. Compression: ZSTD (parquet crate
//! default level). This is a fair *columnar compressed file* baseline — not a
//! claim that Strata beats Parquet for every workload.

use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use arrow_array::{
    ArrayRef, BinaryArray, BooleanArray, Int64Array, RecordBatch, StringArray, UInt32Array,
    UInt64Array,
};
use arrow_schema::{DataType, Field as ArrowField, Schema as ArrowSchema};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use strata_core::{FieldValue, IntegerWidth, LogicalType, Record, Schema};

use super::{elapsed_ms, file_size, rps, FormatMeasurement};

fn arrow_type(lt: &LogicalType) -> DataType {
    match lt {
        LogicalType::Boolean(_) => DataType::Boolean,
        LogicalType::UnsignedInteger(t) => match t.width {
            IntegerWidth::Bits8 | IntegerWidth::Bits16 | IntegerWidth::Bits32 => DataType::UInt32,
            IntegerWidth::Bits64 => DataType::UInt64,
        },
        // Store signed / timestamp / decimal as Int64 for a simple, stable mapping.
        LogicalType::SignedInteger(_)
        | LogicalType::Timestamp(_)
        | LogicalType::FixedDecimal(_) => DataType::Int64,
        LogicalType::CategoricalString(_) | LogicalType::Enum(_) => DataType::Utf8,
        LogicalType::Binary(_) => DataType::Binary,
    }
}

fn to_arrow_schema(schema: &Schema) -> ArrowSchema {
    let fields: Vec<ArrowField> = schema
        .fields
        .iter()
        .map(|f| ArrowField::new(&f.name, arrow_type(&f.logical_type), f.nullable))
        .collect();
    ArrowSchema::new(fields)
}

fn build_batch(
    schema: &Schema,
    arrow_schema: &Arc<ArrowSchema>,
    records: &[Record],
) -> Result<RecordBatch> {
    for record in records {
        record.validate_against(schema)?;
    }
    let mut columns: Vec<ArrayRef> = Vec::with_capacity(schema.fields.len());
    let ordered = records.iter().all(|r| r.is_schema_ordered(schema));
    for (index, field) in schema.fields.iter().enumerate() {
        let col: ArrayRef = match &field.logical_type {
            LogicalType::Boolean(_) => {
                let v: Vec<Option<bool>> = records
                    .iter()
                    .map(|r| {
                        match if ordered {
                            r.get_ordinal(index)
                        } else {
                            r.get(&field.name)
                        } {
                            Some(FieldValue::Boolean(b)) => Some(*b),
                            _ => None,
                        }
                    })
                    .collect();
                Arc::new(BooleanArray::from(v))
            }
            LogicalType::UnsignedInteger(t) if t.width == IntegerWidth::Bits64 => {
                let v: Vec<Option<u64>> = records
                    .iter()
                    .map(|r| {
                        match if ordered {
                            r.get_ordinal(index)
                        } else {
                            r.get(&field.name)
                        } {
                            Some(FieldValue::UnsignedInteger(n)) => Some(*n),
                            _ => None,
                        }
                    })
                    .collect();
                Arc::new(UInt64Array::from(v))
            }
            LogicalType::UnsignedInteger(_) => {
                let v: Vec<Option<u32>> = records
                    .iter()
                    .map(|r| {
                        match if ordered {
                            r.get_ordinal(index)
                        } else {
                            r.get(&field.name)
                        } {
                            Some(FieldValue::UnsignedInteger(n)) => Some(*n as u32),
                            _ => None,
                        }
                    })
                    .collect();
                Arc::new(UInt32Array::from(v))
            }
            LogicalType::SignedInteger(_)
            | LogicalType::Timestamp(_)
            | LogicalType::FixedDecimal(_) => {
                let v: Vec<Option<i64>> = records
                    .iter()
                    .map(|r| {
                        match if ordered {
                            r.get_ordinal(index)
                        } else {
                            r.get(&field.name)
                        } {
                            Some(FieldValue::SignedInteger(n))
                            | Some(FieldValue::Timestamp(n))
                            | Some(FieldValue::FixedDecimal(n)) => Some(*n),
                            _ => None,
                        }
                    })
                    .collect();
                Arc::new(Int64Array::from(v))
            }
            LogicalType::CategoricalString(_) | LogicalType::Enum(_) => {
                let v: Vec<Option<&str>> = records
                    .iter()
                    .map(|r| {
                        match if ordered {
                            r.get_ordinal(index)
                        } else {
                            r.get(&field.name)
                        } {
                            Some(FieldValue::CategoricalString(s)) | Some(FieldValue::Enum(s)) => {
                                Some(s.as_str())
                            }
                            _ => None,
                        }
                    })
                    .collect();
                Arc::new(StringArray::from(v))
            }
            LogicalType::Binary(_) => {
                let v: Vec<Option<&[u8]>> = records
                    .iter()
                    .map(|r| {
                        match if ordered {
                            r.get_ordinal(index)
                        } else {
                            r.get(&field.name)
                        } {
                            Some(FieldValue::Binary(b)) => Some(b.as_slice()),
                            _ => None,
                        }
                    })
                    .collect();
                Arc::new(BinaryArray::from(v))
            }
        };
        columns.push(col);
    }
    RecordBatch::try_new(arrow_schema.clone(), columns).context("build arrow RecordBatch")
}

/// Streaming Parquet+Zstd writer.
pub struct ParquetZstdWriter {
    writer: ArrowWriter<std::fs::File>,
    arrow_schema: Arc<ArrowSchema>,
    schema: Schema,
    rows: u64,
}

impl ParquetZstdWriter {
    pub fn create(path: &Path, schema: &Schema) -> Result<Self> {
        let arrow_schema = Arc::new(to_arrow_schema(schema));
        let file =
            std::fs::File::create(path).with_context(|| format!("create {}", path.display()))?;
        let zstd = parquet::basic::ZstdLevel::try_new(3)
            .unwrap_or_else(|_| parquet::basic::ZstdLevel::default());
        let props = WriterProperties::builder()
            .set_compression(Compression::ZSTD(zstd))
            .build();
        let writer = ArrowWriter::try_new(file, arrow_schema.clone(), Some(props))
            .context("parquet ArrowWriter")?;
        Ok(Self {
            writer,
            arrow_schema,
            schema: schema.clone(),
            rows: 0,
        })
    }

    pub fn write_chunk(&mut self, records: &[Record]) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        let batch = build_batch(&self.schema, &self.arrow_schema, records)?;
        self.writer.write(&batch).context("parquet write batch")?;
        self.rows += records.len() as u64;
        Ok(())
    }

    pub fn finish(self) -> Result<u64> {
        let rows = self.rows;
        self.writer.close().context("parquet close")?;
        Ok(rows)
    }
}

#[allow(dead_code)]
pub fn encode_parquet_zstd(
    path: &Path,
    schema: &Schema,
    records: &[Record],
) -> Result<FormatMeasurement> {
    let t0 = Instant::now();
    let mut w = ParquetZstdWriter::create(path, schema)?;
    w.write_chunk(records)?;
    w.finish()?;
    let encode = t0.elapsed();
    Ok(FormatMeasurement {
        format: "parquet_zstd".into(),
        path: path.display().to_string(),
        size_bytes: file_size(path)?,
        encode_elapsed_ms: elapsed_ms(encode),
        encode_records_per_sec: rps(records.len() as u64, encode),
        decode_elapsed_ms: None,
        decode_records_per_sec: None,
        decode_records: None,
        notes: "Apache Parquet via arrow/parquet crates; Compression::ZSTD level 3".into(),
    })
}

pub fn decode_parquet_zstd(path: &Path) -> Result<(u64, std::time::Duration)> {
    let t0 = Instant::now();
    let file = std::fs::File::open(path).with_context(|| format!("open {}", path.display()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).context("parquet reader")?;
    let reader = builder.build().context("parquet build reader")?;
    let mut n = 0u64;
    for batch in reader {
        let batch = batch.context("parquet batch")?;
        n += batch.num_rows() as u64;
        std::hint::black_box(batch);
    }
    if n == 0 {
        // Empty file is ok; distinguish corrupt.
        let meta = std::fs::metadata(path)?;
        if meta.len() == 0 {
            bail!("empty parquet file");
        }
    }
    Ok((n, t0.elapsed()))
}

/// Fully materialize the same typed records for ordered lossless validation.
#[allow(dead_code)] // Also compiled as a module by the legacy benchmark executable.
pub fn decode_parquet_records(
    path: &Path,
    schema: &Schema,
    on_record: &mut dyn FnMut(Record) -> Result<()>,
) -> Result<u64> {
    use arrow_array::Array;
    let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(path)?)?
        .with_batch_size(16384)
        .build()?;
    let mut count = 0;
    for batch in reader {
        let batch = batch?;
        anyhow::ensure!(
            batch.num_columns() == schema.fields.len(),
            "Parquet column count mismatch"
        );
        for row in 0..batch.num_rows() {
            let mut record = Record::new();
            for (index, field) in schema.fields.iter().enumerate() {
                anyhow::ensure!(
                    batch.schema().field(index).name() == &field.name,
                    "Parquet field order mismatch"
                );
                let col = batch.column(index);
                let value = if col.is_null(row) {
                    FieldValue::Null
                } else {
                    match &field.logical_type {
                        LogicalType::Boolean(_) => FieldValue::Boolean(
                            col.as_any()
                                .downcast_ref::<BooleanArray>()
                                .context("Parquet bool type")?
                                .value(row),
                        ),
                        LogicalType::UnsignedInteger(t) if t.width == IntegerWidth::Bits64 => {
                            FieldValue::UnsignedInteger(
                                col.as_any()
                                    .downcast_ref::<UInt64Array>()
                                    .context("Parquet u64 type")?
                                    .value(row),
                            )
                        }
                        LogicalType::UnsignedInteger(_) => FieldValue::UnsignedInteger(
                            col.as_any()
                                .downcast_ref::<UInt32Array>()
                                .context("Parquet u32 type")?
                                .value(row) as u64,
                        ),
                        LogicalType::SignedInteger(_) => FieldValue::SignedInteger(
                            col.as_any()
                                .downcast_ref::<Int64Array>()
                                .context("Parquet i64 type")?
                                .value(row),
                        ),
                        LogicalType::Timestamp(_) => FieldValue::Timestamp(
                            col.as_any()
                                .downcast_ref::<Int64Array>()
                                .context("Parquet timestamp type")?
                                .value(row),
                        ),
                        LogicalType::FixedDecimal(_) => FieldValue::FixedDecimal(
                            col.as_any()
                                .downcast_ref::<Int64Array>()
                                .context("Parquet decimal type")?
                                .value(row),
                        ),
                        LogicalType::CategoricalString(_) => FieldValue::CategoricalString(
                            col.as_any()
                                .downcast_ref::<StringArray>()
                                .context("Parquet string type")?
                                .value(row)
                                .to_owned(),
                        ),
                        LogicalType::Enum(_) => FieldValue::Enum(
                            col.as_any()
                                .downcast_ref::<StringArray>()
                                .context("Parquet enum type")?
                                .value(row)
                                .to_owned(),
                        ),
                        LogicalType::Binary(_) => FieldValue::Binary(
                            col.as_any()
                                .downcast_ref::<BinaryArray>()
                                .context("Parquet binary type")?
                                .value(row)
                                .to_vec(),
                        ),
                    }
                };
                record = record.with(&field.name, value);
            }
            on_record(record)?;
            count += 1;
        }
    }
    Ok(count)
}

/// Decode every Arrow column in bounded batches without reconstructing rows.
#[allow(dead_code)] // Also included by the legacy benchmark executable.
pub fn scan_parquet_columns(path: &Path) -> Result<u64> {
    let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(path)?)?
        .with_batch_size(16384)
        .build()?;
    let mut count = 0u64;
    for batch in reader {
        let batch = batch?;
        count += batch.num_rows() as u64;
        std::hint::black_box(&batch);
    }
    Ok(count)
}

/// Reconstruct all fixed integer fields from Arrow arrays into canonical packed
/// rows. No map/string allocation per row; reject nulls, width and schema drift.
#[allow(dead_code)] // Also included by the legacy benchmark executable.
pub fn decode_parquet_integer_bytes(
    path: &Path,
    schema: &Schema,
    on_batch: &mut dyn FnMut(&[u8]) -> Result<()>,
) -> Result<u64> {
    let widths: Vec<_> = schema
        .fields
        .iter()
        .map(|f| match &f.logical_type {
            LogicalType::UnsignedInteger(t) => Ok((false, t.width.bytes() as usize)),
            LogicalType::SignedInteger(t) => Ok((true, t.width.bytes() as usize)),
            _ => bail!("integer canonical baseline requires integer fields"),
        })
        .collect::<Result<_>>()?;
    let width: usize = widths.iter().map(|(_, n)| n).sum();
    let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(path)?)?
        .with_batch_size(16384)
        .build()?;
    let mut count = 0;
    for batch in reader {
        let batch = batch?;
        anyhow::ensure!(
            batch.num_columns() == schema.len(),
            "Parquet column count mismatch"
        );
        let mut bytes = vec![0u8; batch.num_rows() * width];
        let mut offset = 0;
        for (i, &(signed, n)) in widths.iter().enumerate() {
            let column = batch.column(i);
            anyhow::ensure!(
                column.null_count() == 0
                    && batch.schema().field(i).name() == &schema.fields[i].name,
                "Parquet null/schema mismatch"
            );
            let mut store = |row: usize, word: [u8; 8]| {
                bytes[row * width + offset..row * width + offset + n].copy_from_slice(&word[..n]);
            };
            if signed {
                let values = column
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .context("Parquet signed type")?
                    .values();
                for (row, &v) in values.iter().enumerate() {
                    anyhow::ensure!(
                        n == 8 || (v >= -(1i64 << (n * 8 - 1)) && v < (1i64 << (n * 8 - 1))),
                        "Parquet signed width overflow"
                    );
                    store(row, v.to_le_bytes());
                }
            } else if n == 8 {
                let values = column
                    .as_any()
                    .downcast_ref::<UInt64Array>()
                    .context("Parquet u64 type")?
                    .values();
                for (row, &v) in values.iter().enumerate() {
                    store(row, v.to_le_bytes());
                }
            } else {
                let values = column
                    .as_any()
                    .downcast_ref::<UInt32Array>()
                    .context("Parquet u32 type")?
                    .values();
                for (row, &v) in values.iter().enumerate() {
                    anyhow::ensure!(
                        n == 4 || u64::from(v) < (1u64 << (n * 8)),
                        "Parquet unsigned width overflow"
                    );
                    store(row, u64::from(v).to_le_bytes());
                }
            }
            offset += n;
        }
        on_batch(&bytes)?;
        count += batch.num_rows() as u64;
    }
    Ok(count)
}

/// Decode independent Parquet row groups on the same requested worker budget.
/// Each worker streams Arrow batches; no full-file row accumulation.
#[allow(dead_code)]
pub fn scan_parquet_columns_parallel(path: &Path, workers: usize) -> Result<u64> {
    anyhow::ensure!((1..=32).contains(&workers), "decode workers must be 1..32");
    let builder = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(path)?)?;
    let groups = builder.metadata().num_row_groups();
    let workers = workers.min(groups).max(1);
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        for worker in 0..workers {
            handles.push(scope.spawn(move || -> Result<u64> {
                let selected = (worker..groups).step_by(workers).collect();
                let reader = ParquetRecordBatchReaderBuilder::try_new(std::fs::File::open(path)?)?
                    .with_row_groups(selected)
                    .with_batch_size(16384)
                    .build()?;
                let mut rows = 0;
                for batch in reader {
                    let batch = batch?;
                    rows += batch.num_rows() as u64;
                    std::hint::black_box(&batch);
                }
                Ok(rows)
            }));
        }
        let mut rows = 0;
        for handle in handles {
            rows += handle
                .join()
                .map_err(|_| anyhow::anyhow!("Parquet scan worker panicked"))??;
        }
        Ok(rows)
    })
}
