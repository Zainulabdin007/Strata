//! JSONL baseline (one JSON object per line, schema field order).

use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use strata_core::{FieldValue, Record, Schema};

use super::{buf_reader, buf_writer, elapsed_ms, file_size, rps, FormatMeasurement};

fn value_to_json(v: &FieldValue) -> Value {
    match v {
        FieldValue::Null => Value::Null,
        FieldValue::Boolean(b) => json!(b),
        FieldValue::UnsignedInteger(n) => json!(n),
        FieldValue::SignedInteger(n) => json!(n),
        FieldValue::Timestamp(n) => json!(n),
        FieldValue::FixedDecimal(n) => json!(n),
        FieldValue::CategoricalString(s) | FieldValue::Enum(s) => json!(s),
        FieldValue::Binary(b) => json!(b),
    }
}

fn record_to_json(schema: &Schema, record: &Record) -> Value {
    let mut map = Map::new();
    for field in &schema.fields {
        let v = record
            .get(&field.name)
            .map(value_to_json)
            .unwrap_or(Value::Null);
        map.insert(field.name.clone(), v);
    }
    Value::Object(map)
}

/// Streaming JSONL writer.
pub struct JsonlWriter {
    w: std::io::BufWriter<std::fs::File>,
}

impl JsonlWriter {
    pub fn create(path: &Path) -> Result<Self> {
        Ok(Self {
            w: buf_writer(path)?,
        })
    }

    pub fn write_chunk(&mut self, schema: &Schema, records: &[Record]) -> Result<()> {
        for rec in records {
            let v = record_to_json(schema, rec);
            serde_json::to_writer(&mut self.w, &v)?;
            self.w.write_all(b"\n")?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.w.flush()?;
        Ok(())
    }
}

/// Encode all records already buffered (small sizes) — prefer streaming for large.
#[allow(dead_code)]
pub fn encode_jsonl(path: &Path, schema: &Schema, records: &[Record]) -> Result<FormatMeasurement> {
    let t0 = Instant::now();
    let mut w = JsonlWriter::create(path)?;
    w.write_chunk(schema, records)?;
    w.finish()?;
    let encode = t0.elapsed();
    let size = file_size(path)?;
    Ok(FormatMeasurement {
        format: "jsonl".into(),
        path: path.display().to_string(),
        size_bytes: size,
        encode_elapsed_ms: elapsed_ms(encode),
        encode_records_per_sec: rps(records.len() as u64, encode),
        decode_elapsed_ms: None,
        decode_records_per_sec: None,
        decode_records: None,
        notes: "NDJSON objects; field names repeated per row; UTF-8 text".into(),
    })
}

/// Decode / scan JSONL counting objects (does not rebuild Strata records).
pub fn decode_jsonl(path: &Path) -> Result<(u64, std::time::Duration)> {
    let t0 = Instant::now();
    let r = buf_reader(path)?;
    let mut n = 0u64;
    for line in r.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&line).with_context(|| "jsonl parse")?;
        std::hint::black_box(value);
        n += 1;
    }
    Ok((n, t0.elapsed()))
}
