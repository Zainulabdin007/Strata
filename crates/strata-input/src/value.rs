//! Coerce textual / JSON values into [`FieldValue`]s.

use strata_core::{FieldValue, IntegerWidth, LogicalType, TimestampUnit};

use crate::error::Error;

/// Parses a missing/empty token into null or an error depending on nullability.
pub fn parse_optional_text(
    column: &str,
    text: Option<&str>,
    logical_type: &LogicalType,
    nullable: bool,
    path: &std::path::Path,
    line: u64,
) -> Result<FieldValue, Error> {
    match text.map(str::trim) {
        None | Some("") => {
            if nullable {
                Ok(FieldValue::Null)
            } else {
                Err(Error::MissingColumn {
                    path: path.to_path_buf(),
                    line,
                    column: column.to_string(),
                })
            }
        }
        Some(raw) => parse_text(column, raw, logical_type, path, line),
    }
}

/// Parses a non-empty textual cell into a field value.
pub fn parse_text(
    column: &str,
    raw: &str,
    logical_type: &LogicalType,
    path: &std::path::Path,
    line: u64,
) -> Result<FieldValue, Error> {
    let invalid = |message: String| Error::InvalidValue {
        path: path.to_path_buf(),
        line,
        column: column.to_string(),
        message,
    };

    match logical_type {
        LogicalType::Boolean(_) => {
            let v = match raw.to_ascii_lowercase().as_str() {
                "true" | "t" | "1" | "yes" => true,
                "false" | "f" | "0" | "no" => false,
                _ => return Err(invalid(format!("expected boolean, got `{raw}`"))),
            };
            Ok(FieldValue::Boolean(v))
        }
        LogicalType::UnsignedInteger(t) => {
            let v: u64 = raw
                .parse()
                .map_err(|_| invalid(format!("expected unsigned integer, got `{raw}`")))?;
            check_unsigned_width(v, t.width).map_err(&invalid)?;
            Ok(FieldValue::UnsignedInteger(v))
        }
        LogicalType::SignedInteger(t) => {
            let v: i64 = raw
                .parse()
                .map_err(|_| invalid(format!("expected signed integer, got `{raw}`")))?;
            check_signed_width(v, t.width).map_err(&invalid)?;
            Ok(FieldValue::SignedInteger(v))
        }
        LogicalType::Timestamp(t) => {
            let v: i64 = raw.parse().map_err(|_| {
                invalid(format!(
                    "expected timestamp integer ({}), got `{raw}`",
                    unit_name(t.unit)
                ))
            })?;
            Ok(FieldValue::Timestamp(v))
        }
        LogicalType::FixedDecimal(t) => {
            let scaled = parse_fixed_decimal(raw, t.scale).map_err(&invalid)?;
            check_signed_width(scaled, t.width).map_err(&invalid)?;
            Ok(FieldValue::FixedDecimal(scaled))
        }
        LogicalType::CategoricalString(t) => {
            if let Some(max) = t.max_bytes {
                if raw.len() as u32 > max {
                    return Err(invalid(format!(
                        "string length {} exceeds max_bytes {max}",
                        raw.len()
                    )));
                }
            }
            Ok(FieldValue::CategoricalString(raw.to_string()))
        }
        LogicalType::Enum(t) => {
            if !t.variants.iter().any(|v| v == raw) {
                return Err(invalid(format!(
                    "invalid enum variant `{raw}`; expected one of: {}",
                    t.variants.join(", ")
                )));
            }
            Ok(FieldValue::Enum(raw.to_string()))
        }
        LogicalType::Binary(t) => {
            let bytes = parse_binary(raw).map_err(&invalid)?;
            if let Some(max) = t.max_bytes {
                if bytes.len() as u32 > max {
                    return Err(invalid(format!(
                        "binary length {} exceeds max_bytes {max}",
                        bytes.len()
                    )));
                }
            }
            Ok(FieldValue::Binary(bytes))
        }
    }
}

