//! Append-only updates, tombstones, and latest-version resolution.
//!
//! Sealed blocks are never rewritten in place. Updates append a new version;
//! deletes append a tombstone (`__deleted = true`). Compaction folds history
//! into new sealed segments.

use strata_core::{Field, FieldValue, LogicalType, Record, Schema};

use crate::error::{Error, Result};

/// Reserved boolean column marking tombstones on keyed datasets.
pub const TOMBSTONE_FIELD: &str = "__deleted";

/// Ordered key extracted from a record.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordKey(pub Vec<KeyPart>);

/// One component of a [`RecordKey`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum KeyPart {
    /// Null key component (nullable key fields).
    Null,
    /// Boolean.
    Bool(bool),
    /// Unsigned integer.
    U64(u64),
    /// Signed / timestamp / fixed-decimal.
    I64(i64),
    /// String (categorical / enum).
    Str(String),
    /// Binary.
    Bytes(Vec<u8>),
}

impl KeyPart {
    /// Converts a field value into a key part.
    pub fn from_value(v: &FieldValue) -> Result<Self> {
        Ok(match v {
            FieldValue::Null => Self::Null,
            FieldValue::Boolean(b) => Self::Bool(*b),
            FieldValue::UnsignedInteger(n) => Self::U64(*n),
            FieldValue::SignedInteger(n)
            | FieldValue::Timestamp(n)
            | FieldValue::FixedDecimal(n) => Self::I64(*n),
            FieldValue::CategoricalString(s) | FieldValue::Enum(s) => Self::Str(s.clone()),
            FieldValue::Binary(b) => Self::Bytes(b.clone()),
        })
    }

    /// Display for indexes / diagnostics.
    pub fn display(&self) -> String {
        match self {
            Self::Null => "null".into(),
            Self::Bool(b) => b.to_string(),
            Self::U64(n) => n.to_string(),
            Self::I64(n) => n.to_string(),
            Self::Str(s) => s.clone(),
            Self::Bytes(b) => format!("<{} bytes>", b.len()),
        }
    }
}

impl RecordKey {
    /// Builds a key from a record using `key_fields` order.
    pub fn from_record(record: &Record, key_fields: &[String]) -> Result<Self> {
        if key_fields.is_empty() {
            return Err(Error::InvalidState {
                message: "cannot extract key: dataset has no key_fields".into(),
            });
        }
        let mut parts = Vec::with_capacity(key_fields.len());
        for name in key_fields {
            let v = record.get(name).ok_or_else(|| Error::InvalidState {
                message: format!("record missing key field `{name}`"),
            })?;
            parts.push(KeyPart::from_value(v)?);
        }
        Ok(Self(parts))
    }

    /// Stable display string for JSON indexes.
    pub fn display_parts(&self) -> Vec<String> {
        self.0.iter().map(KeyPart::display).collect()
    }
}

/// Returns true when the record is a tombstone.
pub fn is_tombstone(record: &Record) -> bool {
    matches!(record.get(TOMBSTONE_FIELD), Some(FieldValue::Boolean(true)))
}

/// Ensures the schema contains the reserved tombstone boolean field.
pub fn ensure_tombstone_field(schema: Schema) -> Result<Schema> {
    if schema.field(TOMBSTONE_FIELD).is_some() {
        return Ok(schema);
    }
    let mut fields = schema.fields.clone();
    fields.push(Field::required(TOMBSTONE_FIELD, LogicalType::boolean()));
    Schema::new(schema.id.clone(), fields).map_err(|e| Error::InvalidState {
        message: format!("failed to add tombstone field: {e}"),
    })
}

/// Marks a record as a live version (`__deleted = false`).
pub fn mark_live(mut record: Record) -> Record {
    record.insert(TOMBSTONE_FIELD, FieldValue::Boolean(false));
    record
}

/// Builds a tombstone record for `key_fields` from a key-bearing record.
pub fn make_tombstone(
    key_record: &Record,
    key_fields: &[String],
    schema: &Schema,
) -> Result<Record> {
    let mut out = Record::new();
    for field in &schema.fields {
        if key_fields.iter().any(|k| k == &field.name) {
            let v = key_record
                .get(&field.name)
                .ok_or_else(|| Error::InvalidState {
                    message: format!("tombstone missing key field `{}`", field.name),
                })?;
            out.insert(field.name.clone(), v.clone());
        } else if field.name == TOMBSTONE_FIELD {
            out.insert(TOMBSTONE_FIELD, FieldValue::Boolean(true));
        } else if field.nullable {
            out.insert(field.name.clone(), FieldValue::Null);
        } else {
            // Non-null non-key fields: use type-appropriate zero defaults.
            out.insert(field.name.clone(), default_value(&field.logical_type));
        }
    }
    Ok(out)
}

fn default_value(ty: &LogicalType) -> FieldValue {
    match ty {
        LogicalType::Boolean(_) => FieldValue::Boolean(false),
        LogicalType::UnsignedInteger(_) => FieldValue::UnsignedInteger(0),
        LogicalType::SignedInteger(_) => FieldValue::SignedInteger(0),
        LogicalType::Timestamp(_) => FieldValue::Timestamp(0),
        LogicalType::FixedDecimal(_) => FieldValue::FixedDecimal(0),
        LogicalType::CategoricalString(_) => FieldValue::CategoricalString(String::new()),
        LogicalType::Enum(e) => FieldValue::Enum(e.variants.first().cloned().unwrap_or_default()),
        LogicalType::Binary(_) => FieldValue::Binary(Vec::new()),
    }
}

/// Resolves append-only history into the latest live row per key.
///
/// Later rows win. Tombstones remove the key from the logical state.
/// Append-only (empty `key_fields`) returns all records unchanged.
pub fn resolve_latest(key_fields: &[String], records: &[Record]) -> Result<Vec<Record>> {
    if key_fields.is_empty() {
        return Ok(records.to_vec());
    }
    let mut latest: std::collections::BTreeMap<RecordKey, Record> =
        std::collections::BTreeMap::new();
    for rec in records {
        let key = RecordKey::from_record(rec, key_fields)?;
        if is_tombstone(rec) {
            latest.remove(&key);
        } else {
            latest.insert(key, mark_live(rec.clone()));
        }
    }
    Ok(latest.into_values().collect())
}

/// Validates that `key_fields` exist on the schema.
pub fn validate_key_fields(schema: &Schema, key_fields: &[String]) -> Result<()> {
    for name in key_fields {
        if schema.field(name).is_none() {
            return Err(Error::InvalidState {
                message: format!("key field `{name}` is not in schema"),
            });
        }
        if name == TOMBSTONE_FIELD {
            return Err(Error::InvalidState {
                message: "tombstone field cannot be a key field".into(),
            });
        }
    }
    Ok(())
}
