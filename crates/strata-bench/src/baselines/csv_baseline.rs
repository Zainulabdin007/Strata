//! CSV baseline (header + one row per record). Appropriate for flat tabular schemas.

use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use strata_core::{FieldValue, Record, Schema};

use super::{buf_reader, buf_writer, elapsed_ms, file_size, rps, FormatMeasurement};

fn csv_cell(v: &FieldValue) -> String {
    match v {
        FieldValue::Null => String::new(),
        FieldValue::Boolean(b) => if *b { "true" } else { "false" }.into(),
        FieldValue::UnsignedInteger(n) => n.to_string(),
        FieldValue::SignedInteger(n) | FieldValue::Timestamp(n) | FieldValue::FixedDecimal(n) => {
            n.to_string()
        }
        FieldValue::CategoricalString(s) | FieldValue::Enum(s) => s.clone(),
        FieldValue::Binary(b) => {
            // Hex for binary so CSV stays text-safe.
            b.iter().map(|x| format!("{x:02x}")).collect()
        }
    }
}

/// Streaming CSV writer.
pub struct CsvWriter {
    w: csv::Writer<std::io::BufWriter<std::fs::File>>,
    header_written: bool,
}

impl CsvWriter {
    pub fn create(path: &Path) -> Result<Self> {
        let file = buf_writer(path)?;
        Ok(Self {
            w: csv::Writer::from_writer(file),
            header_written: false,
        })
    }

    pub fn write_chunk(&mut self, schema: &Schema, records: &[Record]) -> Result<()> {
        if !self.header_written {
            self.w
                .write_record(schema.fields.iter().map(|f| f.name.as_str()))?;
            self.header_written = true;
        }
        for rec in records {
            let row: Vec<String> = schema
                .fields
                .iter()
                .map(|f| rec.get(&f.name).map(csv_cell).unwrap_or_default())
                .collect();
            self.w.write_record(&row)?;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<()> {
        self.w.flush()?;
        Ok(())
    }
}

#[allow(dead_code)]
pub fn encode_csv(path: &Path, schema: &Schema, records: &[Record]) -> Result<FormatMeasurement> {
    let t0 = Instant::now();
    let mut w = CsvWriter::create(path)?;
    w.write_chunk(schema, records)?;
    w.finish()?;
    let encode = t0.elapsed();
    Ok(FormatMeasurement {
        format: "csv".into(),
        path: path.display().to_string(),
        size_bytes: file_size(path)?,
        encode_elapsed_ms: elapsed_ms(encode),
        encode_records_per_sec: rps(records.len() as u64, encode),
        decode_elapsed_ms: None,
        decode_records_per_sec: None,
        decode_records: None,
        notes: "RFC4180-ish via csv crate; header row; flat tabular only".into(),
    })
}

pub fn decode_csv(path: &Path) -> Result<(u64, std::time::Duration)> {
    let t0 = Instant::now();
    let r = buf_reader(path)?;
    let mut rdr = csv::Reader::from_reader(r);
    let mut n = 0u64;
    for row in rdr.records() {
        let _ = row.with_context(|| "csv row")?;
        n += 1;
    }
    Ok((n, t0.elapsed()))
}
