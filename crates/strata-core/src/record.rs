//! In-memory records and record batches.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::error::{Error, Result};
use crate::schema::Schema;
use crate::types::LogicalType;

/// A single logical field value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum FieldValue {
    /// Explicit null.
    Null,
    /// Boolean.
    Boolean(bool),
    /// Unsigned integer (up to 64 bits).
    UnsignedInteger(u64),
    /// Signed integer (up to 64 bits).
    SignedInteger(i64),
    /// Timestamp as an integer offset in the field's declared unit.
    Timestamp(i64),
    /// Fixed decimal as a scaled integer.
    FixedDecimal(i64),
    /// Categorical / dictionary string.
    CategoricalString(String),
    /// Enum variant name.
    Enum(String),
    /// Opaque bytes.
    Binary(Vec<u8>),
}

impl FieldValue {
    /// Returns true when this value is null.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// Returns a short type label for diagnostics.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean(_) => "boolean",
            Self::UnsignedInteger(_) => "unsigned_integer",
            Self::SignedInteger(_) => "signed_integer",
            Self::Timestamp(_) => "timestamp",
            Self::FixedDecimal(_) => "fixed_decimal",
            Self::CategoricalString(_) => "categorical_string",
            Self::Enum(_) => "enum",
            Self::Binary(_) => "binary",
        }
    }
}

/// Schema-ordered ordinal slots with shared field names. Serialized as a map for
/// compatibility with the original record JSON representation.
#[derive(Debug, Clone, Default)]
pub struct RecordValues {
    names: Arc<Vec<String>>,
    slots: Vec<FieldValue>,
}
impl RecordValues {
    /// Insert or replace a named value. Adding a field uses copy-on-write names.
    pub fn insert(&mut self, name: String, value: FieldValue) -> Option<FieldValue> {
        if let Some(i) = self.names.iter().position(|n| n == &name) {
            return Some(std::mem::replace(&mut self.slots[i], value));
        }
        Arc::make_mut(&mut self.names).push(name);
        self.slots.push(value);
        None
    }
    /// Lookup for row APIs; columnar consumers use ordinal slots directly.
    pub fn get(&self, name: &str) -> Option<&FieldValue> {
        self.names
            .iter()
            .position(|n| n == name)
            .map(|i| &self.slots[i])
    }
    /// Ordered field names.
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.names.iter()
    }
    /// Number of slots.
    pub fn len(&self) -> usize {
        self.slots.len()
    }
    /// Whether there are no slots.
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}
impl<'a> IntoIterator for &'a RecordValues {
    type Item = (&'a String, &'a FieldValue);
    type IntoIter = std::iter::Zip<std::slice::Iter<'a, String>, std::slice::Iter<'a, FieldValue>>;
    fn into_iter(self) -> Self::IntoIter {
        self.names.iter().zip(&self.slots)
    }
}
impl PartialEq for RecordValues {
    fn eq(&self, other: &Self) -> bool {
        if self.names == other.names {
            return self.slots == other.slots;
        }
        self.len() == other.len()
            && self
                .names
                .iter()
                .zip(&self.slots)
                .all(|(name, value)| other.get(name) == Some(value))
    }
}
impl Serialize for RecordValues {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.len()))?;
        for (name, value) in self.names.iter().zip(&self.slots) {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}
impl<'de> Deserialize<'de> for RecordValues {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<Self, D::Error> {
        let map = IndexMap::<String, FieldValue>::deserialize(deserializer)?;
        let (names, slots) = map.into_iter().unzip();
        Ok(Self {
            names: Arc::new(names),
            slots,
        })
    }
}

/// One logical record: an ordered map of field name → value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Field values keyed by name. Insertion order is preserved.
    pub values: RecordValues,
}

impl Record {
    /// Creates an empty record.
    pub fn new() -> Self {
        Self {
            values: RecordValues::default(),
        }
    }

    /// Construct ordinal slots sharing a schema layout. The caller supplies unique
    /// names in schema order; normal schema validation still applies on ingest.
    pub fn from_ordered(names: Arc<Vec<String>>, slots: Vec<FieldValue>) -> Self {
        assert_eq!(names.len(), slots.len(), "record layout length mismatch");
        Self {
            values: RecordValues { names, slots },
        }
    }

    /// Value by schema ordinal, without string lookup.
    pub fn get_ordinal(&self, index: usize) -> Option<&FieldValue> {
        self.values.slots.get(index)
    }

    /// Whether this record follows the schema's complete field order.
    pub fn is_schema_ordered(&self, schema: &Schema) -> bool {
        self.len() == schema.len()
            && self
                .values
                .names
                .iter()
                .zip(&schema.fields)
                .all(|(name, field)| name == &field.name)
    }

