//! Column encoding interfaces and PLAIN codec.

use strata_core::{FieldValue, IntegerWidth, LogicalType};

use crate::error::{Error, Result};

/// Stable identifier for an encoder implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EncoderId(pub &'static str);

/// Encodes a column of logical values into bytes.
pub trait ColumnEncoder: Send + Sync {
    /// Stable id for metrics and format metadata.
    fn id(&self) -> EncoderId;

    /// Returns true when this encoder can handle the logical type.
    fn supports(&self, logical_type: &LogicalType) -> bool;

    /// Encodes `values` (length = row count, may include nulls).
    fn encode(&self, logical_type: &LogicalType, values: &[FieldValue]) -> Result<Vec<u8>>;
}

/// Decodes a column of logical values from bytes.
pub trait ColumnDecoder: Send + Sync {
    /// Decodes `row_count` values from `bytes`.
    fn decode(
        &self,
        logical_type: &LogicalType,
        row_count: u32,
        bytes: &[u8],
    ) -> Result<Vec<FieldValue>>;
}

/// PLAIN columnar encoding: null bitmap + dense typed present values.
///
/// Layout (little-endian):
/// ```text
/// bitmap_len: u32
/// null_bitmap: [u8; bitmap_len]   // bit i set => row i is non-null
/// values: typed payloads for each non-null row in order
/// ```
#[derive(Debug, Default, Clone, Copy)]
pub struct PlainEncoder;

impl ColumnEncoder for PlainEncoder {
    fn id(&self) -> EncoderId {
        EncoderId("plain")
    }

    fn supports(&self, _logical_type: &LogicalType) -> bool {
        true
    }

    fn encode(&self, logical_type: &LogicalType, values: &[FieldValue]) -> Result<Vec<u8>> {
        let row_count = values.len();
        let bitmap_len = row_count.div_ceil(8);
        let mut bitmap = vec![0u8; bitmap_len];
        let mut payload = Vec::new();

        for (i, value) in values.iter().enumerate() {
            if value.is_null() {
                continue;
            }
            bitmap[i / 8] |= 1 << (i % 8);
            encode_present(logical_type, value, &mut payload)?;
        }

        let mut out = Vec::with_capacity(4 + bitmap.len() + payload.len());
        let bitmap_len_u32 = u32::try_from(bitmap.len()).map_err(|_| Error::InvalidInput {
            message: "null bitmap too large".into(),
        })?;
        out.extend_from_slice(&bitmap_len_u32.to_le_bytes());
        out.extend_from_slice(&bitmap);
        out.extend_from_slice(&payload);
        Ok(out)
    }
}

impl ColumnDecoder for PlainEncoder {
    fn decode(
        &self,
        logical_type: &LogicalType,
        row_count: u32,
        bytes: &[u8],
    ) -> Result<Vec<FieldValue>> {
        crate::limits::check_count(row_count as usize)?;
        let mut cursor = Cursor::new(bytes);
        let bitmap_len = cursor.read_u32()? as usize;
        let bitmap = cursor.read_exact(bitmap_len)?;
        let expected = (row_count as usize).div_ceil(8);
        if bitmap_len != expected {
            return Err(Error::InvalidInput {
                message: format!(
                    "PLAIN null bitmap length {bitmap_len} does not match row_count {row_count}"
                ),
            });
        }

        let mut values = Vec::with_capacity(row_count as usize);
        for i in 0..row_count as usize {
            let present = bitmap[i / 8] & (1 << (i % 8)) != 0;
            if present {
                values.push(decode_present(logical_type, &mut cursor)?);
            } else {
                values.push(FieldValue::Null);
            }
        }

        if !cursor.is_empty() {
            return Err(Error::InvalidInput {
                message: format!(
                    "PLAIN column has {} trailing bytes after decoding {row_count} rows",
                    cursor.remaining()
                ),
            });
        }

        Ok(values)
    }
}

