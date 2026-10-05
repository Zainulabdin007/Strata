//! Naive fixed-width binary baseline.
//!
//! Little-endian packed rows. Variable strings/enums/binary are padded (overflow is rejected)
//! to a per-field fixed slot derived from the schema (max_bytes or max enum
//! variant length). This is intentionally simple — not a production format.

use std::io::Write;
use std::path::Path;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema};

use super::{buf_writer, elapsed_ms, file_size, rps, FormatMeasurement};

const MAGIC: &[u8; 4] = b"SFIX";
const VERSION: u32 = 1;

/// Per-field fixed slot width.
#[derive(Debug, Clone)]
pub struct FixedField {
    pub name: String,
    pub width: usize,
}

/// Layout for a schema.
#[derive(Debug, Clone)]
pub struct FixedLayout {
    pub fields: Vec<FixedField>,
    pub row_width: usize,
}

impl FixedLayout {
    pub fn from_schema(schema: &Schema) -> Self {
        let fields: Vec<FixedField> = schema
            .fields
            .iter()
            .map(|f| FixedField {
                name: f.name.clone(),
                width: slot_width(f),
            })
            .collect();
        let row_width = fields.iter().map(|f| f.width).sum();
        Self { fields, row_width }
    }
}

fn slot_width(field: &Field) -> usize {
    match &field.logical_type {
        LogicalType::Boolean(_) => 1,
        LogicalType::UnsignedInteger(t) => t.width.bytes() as usize,
        LogicalType::SignedInteger(t) => t.width.bytes() as usize,
        LogicalType::Timestamp(_) => IntegerWidth::Bits64.bytes() as usize,
        LogicalType::FixedDecimal(t) => t.width.bytes() as usize,
        LogicalType::CategoricalString(t) => t.max_bytes.unwrap_or(32) as usize,
        LogicalType::Enum(t) => t.variants.iter().map(|v| v.len()).max().unwrap_or(8).max(1),
        LogicalType::Binary(t) => t.max_bytes.unwrap_or(32) as usize,
    }
}

fn write_int(buf: &mut [u8], width: IntegerWidth, unsigned: bool, u: u64, i: i64) {
    match (width, unsigned) {
        (IntegerWidth::Bits8, true) => buf[0] = u as u8,
        (IntegerWidth::Bits16, true) => buf[..2].copy_from_slice(&(u as u16).to_le_bytes()),
        (IntegerWidth::Bits32, true) => buf[..4].copy_from_slice(&(u as u32).to_le_bytes()),
        (IntegerWidth::Bits64, true) => buf[..8].copy_from_slice(&u.to_le_bytes()),
        (IntegerWidth::Bits8, false) => buf[0] = i as u8,
        (IntegerWidth::Bits16, false) => buf[..2].copy_from_slice(&(i as i16).to_le_bytes()),
        (IntegerWidth::Bits32, false) => buf[..4].copy_from_slice(&(i as i32).to_le_bytes()),
        (IntegerWidth::Bits64, false) => buf[..8].copy_from_slice(&i.to_le_bytes()),
    }
}

fn pack_field(field: &Field, value: &FieldValue, out: &mut [u8]) -> Result<()> {
    out.fill(0);
    match (&field.logical_type, value) {
        (_, FieldValue::Null) => bail!("fixed baseline does not represent nulls losslessly"),
        (LogicalType::Boolean(_), FieldValue::Boolean(b)) => {
            out[0] = u8::from(*b);
            Ok(())
        }
        (LogicalType::UnsignedInteger(t), FieldValue::UnsignedInteger(n)) => {
            write_int(out, t.width, true, *n, 0);
            Ok(())
        }
        (LogicalType::SignedInteger(t), FieldValue::SignedInteger(n)) => {
            write_int(out, t.width, false, 0, *n);
            Ok(())
        }
        (LogicalType::Timestamp(_), FieldValue::Timestamp(n)) => {
            write_int(out, IntegerWidth::Bits64, false, 0, *n);
            Ok(())
        }
        (LogicalType::FixedDecimal(t), FieldValue::FixedDecimal(n)) => {
            write_int(out, t.width, false, 0, *n);
            Ok(())
        }
        (LogicalType::CategoricalString(_), FieldValue::CategoricalString(s))
        | (LogicalType::Enum(_), FieldValue::Enum(s)) => {
            let bytes = s.as_bytes();
            anyhow::ensure!(
                bytes.len() <= out.len() && !bytes.contains(&0),
                "fixed string does not fit losslessly"
            );
            let n = bytes.len();
            out[..n].copy_from_slice(&bytes[..n]);
            Ok(())
        }
        (LogicalType::Binary(_), FieldValue::Binary(b)) => {
            anyhow::ensure!(b.len() == out.len(), "fixed binary needs exact slot width");
            let n = b.len();
            out[..n].copy_from_slice(&b[..n]);
            Ok(())
        }
        _ => bail!(
            "fixed pack type mismatch for field `{}` ({})",
            field.name,
            value.kind_name()
        ),
    }
}

