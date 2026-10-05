//! Row-level predicate evaluation.

use strata_core::{FieldValue, LogicalType, Record, Schema};

use crate::error::{Error, Result};
use crate::plan::{Literal, Predicate};

/// Returns true when `record` satisfies all predicates (AND).
pub fn record_matches(record: &Record, predicates: &[Predicate]) -> bool {
    predicates.iter().all(|p| eval_one(record, p))
}

fn eval_one(record: &Record, pred: &Predicate) -> bool {
    value_matches(record.get(pred.field()), pred)
}

pub(crate) fn value_matches(v: Option<&FieldValue>, pred: &Predicate) -> bool {
    match pred {
        Predicate::Eq { value, .. } => cmp_eq(v, value),
        Predicate::Ne { value, .. } => !cmp_eq(v, value),
        Predicate::Lt { value, .. } => cmp_ord(v, value).is_some_and(|o| o < 0),
        Predicate::Le { value, .. } => cmp_ord(v, value).is_some_and(|o| o <= 0),
        Predicate::Gt { value, .. } => cmp_ord(v, value).is_some_and(|o| o > 0),
        Predicate::Ge { value, .. } => cmp_ord(v, value).is_some_and(|o| o >= 0),
        Predicate::Between { low, high, .. } => {
            cmp_ord(v, low).is_some_and(|o| o >= 0) && cmp_ord(v, high).is_some_and(|o| o <= 0)
        }
        Predicate::In { values, .. } => values.iter().any(|lit| cmp_eq(v, lit)),
    }
}

fn cmp_eq(value: Option<&FieldValue>, lit: &Literal) -> bool {
    match (value, lit) {
        (None, _) => false,
        (Some(FieldValue::Null), Literal::Null) => true,
        (Some(FieldValue::Boolean(b)), Literal::Bool(x)) => b == x,
        (Some(FieldValue::UnsignedInteger(n)), Literal::U64(x)) => n == x,
        (Some(FieldValue::UnsignedInteger(n)), Literal::I64(x)) if *x >= 0 => *n == *x as u64,
        (Some(FieldValue::SignedInteger(n)), Literal::I64(x)) => n == x,
        (Some(FieldValue::SignedInteger(n)), Literal::U64(x)) => {
            *n == *x as i64 && *x <= i64::MAX as u64
        }
        (Some(FieldValue::Timestamp(n)), Literal::I64(x)) => n == x,
        (Some(FieldValue::Timestamp(n)), Literal::U64(x)) => {
            *n == *x as i64 && *x <= i64::MAX as u64
        }
        (Some(FieldValue::FixedDecimal(n)), Literal::I64(x)) => n == x,
        (Some(FieldValue::FixedDecimal(n)), Literal::U64(x)) => {
            *n == *x as i64 && *x <= i64::MAX as u64
        }
        (Some(FieldValue::CategoricalString(s)), Literal::String(x)) => s == x,
        (Some(FieldValue::Enum(s)), Literal::String(x)) => s == x,
        _ => false,
    }
}

/// Returns `Some(cmp)` where cmp is like `value.cmp(lit)` when comparable.
fn cmp_ord(value: Option<&FieldValue>, lit: &Literal) -> Option<i8> {
    let value = value?;
    if value.is_null() || matches!(lit, Literal::Null) {
        return None;
    }
    match (value, lit) {
        (FieldValue::UnsignedInteger(n), Literal::U64(x)) => Some(ord_u64(*n, *x)),
        (FieldValue::UnsignedInteger(n), Literal::I64(x)) if *x >= 0 => {
            Some(ord_u64(*n, *x as u64))
        }
        (FieldValue::SignedInteger(n), Literal::I64(x)) => Some(ord_i64(*n, *x)),
        (FieldValue::SignedInteger(n), Literal::U64(x)) if *x <= i64::MAX as u64 => {
            Some(ord_i64(*n, *x as i64))
        }
        (FieldValue::Timestamp(n), Literal::I64(x)) => Some(ord_i64(*n, *x)),
        (FieldValue::Timestamp(n), Literal::U64(x)) if *x <= i64::MAX as u64 => {
            Some(ord_i64(*n, *x as i64))
        }
        (FieldValue::FixedDecimal(n), Literal::I64(x)) => Some(ord_i64(*n, *x)),
        (FieldValue::FixedDecimal(n), Literal::U64(x)) if *x <= i64::MAX as u64 => {
            Some(ord_i64(*n, *x as i64))
        }
        (FieldValue::CategoricalString(s), Literal::String(x)) => Some(ord_str(s, x)),
        (FieldValue::Enum(s), Literal::String(x)) => Some(ord_str(s, x)),
        _ => None,
    }
}

fn ord_u64(a: u64, b: u64) -> i8 {
    match a.cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn ord_i64(a: i64, b: i64) -> i8 {
    match a.cmp(&b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn ord_str(a: &str, b: &str) -> i8 {
    match a.cmp(b) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// Validates that query fields/literals are compatible with `schema`.
pub fn validate_against_schema(
    select: &[String],
    predicates: &[Predicate],
    schema: &Schema,
) -> Result<Vec<String>> {
    let select_cols = if select.is_empty() {
        schema.field_names().map(|s| s.to_string()).collect()
    } else {
        for name in select {
            if schema.field(name).is_none() {
                return Err(Error::invalid(format!(
                    "select column `{name}` is not in schema"
                )));
            }
        }
        select.to_vec()
    };

    for pred in predicates {
        let field = schema.field(pred.field()).ok_or_else(|| {
            Error::invalid(format!(
                "predicate field `{}` is not in schema",
                pred.field()
            ))
        })?;
        match pred {
            Predicate::Eq { value, .. }
            | Predicate::Ne { value, .. }
            | Predicate::Lt { value, .. }
            | Predicate::Le { value, .. }
            | Predicate::Gt { value, .. }
            | Predicate::Ge { value, .. } => {
                check_literal(&field.logical_type, value)?;
            }
            Predicate::Between { low, high, .. } => {
                check_literal(&field.logical_type, low)?;
                check_literal(&field.logical_type, high)?;
            }
            Predicate::In { values, .. } => {
                for v in values {
                    check_literal(&field.logical_type, v)?;
                }
            }
        }
    }
    Ok(select_cols)
}

fn check_literal(ty: &LogicalType, lit: &Literal) -> Result<()> {
    let ok = match (ty, lit) {
        (_, Literal::Null) => true,
        (LogicalType::Boolean(_), Literal::Bool(_)) => true,
        (LogicalType::UnsignedInteger(_), Literal::U64(_)) => true,
        (LogicalType::UnsignedInteger(_), Literal::I64(n)) if *n >= 0 => true,
        (LogicalType::SignedInteger(_), Literal::I64(_)) => true,
        (LogicalType::SignedInteger(_), Literal::U64(_)) => true,
        (LogicalType::Timestamp(_), Literal::I64(_) | Literal::U64(_)) => true,
        (LogicalType::FixedDecimal(_), Literal::I64(_) | Literal::U64(_)) => true,
        (LogicalType::CategoricalString(_), Literal::String(_)) => true,
        (LogicalType::Enum(_), Literal::String(_)) => true,
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(Error::invalid(format!(
            "literal {} is not valid for type {}",
            lit.display(),
            ty.name()
        )))
    }
}