fn encode_present(logical_type: &LogicalType, value: &FieldValue, out: &mut Vec<u8>) -> Result<()> {
    match (logical_type, value) {
        (LogicalType::Boolean(_), FieldValue::Boolean(v)) => {
            out.push(u8::from(*v));
            Ok(())
        }
        (LogicalType::UnsignedInteger(t), FieldValue::UnsignedInteger(v)) => {
            write_unsigned(out, *v, t.width)
        }
        (LogicalType::SignedInteger(t), FieldValue::SignedInteger(v)) => {
            write_signed(out, *v, t.width)
        }
        (LogicalType::Timestamp(_), FieldValue::Timestamp(v)) => {
            out.extend_from_slice(&v.to_le_bytes());
            Ok(())
        }
        (LogicalType::FixedDecimal(t), FieldValue::FixedDecimal(v)) => {
            write_signed(out, *v, t.width)
        }
        (LogicalType::CategoricalString(_), FieldValue::CategoricalString(s))
        | (LogicalType::Enum(_), FieldValue::Enum(s)) => write_bytes(out, s.as_bytes()),
        (LogicalType::Binary(_), FieldValue::Binary(b)) => write_bytes(out, b),
        _ => Err(Error::InvalidInput {
            message: format!(
                "PLAIN encode type mismatch: logical={}, value={}",
                logical_type.name(),
                value.kind_name()
            ),
        }),
    }
}

fn decode_present(logical_type: &LogicalType, cursor: &mut Cursor<'_>) -> Result<FieldValue> {
    match logical_type {
        LogicalType::Boolean(_) => {
            let v = cursor.read_u8()?;
            match v {
                0 => Ok(FieldValue::Boolean(false)),
                1 => Ok(FieldValue::Boolean(true)),
                _ => Err(Error::InvalidInput {
                    message: format!("invalid boolean byte {v}"),
                }),
            }
        }
        LogicalType::UnsignedInteger(t) => {
            Ok(FieldValue::UnsignedInteger(read_unsigned(cursor, t.width)?))
        }
        LogicalType::SignedInteger(t) => {
            Ok(FieldValue::SignedInteger(read_signed(cursor, t.width)?))
        }
        LogicalType::Timestamp(_) => {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(cursor.read_exact(8)?);
            Ok(FieldValue::Timestamp(i64::from_le_bytes(buf)))
        }
        LogicalType::FixedDecimal(t) => Ok(FieldValue::FixedDecimal(read_signed(cursor, t.width)?)),
        LogicalType::CategoricalString(_) => {
            let bytes = read_bytes(cursor)?;
            let s = String::from_utf8(bytes).map_err(|e| Error::InvalidInput {
                message: format!("invalid UTF-8 in categorical string: {e}"),
            })?;
            Ok(FieldValue::CategoricalString(s))
        }
        LogicalType::Enum(_) => {
            let bytes = read_bytes(cursor)?;
            let s = String::from_utf8(bytes).map_err(|e| Error::InvalidInput {
                message: format!("invalid UTF-8 in enum: {e}"),
            })?;
            Ok(FieldValue::Enum(s))
        }
        LogicalType::Binary(_) => Ok(FieldValue::Binary(read_bytes(cursor)?)),
    }
}

fn write_unsigned(out: &mut Vec<u8>, v: u64, width: IntegerWidth) -> Result<()> {
    match width {
        IntegerWidth::Bits8 => {
            let n = u8::try_from(v).map_err(|_| Error::InvalidInput {
                message: format!("u64 value {v} does not fit in bits8"),
            })?;
            out.push(n);
        }
        IntegerWidth::Bits16 => {
            let n = u16::try_from(v).map_err(|_| Error::InvalidInput {
                message: format!("u64 value {v} does not fit in bits16"),
            })?;
            out.extend_from_slice(&n.to_le_bytes());
        }
        IntegerWidth::Bits32 => {
            let n = u32::try_from(v).map_err(|_| Error::InvalidInput {
                message: format!("u64 value {v} does not fit in bits32"),
            })?;
            out.extend_from_slice(&n.to_le_bytes());
        }
        IntegerWidth::Bits64 => out.extend_from_slice(&v.to_le_bytes()),
    }
    Ok(())
}