/// Streaming fixed-width writer.
pub struct FixedWriter {
    w: std::io::BufWriter<std::fs::File>,
    layout: FixedLayout,
    schema: Schema,
    rows: u64,
    header_written: bool,
}

impl FixedWriter {
    pub fn create(path: &Path, schema: &Schema) -> Result<Self> {
        Ok(Self {
            w: buf_writer(path)?,
            layout: FixedLayout::from_schema(schema),
            schema: schema.clone(),
            rows: 0,
            header_written: false,
        })
    }

    fn write_header(&mut self) -> Result<()> {
        self.w.write_all(MAGIC)?;
        self.w.write_all(&VERSION.to_le_bytes())?;
        self.w
            .write_all(&(self.layout.fields.len() as u32).to_le_bytes())?;
        self.w
            .write_all(&(self.layout.row_width as u32).to_le_bytes())?;
        for f in &self.layout.fields {
            let name = f.name.as_bytes();
            self.w.write_all(&(name.len() as u16).to_le_bytes())?;
            self.w.write_all(name)?;
            self.w.write_all(&(f.width as u16).to_le_bytes())?;
        }
        // Placeholder row count (patched on finish by rewriting — we append count at end instead).
        self.header_written = true;
        Ok(())
    }

    pub fn write_chunk(&mut self, records: &[Record]) -> Result<()> {
        if !self.header_written {
            self.write_header()?;
        }
        let mut row = vec![0u8; self.layout.row_width];
        for rec in records {
            rec.validate_against(&self.schema)?;
            let ordered = rec.is_schema_ordered(&self.schema);
            let mut offset = 0;
            for (index, (field, slot)) in self
                .schema
                .fields
                .iter()
                .zip(self.layout.fields.iter())
                .enumerate()
            {
                let width = slot.width;
                let cell = &mut row[offset..offset + width];
                let val = (if ordered {
                    rec.get_ordinal(index)
                } else {
                    rec.get(&field.name)
                })
                .unwrap_or(&FieldValue::Null);
                pack_field(field, val, cell)?;
                offset += width;
            }
            self.w.write_all(&row)?;
            self.rows += 1;
        }
        Ok(())
    }

    pub fn finish(mut self) -> Result<u64> {
        if !self.header_written {
            self.write_header()?;
        }
        // Trailer: row count for decoders that skip header math.
        self.w.write_all(&self.rows.to_le_bytes())?;
        self.w.flush()?;
        Ok(self.rows)
    }

    #[allow(dead_code)]
    pub fn row_width(&self) -> usize {
        self.layout.row_width
    }
}

#[allow(dead_code)]
pub fn encode_fixed(path: &Path, schema: &Schema, records: &[Record]) -> Result<FormatMeasurement> {
    let t0 = Instant::now();
    let mut w = FixedWriter::create(path, schema)?;
    let row_width = w.row_width();
    w.write_chunk(records)?;
    w.finish()?;
    let encode = t0.elapsed();
    Ok(FormatMeasurement {
        format: "fixed_binary".into(),
        path: path.display().to_string(),
        size_bytes: file_size(path)?,
        encode_elapsed_ms: elapsed_ms(encode),
        encode_records_per_sec: rps(records.len() as u64, encode),
        decode_elapsed_ms: None,
        decode_records_per_sec: None,
        decode_records: None,
        notes: format!(
            "naive LE packed rows; row_width={row_width}; strings zero-padded/truncated"
        ),
    })
}

