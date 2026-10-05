//! Resolve profile encoding declarations into validated [`EncodingPlan`]s.

use std::collections::HashMap;

use strata_core::{ColumnEncodingSpec, CompressionProfile, LogicalType};

use crate::error::{Error, Result};
use crate::plan::{EncodingPlan, EncodingStep};
use crate::planner::plan_for_field;

/// Intermediate stream kind used for pipeline compatibility checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    I64,
    U64,
    Bytes,
    Bool,
    Encoded,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::I64 => "signed integers",
            Self::U64 => "unsigned integers",
            Self::Bytes => "byte/string values",
            Self::Bool => "booleans",
            Self::Encoded => "encoded bytes",
        }
    }
}

/// Resolves and validates every column encoding in `profile`.
pub fn resolve_profile_plans(
    profile: &CompressionProfile,
) -> Result<HashMap<String, EncodingPlan>> {
    let mut plans = HashMap::with_capacity(profile.columns.len());
    for column in &profile.columns {
        let logical = column.to_logical_type().map_err(|e| Error::InvalidPlan {
            message: format!("column `{}`: {e}", column.name),
        })?;
        let plan = resolve_column_plan(&column.name, &logical, &column.encoding)?;
        plans.insert(column.name.clone(), plan);
    }
    Ok(plans)
}

/// Validates profile encodings without returning the plans.
pub fn validate_profile_encodings(profile: &CompressionProfile) -> Result<()> {
    resolve_profile_plans(profile).map(|_| ())
}

/// Resolves one column's encoding declaration into a validated plan.
pub fn resolve_column_plan(
    column_name: &str,
    logical_type: &LogicalType,
    encoding: &ColumnEncodingSpec,
) -> Result<EncodingPlan> {
    match encoding {
        ColumnEncodingSpec::Hint(hint) => {
            plan_for_field(logical_type, *hint).map_err(|e| Error::InvalidPlan {
                message: format!("column `{column_name}`: {e}"),
            })
        }
        ColumnEncodingSpec::Pipeline(names) => {
            if names.is_empty() {
                return Err(Error::InvalidPlan {
                    message: format!(
                        "column `{column_name}`: encoding pipeline must contain at least one step"
                    ),
                });
            }
            let steps = parse_pipeline_steps(column_name, logical_type, names)?;
            let plan = EncodingPlan::new(steps).map_err(|e| Error::InvalidPlan {
                message: format!("column `{column_name}`: {e}"),
            })?;
            validate_plan_for_type(column_name, logical_type, &plan)?;
            Ok(plan)
        }
    }
}

fn parse_pipeline_steps(
    column_name: &str,
    logical_type: &LogicalType,
    names: &[String],
) -> Result<Vec<EncodingStep>> {
    let mut steps = Vec::with_capacity(names.len());
    for name in names {
        let step = EncodingStep::parse_name(name).ok_or_else(|| Error::InvalidPlan {
            message: format!(
                "column `{column_name}`: unknown encoding step `{name}`; \
                 expected one of: plain, delta, delta_of_delta, zigzag, varint, \
                 frame_of_reference, bitpack, run_length, dictionary, boolean_bitpack, enum_bitpack"
            ),
        })?;
        steps.push(step);
    }

    // Convenience: a lone `bitpack` adapts to the logical type.
    if steps.len() == 1 && steps[0] == EncodingStep::BitPacking {
        steps[0] = match logical_type {
            LogicalType::Boolean(_) => EncodingStep::BooleanBitpack,
            LogicalType::Enum(_) => EncodingStep::EnumBitpack,
            LogicalType::SignedInteger(_)
            | LogicalType::Timestamp(_)
            | LogicalType::FixedDecimal(_) => {
                // Signed domains need zigzag before bit packing.
                return Ok(vec![EncodingStep::ZigZag, EncodingStep::BitPacking]);
            }
            LogicalType::UnsignedInteger(_) => EncodingStep::BitPacking,
            LogicalType::CategoricalString(_) => {
                return Err(Error::InvalidPlan {
                    message: format!(
                        "column `{column_name}`: `bitpack` alone is not valid for categorical \
                         strings; use [\"dictionary\", \"bitpack\"]"
                    ),
                });
            }
            LogicalType::Binary(_) => {
                return Err(Error::InvalidPlan {
                    message: format!(
                        "column `{column_name}`: `bitpack` alone is not valid for binary; use [\"plain\"]"
                    ),
                });
            }
        };
    }

    // Convenience: hint-like single steps that need a terminal.
    if steps.len() == 1 {
        match steps[0] {
            EncodingStep::DictionaryEncoding => {
                return Err(Error::InvalidPlan {
                    message: format!(
                        "column `{column_name}`: `dictionary` must be followed by a terminal \
                         step such as `bitpack` (e.g. [\"dictionary\", \"bitpack\"])"
                    ),
                });
            }
            EncodingStep::Delta | EncodingStep::DeltaOfDelta | EncodingStep::FrameOfReference => {
                return Err(Error::InvalidPlan {
                    message: format!(
                        "column `{column_name}`: `{}` must be followed by a terminal step \
                         (e.g. zigzag + bitpack, or varint)",
                        steps[0].as_str()
                    ),
                });
            }
            EncodingStep::ZigZag => {
                return Err(Error::InvalidPlan {
                    message: format!(
                        "column `{column_name}`: `zigzag` must be followed by a terminal step \
                         such as `bitpack` or `varint`"
                    ),
                });
            }
            _ => {}
        }
    }

    Ok(steps)
}