fn write_signed(out: &mut Vec<u8>, v: i64, width: IntegerWidth) -> Result<()> {
    match width {
        IntegerWidth::Bits8 => {
            let n = i8::try_from(v).map_err(|_| Error::InvalidInput {
                message: format!("i64 value {v} does not fit in bits8"),
            })?;
            out.extend_from_slice(&n.to_le_bytes());
        }
        IntegerWidth::Bits16 => {
            let n = i16::try_from(v).map_err(|_| Error::InvalidInput {
                message: format!("i64 value {v} does not fit in bits16"),
            })?;
            out.extend_from_slice(&n.to_le_bytes());
        }
        IntegerWidth::Bits32 => {
            let n = i32::try_from(v).map_err(|_| Error::InvalidInput {
                message: format!("i64 value {v} does not fit in bits32"),
            })?;
            out.extend_from_slice(&n.to_le_bytes());
        }
        IntegerWidth::Bits64 => out.extend_from_slice(&v.to_le_bytes()),
    }
    Ok(())
}

fn read_unsigned(cursor: &mut Cursor<'_>, width: IntegerWidth) -> Result<u64> {
    Ok(match width {
        IntegerWidth::Bits8 => u64::from(cursor.read_u8()?),
        IntegerWidth::Bits16 => {
            let mut buf = [0u8; 2];
            buf.copy_from_slice(cursor.read_exact(2)?);
            u64::from(u16::from_le_bytes(buf))
        }
        IntegerWidth::Bits32 => {
            let mut buf = [0u8; 4];
            buf.copy_from_slice(cursor.read_exact(4)?);
            u64::from(u32::from_le_bytes(buf))
        }
        IntegerWidth::Bits64 => {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(cursor.read_exact(8)?);
            u64::from_le_bytes(buf)
        }
    })
}

fn read_signed(cursor: &mut Cursor<'_>, width: IntegerWidth) -> Result<i64> {
    Ok(match width {
        IntegerWidth::Bits8 => {
            let mut buf = [0u8; 1];
            buf.copy_from_slice(cursor.read_exact(1)?);
            i64::from(i8::from_le_bytes(buf))
        }
        IntegerWidth::Bits16 => {
            let mut buf = [0u8; 2];
            buf.copy_from_slice(cursor.read_exact(2)?);
            i64::from(i16::from_le_bytes(buf))
        }
        IntegerWidth::Bits32 => {
            let mut buf = [0u8; 4];
            buf.copy_from_slice(cursor.read_exact(4)?);
            i64::from(i32::from_le_bytes(buf))
        }
        IntegerWidth::Bits64 => {
            let mut buf = [0u8; 8];
            buf.copy_from_slice(cursor.read_exact(8)?);
            i64::from_le_bytes(buf)
        }
    })
}

fn write_bytes(out: &mut Vec<u8>, bytes: &[u8]) -> Result<()> {
    let len = u32::try_from(bytes.len()).map_err(|_| Error::InvalidInput {
        message: "value exceeds u32 length".into(),
    })?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

fn read_bytes(cursor: &mut Cursor<'_>) -> Result<Vec<u8>> {
    let len = cursor.read_u32()? as usize;
    Ok(cursor.read_exact(len)?.to_vec())
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.remaining() < len {
            return Err(Error::InvalidInput {
                message: format!(
                    "unexpected end of PLAIN column (need {len}, have {})",
                    self.remaining()
                ),
            });
        }
        let start = self.pos;
        self.pos += len;
        Ok(&self.data[start..self.pos])
    }

    fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_exact(1)?[0])
    }

    fn read_u32(&mut self) -> Result<u32> {
        let mut buf = [0u8; 4];
        buf.copy_from_slice(self.read_exact(4)?);
        Ok(u32::from_le_bytes(buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_core::IntegerWidth;

    #[test]
    fn plain_roundtrip_with_nulls() {
        let enc = PlainEncoder;
        let ty = LogicalType::unsigned_integer(IntegerWidth::Bits64);
        let values = vec![
            FieldValue::UnsignedInteger(1),
            FieldValue::Null,
            FieldValue::UnsignedInteger(u64::MAX),
        ];
        let bytes = enc.encode(&ty, &values).unwrap();
        let decoded = enc.decode(&ty, 3, &bytes).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn plain_roundtrip_strings() {
        let enc = PlainEncoder;
        let ty = LogicalType::categorical_string(None);
        let values = vec![
            FieldValue::CategoricalString("AAPL".into()),
            FieldValue::Null,
            FieldValue::CategoricalString("".into()),
        ];
        let bytes = enc.encode(&ty, &values).unwrap();
        assert_eq!(enc.decode(&ty, 3, &bytes).unwrap(), values);
    }
}