    /// Inserts a field value, replacing any previous value for the name.
    pub fn insert(&mut self, name: impl Into<String>, value: FieldValue) -> Option<FieldValue> {
        self.values.insert(name.into(), value)
    }

    /// Builder-style insert.
    pub fn with(mut self, name: impl Into<String>, value: FieldValue) -> Self {
        self.insert(name, value);
        self
    }

    /// Returns a field value by name.
    pub fn get(&self, name: &str) -> Option<&FieldValue> {
        self.values.get(name)
    }

    /// Number of fields present in the record (may be less than schema width).
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns true when the record has no fields.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Validates this record against a schema.
    pub fn validate_against(&self, schema: &Schema) -> Result<()> {
        let ordered = self.is_schema_ordered(schema);
        for (i, field) in schema.fields.iter().enumerate() {
            match if ordered {
                self.get_ordinal(i)
            } else {
                self.values.get(&field.name)
            } {
                None => {
                    return Err(Error::MissingField {
                        field: field.name.clone(),
                    });
                }
                Some(FieldValue::Null) if !field.nullable => {
                    return Err(Error::NullNotAllowed {
                        field: field.name.clone(),
                    });
                }
                Some(FieldValue::Null) => {}
                Some(value) => check_type_compatibility(&field.name, &field.logical_type, value)?,
            }
        }

        if ordered {
            return Ok(());
        }
        for name in self.values.keys() {
            if schema.field(name).is_none() {
                return Err(Error::UnknownField {
                    field: name.clone(),
                });
            }
        }

        Ok(())
    }
}

impl Default for Record {
    fn default() -> Self {
        Self::new()
    }
}

/// A batch of records sharing a common schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordBatch {
    /// Schema describing every record in the batch.
    pub schema: Schema,
    /// Records in ingest order.
    pub records: Vec<Record>,
}

impl RecordBatch {
    /// Creates an empty batch for the given schema.
    pub fn empty(schema: Schema) -> Self {
        Self {
            schema,
            records: Vec::new(),
        }
    }

    /// Creates a batch from a schema and records, validating each record.
    pub fn try_new(schema: Schema, records: Vec<Record>) -> Result<Self> {
        let mut batch = Self::empty(schema);
        for record in records {
            batch.push(record)?;
        }
        Ok(batch)
    }

    /// Appends a record after validating it against the batch schema.
    pub fn push(&mut self, record: Record) -> Result<()> {
        record.validate_against(&self.schema)?;
        self.records.push(record);
        Ok(())
    }

