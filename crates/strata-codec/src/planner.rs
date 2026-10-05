//! Select an [`EncodingPlan`] from logical type + encoding hints.

use strata_core::{FieldEncodingHint, LogicalType};

use crate::error::Result;
use crate::plan::{EncodingPlan, EncodingStep};

/// Builds a default composable plan for a field.
pub fn plan_for_field(logical_type: &LogicalType, hint: FieldEncodingHint) -> Result<EncodingPlan> {
    let steps = match logical_type {
        LogicalType::Boolean(_) => vec![EncodingStep::BooleanBitpack],
        LogicalType::Enum(_) => match hint {
            FieldEncodingHint::Plain => vec![EncodingStep::Plain],
            _ => vec![EncodingStep::EnumBitpack],
        },
        LogicalType::CategoricalString(_) => match hint {
            FieldEncodingHint::Plain => vec![EncodingStep::Plain],
            _ => vec![EncodingStep::DictionaryEncoding, EncodingStep::BitPacking],
        },
        LogicalType::Binary(_) => vec![EncodingStep::Plain],
        LogicalType::Timestamp(_) => match hint {
            FieldEncodingHint::Plain => vec![EncodingStep::Plain],
            FieldEncodingHint::RunLength => {
                vec![EncodingStep::ZigZag, EncodingStep::RunLengthEncoding]
            }
            FieldEncodingHint::BitPack => {
                vec![EncodingStep::ZigZag, EncodingStep::BitPacking]
            }
            _ => vec![
                EncodingStep::DeltaOfDelta,
                EncodingStep::ZigZag,
                EncodingStep::BitPacking,
            ],
        },
        LogicalType::FixedDecimal(_) => match hint {
            FieldEncodingHint::Plain => vec![EncodingStep::Plain],
            FieldEncodingHint::Delta => vec![
                EncodingStep::Delta,
                EncodingStep::ZigZag,
                EncodingStep::BitPacking,
            ],
            FieldEncodingHint::RunLength => {
                vec![EncodingStep::ZigZag, EncodingStep::RunLengthEncoding]
            }
            _ => vec![EncodingStep::FrameOfReference, EncodingStep::BitPacking],
        },
        LogicalType::UnsignedInteger(_) => match hint {
            FieldEncodingHint::Plain => vec![EncodingStep::Plain],
            FieldEncodingHint::Delta => vec![
                EncodingStep::Delta,
                EncodingStep::ZigZag,
                EncodingStep::BitPacking,
            ],
            FieldEncodingHint::RunLength => vec![EncodingStep::RunLengthEncoding],
            FieldEncodingHint::BitPack => vec![EncodingStep::BitPacking],
            FieldEncodingHint::Dictionary | FieldEncodingHint::Auto => {
                vec![EncodingStep::FrameOfReference, EncodingStep::BitPacking]
            }
        },
        LogicalType::SignedInteger(_) => match hint {
            FieldEncodingHint::Plain => vec![EncodingStep::Plain],
            FieldEncodingHint::Delta => vec![
                EncodingStep::Delta,
                EncodingStep::ZigZag,
                EncodingStep::BitPacking,
            ],
            FieldEncodingHint::RunLength => {
                vec![EncodingStep::ZigZag, EncodingStep::RunLengthEncoding]
            }
            FieldEncodingHint::BitPack => {
                vec![EncodingStep::ZigZag, EncodingStep::BitPacking]
            }
            FieldEncodingHint::Dictionary | FieldEncodingHint::Auto => {
                vec![EncodingStep::FrameOfReference, EncodingStep::BitPacking]
            }
        },
    };
    EncodingPlan::new(steps)
}

/// Explicit example pipelines from the design doc.
pub mod examples {
    use super::*;

    /// Timestamp: DeltaOfDelta → ZigZag → BitPack
    pub fn timestamp() -> EncodingPlan {
        EncodingPlan::new(vec![
            EncodingStep::DeltaOfDelta,
            EncodingStep::ZigZag,
            EncodingStep::BitPacking,
        ])
        .expect("valid")
    }

    /// Price: FrameOfReference → BitPack
    pub fn price() -> EncodingPlan {
        EncodingPlan::new(vec![
            EncodingStep::FrameOfReference,
            EncodingStep::BitPacking,
        ])
        .expect("valid")
    }

    /// Symbol: Dictionary → BitPack
    pub fn symbol() -> EncodingPlan {
        EncodingPlan::new(vec![
            EncodingStep::DictionaryEncoding,
            EncodingStep::BitPacking,
        ])
        .expect("valid")
    }

    /// Side: 1-bit / enum bitpack
    pub fn side() -> EncodingPlan {
        EncodingPlan::new(vec![EncodingStep::EnumBitpack]).expect("valid")
    }
}