/// Validates that `plan` is type-compatible with `logical_type`.
pub fn validate_plan_for_type(
    column_name: &str,
    logical_type: &LogicalType,
    plan: &EncodingPlan,
) -> Result<()> {
    plan.validate().map_err(|e| Error::InvalidPlan {
        message: format!("column `{column_name}`: {e}"),
    })?;

    let mut kind = initial_kind(logical_type, plan);
    for step in &plan.steps {
        kind = apply_step(column_name, *step, kind)?;
    }
    if kind != Kind::Encoded {
        return Err(Error::InvalidPlan {
            message: format!(
                "column `{column_name}`: encoding pipeline must end in encoded bytes, \
                 but finished as {}",
                kind.name()
            ),
        });
    }
    Ok(())
}

fn initial_kind(logical_type: &LogicalType, plan: &EncodingPlan) -> Kind {
    match logical_type {
        LogicalType::Boolean(_) => Kind::Bool,
        LogicalType::CategoricalString(_) | LogicalType::Enum(_) | LogicalType::Binary(_) => {
            Kind::Bytes
        }
        LogicalType::UnsignedInteger(_) => {
            if plan_needs_signed(plan) {
                Kind::I64
            } else {
                Kind::U64
            }
        }
        LogicalType::SignedInteger(_)
        | LogicalType::Timestamp(_)
        | LogicalType::FixedDecimal(_) => Kind::I64,
    }
}

fn plan_needs_signed(plan: &EncodingPlan) -> bool {
    plan.steps.iter().any(|s| {
        matches!(
            s,
            EncodingStep::Delta
                | EncodingStep::DeltaOfDelta
                | EncodingStep::ZigZag
                | EncodingStep::FrameOfReference
        )
    })
}

fn apply_step(column_name: &str, step: EncodingStep, kind: Kind) -> Result<Kind> {
    let next = match (step, kind) {
        (EncodingStep::Plain, Kind::I64 | Kind::U64 | Kind::Bytes | Kind::Bool) => Kind::Encoded,
        (EncodingStep::Delta, Kind::I64) => Kind::I64,
        (EncodingStep::DeltaOfDelta, Kind::I64) => Kind::I64,
        (EncodingStep::ZigZag, Kind::I64) => Kind::U64,
        (EncodingStep::FrameOfReference, Kind::I64) => Kind::U64,
        (EncodingStep::Varint, Kind::U64) => Kind::Encoded,
        (EncodingStep::BitPacking, Kind::U64) => Kind::Encoded,
        (EncodingStep::RunLengthEncoding, Kind::U64) => Kind::Encoded,
        (EncodingStep::DictionaryEncoding, Kind::Bytes) => Kind::U64,
        (EncodingStep::BooleanBitpack, Kind::Bool) => Kind::Encoded,
        (EncodingStep::EnumBitpack, Kind::Bytes) => Kind::Encoded,
        _ => {
            return Err(Error::InvalidPlan {
                message: format!(
                    "column `{column_name}`: step `{}` is incompatible with input kind `{}`",
                    step.as_str(),
                    kind.name()
                ),
            });
        }
    };
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_core::{IntegerWidth, TimestampUnit};

    #[test]
    fn resolves_explicit_timestamp_pipeline() {
        let ty = LogicalType::timestamp(TimestampUnit::Nanoseconds);
        let enc = ColumnEncodingSpec::Pipeline(vec![
            "delta_of_delta".into(),
            "zigzag".into(),
            "bitpack".into(),
        ]);
        let plan = resolve_column_plan("ts", &ty, &enc).unwrap();
        assert_eq!(
            plan.steps,
            vec![
                EncodingStep::DeltaOfDelta,
                EncodingStep::ZigZag,
                EncodingStep::BitPacking
            ]
        );
    }

    #[test]
    fn rejects_dictionary_without_terminal() {
        let ty = LogicalType::categorical_string(None);
        let enc = ColumnEncodingSpec::Pipeline(vec!["dictionary".into()]);
        let err = resolve_column_plan("symbol", &ty, &enc).unwrap_err();
        assert!(err.to_string().contains("terminal"), "{err}");
    }

    #[test]
    fn rejects_varint_on_strings() {
        let ty = LogicalType::categorical_string(None);
        let enc = ColumnEncodingSpec::Pipeline(vec!["varint".into()]);
        let err = resolve_column_plan("symbol", &ty, &enc).unwrap_err();
        assert!(err.to_string().contains("incompatible"), "{err}");
    }

    #[test]
    fn bitpack_alone_on_enum_becomes_enum_bitpack() {
        let ty = LogicalType::enum_type(vec!["buy".into(), "sell".into()]);
        let enc = ColumnEncodingSpec::Pipeline(vec!["bitpack".into()]);
        let plan = resolve_column_plan("side", &ty, &enc).unwrap();
        assert_eq!(plan.steps, vec![EncodingStep::EnumBitpack]);
    }

    #[test]
    fn bitpack_alone_on_unsigned() {
        let ty = LogicalType::unsigned_integer(IntegerWidth::Bits32);
        let enc = ColumnEncodingSpec::Pipeline(vec!["bitpack".into()]);
        let plan = resolve_column_plan("qty", &ty, &enc).unwrap();
        assert_eq!(plan.steps, vec![EncodingStep::BitPacking]);
    }

    #[test]
    fn unknown_step_errors_clearly() {
        let ty = LogicalType::unsigned_integer(IntegerWidth::Bits64);
        let enc = ColumnEncodingSpec::Pipeline(vec!["lz4".into()]);
        let err = resolve_column_plan("x", &ty, &enc).unwrap_err();
        assert!(err.to_string().contains("unknown encoding step"), "{err}");
    }
}
