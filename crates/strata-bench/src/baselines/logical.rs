//! Raw logical payload size (format-independent).

use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema};

/// Bytes contributed by one value under a field's logical type.
///
/// Definition (methodology): width of the *decoded* payload only — no keys,
/// separators, dictionaries, headers, or compression. Strings/enums/binary use
/// UTF-8 / byte length; fixed-width numerics use schema integer width.
pub fn logical_bytes_of_value(field: &Field, value: &FieldValue) -> u64 {
    match (&field.logical_type, value) {
        (_, FieldValue::Null) => 0,
        (LogicalType::Boolean(_), FieldValue::Boolean(_)) => 1,
        (LogicalType::UnsignedInteger(t), FieldValue::UnsignedInteger(_)) => {
            u64::from(t.width.bytes())
        }
        (LogicalType::SignedInteger(t), FieldValue::SignedInteger(_)) => u64::from(t.width.bytes()),
        (LogicalType::Timestamp(_), FieldValue::Timestamp(_)) => {
            u64::from(IntegerWidth::Bits64.bytes())
        }
        (LogicalType::FixedDecimal(t), FieldValue::FixedDecimal(_)) => u64::from(t.width.bytes()),
        (LogicalType::CategoricalString(_), FieldValue::CategoricalString(s)) => s.len() as u64,
        (LogicalType::Enum(_), FieldValue::Enum(s)) => s.len() as u64,
        (LogicalType::Binary(_), FieldValue::Binary(b)) => b.len() as u64,
        // Mismatched / unexpected: best-effort by value kind.
        (_, FieldValue::Boolean(_)) => 1,
        (_, FieldValue::UnsignedInteger(_))
        | (_, FieldValue::SignedInteger(_))
        | (_, FieldValue::Timestamp(_))
        | (_, FieldValue::FixedDecimal(_)) => 8,
        (_, FieldValue::CategoricalString(s)) | (_, FieldValue::Enum(s)) => s.len() as u64,
        (_, FieldValue::Binary(b)) => b.len() as u64,
    }
}

/// Sum of logical bytes for one record against `schema` field order.
pub fn logical_bytes_of_record(schema: &Schema, record: &Record) -> u64 {
    let mut n = 0u64;
    for field in &schema.fields {
        if let Some(v) = record.get(&field.name) {
            n += logical_bytes_of_value(field, v);
        }
    }
    n
}