/// Stream and materialize every fixed row; schema is embedded in field names/widths,
/// while the caller supplies logical types (this benchmark format stores no type tags).
pub fn decode_fixed_typed(path: &Path, schema: &Schema) -> Result<(u64, std::time::Duration)> {
    decode_fixed_records(path, schema, &mut |record| {
        std::hint::black_box(record);
        Ok(())
    })
}

/// Decode every typed record through a caller's validation sink.
pub fn decode_fixed_records(
    path: &Path,
    schema: &Schema,
    on_record: &mut dyn FnMut(Record) -> Result<()>,
) -> Result<(u64, std::time::Duration)> {
    use std::io::Read;
    let t0 = Instant::now();
    let mut r = super::buf_reader(path)?;
    let mut header = [0u8; 16];
    r.read_exact(&mut header)?;
    anyhow::ensure!(
        &header[..4] == MAGIC && u32::from_le_bytes(header[4..8].try_into()?) == VERSION,
        "bad fixed header"
    );
    let fields = u32::from_le_bytes(header[8..12].try_into()?) as usize;
    let width = u32::from_le_bytes(header[12..16].try_into()?) as usize;
    let layout = FixedLayout::from_schema(schema);
    anyhow::ensure!(
        fields == schema.fields.len() && width == layout.row_width && width > 0,
        "fixed schema mismatch"
    );
    let mut offset = 16u64;
    for expected in &layout.fields {
        let mut two = [0; 2];
        r.read_exact(&mut two)?;
        let len = u16::from_le_bytes(two) as usize;
        let mut name = vec![0; len];
        r.read_exact(&mut name)?;
        r.read_exact(&mut two)?;
        anyhow::ensure!(
            name == expected.name.as_bytes() && u16::from_le_bytes(two) as usize == expected.width,
            "fixed field mismatch"
        );
        offset += 4 + len as u64;
    }
    let payload = std::fs::metadata(path)?
        .len()
        .checked_sub(offset + 8)
        .context("truncated fixed file")?;
    anyhow::ensure!(payload % width as u64 == 0, "truncated fixed row");
    let rows = payload / width as u64;
    let mut bytes = vec![0; width];
    for _ in 0..rows {
        r.read_exact(&mut bytes)?;
        let mut offset = 0;
        let mut rec = Record::new();
        for (field, slot) in schema.fields.iter().zip(&layout.fields) {
            let cell = &bytes[offset..offset + slot.width];
            offset += slot.width;
            let mut unsigned = [0; 8];
            if cell.len() <= 8 {
                unsigned[..cell.len()].copy_from_slice(cell);
            }
            let mut signed = [if cell.last().copied().unwrap_or(0) & 128 != 0 {
                255
            } else {
                0
            }; 8];
            if cell.len() <= 8 {
                signed[..cell.len()].copy_from_slice(cell);
            }
            let value = match &field.logical_type {
                LogicalType::UnsignedInteger(_) => {
                    FieldValue::UnsignedInteger(u64::from_le_bytes(unsigned))
                }
                LogicalType::SignedInteger(_) => {
                    FieldValue::SignedInteger(i64::from_le_bytes(signed))
                }
                LogicalType::Timestamp(_) => FieldValue::Timestamp(i64::from_le_bytes(signed)),
                LogicalType::FixedDecimal(_) => {
                    FieldValue::FixedDecimal(i64::from_le_bytes(signed))
                }
                LogicalType::Boolean(_) => {
                    anyhow::ensure!(cell[0] <= 1, "invalid fixed boolean");
                    FieldValue::Boolean(cell[0] == 1)
                }
                LogicalType::CategoricalString(_) | LogicalType::Enum(_) => {
                    let n = cell.iter().position(|b| *b == 0).unwrap_or(cell.len());
                    let value = std::str::from_utf8(&cell[..n])?.to_owned();
                    if matches!(field.logical_type, LogicalType::Enum(_)) {
                        FieldValue::Enum(value)
                    } else {
                        FieldValue::CategoricalString(value)
                    }
                }
                LogicalType::Binary(_) => FieldValue::Binary(cell.to_vec()),
            };
            rec = rec.with(&field.name, value);
        }
        on_record(rec)?;
    }
    let mut trailer = [0; 8];
    r.read_exact(&mut trailer)?;
    anyhow::ensure!(u64::from_le_bytes(trailer) == rows, "fixed count mismatch");
    Ok((rows, t0.elapsed()))
}
