//! Per-column profile configuration.

use serde::Deserialize;

use crate::error::{Error, Result};
use crate::schema::Field;
use crate::types::{
    BinaryType, BooleanType, CategoricalStringType, EnumType, FixedDecimalType, IntegerWidth,
    LogicalType, SignedIntegerType, TimestampType, TimestampUnit, UnsignedIntegerType,
};

use super::encoding::ColumnEncodingSpec;
use super::hints::{NumericBound, SemanticHint};

/// One column entry inside a compression profile.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ColumnConfig {
    /// Column / field name.
    pub name: String,
    /// Logical type name (e.g. `unsigned_integer`, `timestamp`).
    pub logical_type: String,
    /// Whether nulls are allowed.
    #[serde(default)]
    pub nullable: bool,
    /// Integer / decimal width when applicable.
    #[serde(default)]
    pub width: Option<IntegerWidth>,
    /// Timestamp unit when `logical_type = "timestamp"`.
    #[serde(default)]
    pub unit: Option<TimestampUnit>,
    /// Fixed-decimal precision.
    #[serde(default)]
    pub precision: Option<u8>,
    /// Fixed-decimal scale.
    #[serde(default)]
    pub scale: Option<u8>,
    /// Optional max bytes for string/binary types.
    #[serde(default)]
    pub max_bytes: Option<u32>,
    /// Enum variants when `logical_type = "enum"`.
    #[serde(default)]
    pub variants: Vec<String>,
    /// Flag semantic hints.
    #[serde(default)]
    pub hints: Vec<SemanticHint>,
    /// Optional expected minimum (semantic hint).
    #[serde(default)]
    pub expected_min: Option<NumericBound>,
    /// Optional expected maximum (semantic hint).
    #[serde(default)]
    pub expected_max: Option<NumericBound>,
    /// Encoding preference: a hint string or an explicit pipeline array.
    #[serde(default)]
    pub encoding: ColumnEncodingSpec,
}

impl ColumnConfig {
    /// Converts this column into a schema [`Field`].
    pub fn to_field(&self) -> Result<Field> {
        let logical_type = self.to_logical_type()?;
        Ok(Field {
            name: self.name.clone(),
            logical_type,
            nullable: self.nullable,
        })
    }

    /// Builds the logical type from the flat TOML fields.
    pub fn to_logical_type(&self) -> Result<LogicalType> {
        let lt = self.logical_type.as_str();
        match lt {
            "boolean" => Ok(LogicalType::Boolean(BooleanType)),
            "unsigned_integer" => Ok(LogicalType::UnsignedInteger(UnsignedIntegerType {
                width: self.require_width()?,
            })),
            "signed_integer" => Ok(LogicalType::SignedInteger(SignedIntegerType {
                width: self.require_width()?,
            })),
            "timestamp" => {
                let unit = self.unit.ok_or_else(|| Error::InvalidColumn {
                    column: self.name.clone(),
                    message: "timestamp columns require `unit` (seconds|milliseconds|microseconds|nanoseconds)"
                        .into(),
                })?;
                Ok(LogicalType::Timestamp(TimestampType { unit }))
            }
            "fixed_decimal" => {
                let precision = self.precision.ok_or_else(|| Error::InvalidColumn {
                    column: self.name.clone(),
                    message: "fixed_decimal columns require `precision`".into(),
                })?;
                let scale = self.scale.ok_or_else(|| Error::InvalidColumn {
                    column: self.name.clone(),
                    message: "fixed_decimal columns require `scale`".into(),
                })?;
                if scale > precision {
                    return Err(Error::InvalidFixedDecimal {
                        field: self.name.clone(),
                        precision,
                        scale,
                    });
                }
                Ok(LogicalType::FixedDecimal(FixedDecimalType {
                    precision,
                    scale,
                    width: self.require_width()?,
                }))
            }
            "categorical_string" => Ok(LogicalType::CategoricalString(CategoricalStringType {
                max_bytes: self.max_bytes,
            })),
            "enum" => {
                if self.variants.is_empty() {
                    return Err(Error::EmptyEnum {
                        field: self.name.clone(),
                    });
                }
                Ok(LogicalType::Enum(EnumType {
                    variants: self.variants.clone(),
                }))
            }
            "binary" => Ok(LogicalType::Binary(BinaryType {
                max_bytes: self.max_bytes,
            })),
            other => Err(Error::InvalidColumn {
                column: self.name.clone(),
                message: format!(
                    "unknown logical type `{other}`; expected one of: boolean, unsigned_integer, \
                     signed_integer, timestamp, fixed_decimal, categorical_string, enum, binary"
                ),
            }),
        }
    }

    fn require_width(&self) -> Result<IntegerWidth> {
        self.width.ok_or_else(|| Error::InvalidColumn {
            column: self.name.clone(),
            message: format!(
                "`{}` columns require `width` (bits8|bits16|bits32|bits64)",
                self.logical_type
            ),
        })
    }

    /// Returns true when the column declares the given flag hint.
    pub fn has_hint(&self, hint: SemanticHint) -> bool {
        self.hints.contains(&hint)
    }
}
