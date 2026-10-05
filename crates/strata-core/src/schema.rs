//! Schema definitions: fields and ordered field lists.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::types::LogicalType;

/// Opaque schema identifier used in segment metadata.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SchemaId(pub String);

impl SchemaId {
    /// Creates a new schema id from any string-like value.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Returns the underlying string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SchemaId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SchemaId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// A named field in a schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    /// Field name (unique within a schema).
    pub name: String,
    /// Logical type of the field.
    pub logical_type: LogicalType,
    /// Whether null values are permitted.
    pub nullable: bool,
}

impl Field {
    /// Creates a required (non-null) field.
    pub fn required(name: impl Into<String>, logical_type: LogicalType) -> Self {
        Self {
            name: name.into(),
            logical_type,
            nullable: false,
        }
    }

    /// Creates a nullable field.
    pub fn nullable(name: impl Into<String>, logical_type: LogicalType) -> Self {
        Self {
            name: name.into(),
            logical_type,
            nullable: true,
        }
    }
}

/// Ordered collection of fields describing a record shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Schema {
    /// Stable identifier for this schema version.
    pub id: SchemaId,
    /// Optional human-readable description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Fields in declaration order.
    pub fields: Vec<Field>,
}

impl Schema {
    /// Creates a schema from an id and field list.
    ///
    /// # Errors
    ///
    /// Returns [`Error::DuplicateField`] if two fields share a name, or
    /// [`Error::EmptySchema`] if `fields` is empty.
    pub fn new(id: impl Into<SchemaId>, fields: Vec<Field>) -> Result<Self> {
        let schema = Self {
            id: id.into(),
            description: None,
            fields,
        };
        schema.validate()?;
        Ok(schema)
    }

    /// Attaches a description and returns the modified schema.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Validates uniqueness and non-emptiness constraints.
    pub fn validate(&self) -> Result<()> {
        if self.fields.is_empty() {
            return Err(Error::EmptySchema);
        }

        let mut seen = IndexMap::new();
        for field in &self.fields {
            if seen.insert(field.name.clone(), ()).is_some() {
                return Err(Error::DuplicateField {
                    name: field.name.clone(),
                });
            }
            if let LogicalType::Enum(enum_ty) = &field.logical_type {
                if enum_ty.variants.is_empty() {
                    return Err(Error::EmptyEnum {
                        field: field.name.clone(),
                    });
                }
            }
            if let LogicalType::FixedDecimal(dec) = &field.logical_type {
                if dec.scale > dec.precision {
                    return Err(Error::InvalidFixedDecimal {
                        field: field.name.clone(),
                        precision: dec.precision,
                        scale: dec.scale,
                    });
                }
            }
        }
        Ok(())
    }

    /// Returns the number of fields.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Returns true when the schema has no fields (should not occur after `new`).
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Looks up a field by name.
    pub fn field(&self, name: &str) -> Option<&Field> {
        self.fields.iter().find(|f| f.name == name)
    }

    /// Returns field names in declaration order.
    pub fn field_names(&self) -> impl Iterator<Item = &str> {
        self.fields.iter().map(|f| f.name.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{IntegerWidth, LogicalType};

    #[test]
    fn rejects_duplicate_fields() {
        let err = Schema::new(
            "events",
            vec![
                Field::required("id", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
                Field::required("id", LogicalType::boolean()),
            ],
        )
        .unwrap_err();
        assert!(matches!(err, Error::DuplicateField { .. }));
    }

    #[test]
    fn accepts_valid_schema() {
        let schema = Schema::new(
            "events",
            vec![
                Field::required("id", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
                Field::nullable("flag", LogicalType::boolean()),
            ],
        )
        .unwrap();
        assert_eq!(schema.len(), 2);
        assert!(schema.field("flag").unwrap().nullable);
    }
}