    /// Number of records in the batch.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns true when the batch has no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Validate a decoded value against a field, without constructing a row.
pub fn validate_field_value(field: &crate::Field, value: &FieldValue) -> Result<()> {
    if value.is_null() {
        return if field.nullable {
            Ok(())
        } else {
            Err(Error::NullNotAllowed {
                field: field.name.clone(),
            })
        };
    }
    check_type_compatibility(&field.name, &field.logical_type, value)
}

fn check_type_compatibility(field: &str, logical: &LogicalType, value: &FieldValue) -> Result<()> {
    let range_ok = match (logical, value) {
        (LogicalType::UnsignedInteger(t), FieldValue::UnsignedInteger(n)) => {
            let bits = t.width.bytes() * 8;
            bits == 64 || *n < (1u64 << bits)
        }
        (LogicalType::SignedInteger(t), FieldValue::SignedInteger(n)) => {
            let bits = t.width.bytes() * 8;
            bits == 64 || (*n >= -(1i64 << (bits - 1)) && *n < (1i64 << (bits - 1)))
        }
        (LogicalType::FixedDecimal(t), FieldValue::FixedDecimal(n)) => {
            let bits = t.width.bytes() * 8;
            bits == 64 || (*n >= -(1i64 << (bits - 1)) && *n < (1i64 << (bits - 1)))
        }
        (LogicalType::CategoricalString(t), FieldValue::CategoricalString(s)) => {
            t.max_bytes.is_none_or(|max| s.len() <= max as usize)
        }
        (LogicalType::Binary(t), FieldValue::Binary(b)) => {
            t.max_bytes.is_none_or(|max| b.len() <= max as usize)
        }
        _ => true,
    };
    if !range_ok {
        return Err(Error::InvalidValue {
            field: field.to_owned(),
            reason: "exceeds declared integer width or maximum byte length".into(),
        });
    }
    let compatible = match (logical, value) {
        (_, FieldValue::Null) => true,
        (LogicalType::Boolean(_), FieldValue::Boolean(_)) => true,
        (LogicalType::UnsignedInteger(_), FieldValue::UnsignedInteger(_)) => true,
        (LogicalType::SignedInteger(_), FieldValue::SignedInteger(_)) => true,
        (LogicalType::Timestamp(_), FieldValue::Timestamp(_)) => true,
        (LogicalType::FixedDecimal(_), FieldValue::FixedDecimal(_)) => true,
        (LogicalType::CategoricalString(_), FieldValue::CategoricalString(_)) => true,
        (LogicalType::Enum(enum_ty), FieldValue::Enum(variant)) => {
            if enum_ty.variants.iter().any(|v| v == variant) {
                true
            } else {
                return Err(Error::InvalidEnumVariant {
                    field: field.to_string(),
                    variant: variant.clone(),
                });
            }
        }
        (LogicalType::Binary(_), FieldValue::Binary(_)) => true,
        _ => false,
    };

    if compatible {
        Ok(())
    } else {
        Err(Error::TypeMismatch {
            field: field.to_string(),
            expected: logical.name().to_string(),
            actual: value.kind_name().to_string(),
        })
    }
}

/// Logical payload bytes under the declared schema width, excluding format overhead.
pub fn logical_value_bytes(ty: &LogicalType, value: &FieldValue) -> u64 {
    match value {
        FieldValue::Null => 0,
        FieldValue::Boolean(_) => 1,
        FieldValue::UnsignedInteger(_) => match ty {
            LogicalType::UnsignedInteger(t) => u64::from(t.width.bytes()),
            _ => 8,
        },
        FieldValue::SignedInteger(_) => match ty {
            LogicalType::SignedInteger(t) => u64::from(t.width.bytes()),
            _ => 8,
        },
        FieldValue::FixedDecimal(_) => match ty {
            LogicalType::FixedDecimal(t) => u64::from(t.width.bytes()),
            _ => 8,
        },
        FieldValue::Timestamp(_) => 8,
        FieldValue::CategoricalString(s) | FieldValue::Enum(s) => s.len() as u64,
        FieldValue::Binary(b) => b.len() as u64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::Field;
    use crate::types::{IntegerWidth, LogicalType};

    fn sample_schema() -> Schema {
        Schema::new(
            "ticks",
            vec![
                Field::required("seq", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
                Field::nullable("symbol", LogicalType::categorical_string(Some(16))),
            ],
        )
        .unwrap()
    }

    #[test]
    fn validates_matching_record() {
        let schema = sample_schema();
        let record = Record::new()
            .with("seq", FieldValue::UnsignedInteger(1))
            .with("symbol", FieldValue::CategoricalString("AAPL".into()));
        record.validate_against(&schema).unwrap();
    }

    #[test]
    fn rejects_type_mismatch() {
        let schema = sample_schema();
        let record = Record::new()
            .with("seq", FieldValue::Boolean(true))
            .with("symbol", FieldValue::Null);
        let err = record.validate_against(&schema).unwrap_err();
        assert!(matches!(err, Error::TypeMismatch { .. }));
    }

    #[test]
    fn batch_push_validates() {
        let mut batch = RecordBatch::empty(sample_schema());
        batch
            .push(
                Record::new()
                    .with("seq", FieldValue::UnsignedInteger(7))
                    .with("symbol", FieldValue::Null),
            )
            .unwrap();
        assert_eq!(batch.len(), 1);
    }
}

#[cfg(test)]
mod ordinal_tests {
    use super::*;
    #[test]
    fn shared_layout_preserves_map_json_and_copy_on_write() {
        let names = Arc::new(vec!["a".to_owned(), "b".to_owned()]);
        let row = Record::from_ordered(
            names.clone(),
            vec![FieldValue::UnsignedInteger(7), FieldValue::Null],
        );
        let old_style = Record::new()
            .with("a", FieldValue::UnsignedInteger(7))
            .with("b", FieldValue::Null);
        assert_eq!(row, old_style);
        assert_eq!(
            serde_json::to_string(&row).unwrap(),
            serde_json::to_string(&old_style).unwrap()
        );
        let restored: Record = serde_json::from_str(&serde_json::to_string(&row).unwrap()).unwrap();
        assert_eq!(restored, row);
        let mut changed = row.clone();
        changed.insert("c", FieldValue::Boolean(true));
        changed.insert("a", FieldValue::UnsignedInteger(9));
        assert_eq!(row.get_ordinal(0), Some(&FieldValue::UnsignedInteger(7)));
        assert_eq!(names.len(), 2);
        assert_eq!(changed.len(), 3);
        assert_eq!(
            row,
            Record::new()
                .with("b", FieldValue::Null)
                .with("a", FieldValue::UnsignedInteger(7))
        );
    }
}
