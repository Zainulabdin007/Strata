//! Logical types supported by Strata.

use serde::{Deserialize, Serialize};

/// Bit width of an integer logical type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegerWidth {
    /// 8-bit integer.
    Bits8,
    /// 16-bit integer.
    Bits16,
    /// 32-bit integer.
    Bits32,
    /// 64-bit integer.
    Bits64,
}

impl IntegerWidth {
    /// Returns the number of bits represented by this width.
    pub const fn bits(self) -> u8 {
        match self {
            Self::Bits8 => 8,
            Self::Bits16 => 16,
            Self::Bits32 => 32,
            Self::Bits64 => 64,
        }
    }

    /// Returns the number of bytes represented by this width.
    pub const fn bytes(self) -> u8 {
        self.bits() / 8
    }
}

/// Time unit for [`TimestampType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampUnit {
    /// Whole seconds since the epoch.
    Seconds,
    /// Milliseconds since the epoch.
    Milliseconds,
    /// Microseconds since the epoch.
    Microseconds,
    /// Nanoseconds since the epoch.
    Nanoseconds,
}

/// Boolean logical type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BooleanType;

/// Unsigned integer logical type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UnsignedIntegerType {
    /// Storage width of the integer.
    pub width: IntegerWidth,
}

/// Signed integer logical type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SignedIntegerType {
    /// Storage width of the integer.
    pub width: IntegerWidth,
}

/// Timestamp logical type, stored as an integer offset from an epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimestampType {
    /// Resolution of the timestamp value.
    pub unit: TimestampUnit,
}

/// Fixed-point decimal logical type.
///
/// Values are stored as scaled integers: `logical = scaled / 10^scale`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FixedDecimalType {
    /// Total significant digits.
    pub precision: u8,
    /// Digits after the decimal point.
    pub scale: u8,
    /// Width of the underlying scaled integer.
    pub width: IntegerWidth,
}

/// Low-cardinality string logical type.
///
/// Intended for dictionary / categorical encodings in later phases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CategoricalStringType {
    /// Optional maximum byte length of a single value.
    pub max_bytes: Option<u32>,
}

/// Closed-set enumeration logical type.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnumType {
    /// Ordered list of allowed variant names.
    pub variants: Vec<String>,
}

/// Opaque binary logical type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BinaryType {
    /// Optional maximum byte length of a single value.
    pub max_bytes: Option<u32>,
}

/// Strata logical type system (initial set).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogicalType {
    /// True / false.
    Boolean(BooleanType),
    /// Unsigned integer of a fixed width.
    UnsignedInteger(UnsignedIntegerType),
    /// Signed integer of a fixed width.
    SignedInteger(SignedIntegerType),
    /// Timestamp with a declared unit.
    Timestamp(TimestampType),
    /// Fixed-point decimal.
    FixedDecimal(FixedDecimalType),
    /// Low-cardinality string.
    CategoricalString(CategoricalStringType),
    /// Closed enumeration.
    Enum(EnumType),
    /// Opaque binary blob.
    Binary(BinaryType),
}

impl LogicalType {
    /// Convenience constructor for a boolean type.
    pub const fn boolean() -> Self {
        Self::Boolean(BooleanType)
    }

    /// Convenience constructor for an unsigned integer type.
    pub const fn unsigned_integer(width: IntegerWidth) -> Self {
        Self::UnsignedInteger(UnsignedIntegerType { width })
    }

    /// Convenience constructor for a signed integer type.
    pub const fn signed_integer(width: IntegerWidth) -> Self {
        Self::SignedInteger(SignedIntegerType { width })
    }

    /// Convenience constructor for a timestamp type.
    pub const fn timestamp(unit: TimestampUnit) -> Self {
        Self::Timestamp(TimestampType { unit })
    }

    /// Convenience constructor for a fixed decimal type.
    pub const fn fixed_decimal(precision: u8, scale: u8, width: IntegerWidth) -> Self {
        Self::FixedDecimal(FixedDecimalType {
            precision,
            scale,
            width,
        })
    }

    /// Convenience constructor for a categorical string type.
    pub const fn categorical_string(max_bytes: Option<u32>) -> Self {
        Self::CategoricalString(CategoricalStringType { max_bytes })
    }

    /// Convenience constructor for an enum type.
    pub fn enum_type(variants: Vec<String>) -> Self {
        Self::Enum(EnumType { variants })
    }

    /// Convenience constructor for a binary type.
    pub const fn binary(max_bytes: Option<u32>) -> Self {
        Self::Binary(BinaryType { max_bytes })
    }

    /// Short stable name for diagnostics and metrics labels.
    pub fn name(&self) -> &'static str {
        match self {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_width_bits() {
        assert_eq!(IntegerWidth::Bits8.bits(), 8);
        assert_eq!(IntegerWidth::Bits64.bytes(), 8);
    }

    #[test]
    fn logical_type_names() {
        assert_eq!(LogicalType::boolean().name(), "boolean");
        assert_eq!(
            LogicalType::unsigned_integer(IntegerWidth::Bits32).name(),
            "unsigned_integer"
        );
    }
}