/// Parses a JSON value into a field value.
pub fn parse_json_value(
    column: &str,
    value: &serde_json::Value,
    logical_type: &LogicalType,
    nullable: bool,
    path: &std::path::Path,
    line: u64,
) -> Result<FieldValue, Error> {
    if value.is_null() {
        if nullable {
            return Ok(FieldValue::Null);
        }
        return Err(Error::MissingColumn {
            path: path.to_path_buf(),
            line,
            column: column.to_string(),
        });
    }

    // Accept JSON strings for all types; numbers for numeric types.
    match (logical_type, value) {
        (_, serde_json::Value::String(s)) => parse_text(column, s, logical_type, path, line),
        (LogicalType::Boolean(_), serde_json::Value::Bool(b)) => Ok(FieldValue::Boolean(*b)),
        (LogicalType::UnsignedInteger(t), serde_json::Value::Number(n)) => {
            let v = n.as_u64().ok_or_else(|| Error::InvalidValue {
                path: path.to_path_buf(),
                line,
                column: column.to_string(),
                message: format!("expected unsigned integer, got {value}"),
            })?;
            check_unsigned_width(v, t.width).map_err(|message| Error::InvalidValue {
                path: path.to_path_buf(),
                line,
                column: column.to_string(),
                message,
            })?;
            Ok(FieldValue::UnsignedInteger(v))
        }
        (LogicalType::SignedInteger(t), serde_json::Value::Number(n)) => {
            let v = n.as_i64().ok_or_else(|| Error::InvalidValue {
                path: path.to_path_buf(),
                line,
                column: column.to_string(),
                message: format!("expected signed integer, got {value}"),
            })?;
            check_signed_width(v, t.width).map_err(|message| Error::InvalidValue {
                path: path.to_path_buf(),
                line,
                column: column.to_string(),
                message,
            })?;
            Ok(FieldValue::SignedInteger(v))
        }
        (LogicalType::Timestamp(_), serde_json::Value::Number(n)) => {
            let v = n.as_i64().ok_or_else(|| Error::InvalidValue {
                path: path.to_path_buf(),
                line,
                column: column.to_string(),
                message: format!("expected timestamp integer, got {value}"),
            })?;
            Ok(FieldValue::Timestamp(v))
        }
        (LogicalType::FixedDecimal(t), serde_json::Value::Number(n)) => {
            if n.as_i64().is_none() {
                return parse_text(column, &value.to_string(), logical_type, path, line);
            }
            let v = n.as_i64().unwrap();
            check_signed_width(v, t.width).map_err(|message| Error::InvalidValue {
                path: path.to_path_buf(),
                line,
                column: column.to_string(),
                message,
            })?;
            Ok(FieldValue::FixedDecimal(v))
        }
        _ => parse_text(column, &value.to_string(), logical_type, path, line),
    }
}

fn unit_name(unit: TimestampUnit) -> &'static str {
    match unit {
        TimestampUnit::Seconds => "seconds",
        TimestampUnit::Milliseconds => "milliseconds",
        TimestampUnit::Microseconds => "microseconds",
        TimestampUnit::Nanoseconds => "nanoseconds",
    }
}

fn check_unsigned_width(v: u64, width: IntegerWidth) -> Result<(), String> {
    let max = match width {
        IntegerWidth::Bits8 => u8::MAX as u64,
        IntegerWidth::Bits16 => u16::MAX as u64,
        IntegerWidth::Bits32 => u32::MAX as u64,
        IntegerWidth::Bits64 => u64::MAX,
    };
    if v > max {
        Err(format!("value {v} does not fit in {:?}", width))
    } else {
        Ok(())
    }
}

fn check_signed_width(v: i64, width: IntegerWidth) -> Result<(), String> {
    let (min, max) = match width {
        IntegerWidth::Bits8 => (i8::MIN as i64, i8::MAX as i64),
        IntegerWidth::Bits16 => (i16::MIN as i64, i16::MAX as i64),
        IntegerWidth::Bits32 => (i32::MIN as i64, i32::MAX as i64),
        IntegerWidth::Bits64 => (i64::MIN, i64::MAX),
    };
    if v < min || v > max {
        Err(format!("value {v} does not fit in {:?}", width))
    } else {
        Ok(())
    }
}

fn parse_fixed_decimal(raw: &str, scale: u8) -> Result<i64, String> {
    if !raw.contains('.') {
        return raw
            .parse::<i64>()
            .map_err(|_| format!("expected fixed decimal, got `{raw}`"));
    }

    let negative = raw.starts_with('-');
    let s = raw.trim_start_matches('+').trim_start_matches('-');
    let (whole, frac) = match s.split_once('.') {
        Some(parts) => parts,
        None => (s, ""),
    };
    if whole.is_empty() || !whole.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("expected fixed decimal, got `{raw}`"));
    }
    if !frac.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("expected fixed decimal, got `{raw}`"));
    }
    if frac.len() > scale as usize {
        return Err(format!(
            "fractional digits {} exceed scale {scale}",
            frac.len()
        ));
    }

    let mut digits = String::from(whole);
    digits.push_str(frac);
    for _ in 0..(scale as usize - frac.len()) {
        digits.push('0');
    }

    let mut scaled: i64 = digits
        .parse()
        .map_err(|_| format!("fixed decimal `{raw}` out of range"))?;
    if negative {
        scaled = -scaled;
    }
    Ok(scaled)
}

fn parse_binary(raw: &str) -> Result<Vec<u8>, String> {
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        if hex.len() % 2 != 0 {
            return Err("hex binary value must have even length".into());
        }
        (0..hex.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&hex[i..i + 2], 16)
                    .map_err(|_| format!("invalid hex byte in `{raw}`"))
            })
            .collect()
    } else {
        Ok(raw.as_bytes().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use strata_core::{IntegerWidth, LogicalType};

    #[test]
    fn parses_fixed_decimal_with_scale() {
        let v = parse_text(
            "price",
            "190.25",
            &LogicalType::fixed_decimal(18, 2, IntegerWidth::Bits64),
            Path::new("t.csv"),
            1,
        )
        .unwrap();
        assert_eq!(v, FieldValue::FixedDecimal(19025));
    }
}
