//! Generate sensible candidate encoding pipelines from type + stats.

use strata_analyze::{ColumnStats, DeltaVarianceLabel};
use strata_codec::{validate_plan_for_type, EncodingPlan, EncodingStep};
use strata_core::LogicalType;

use crate::error::Result;

/// Builds a de-duplicated list of type-valid candidate plans for one column.
pub fn candidate_plans(
    logical_type: &LogicalType,
    stats: &ColumnStats,
) -> Result<Vec<EncodingPlan>> {
    let mut raw: Vec<Vec<EncodingStep>> = Vec::new();

    match logical_type {
        LogicalType::Boolean(_) => {
            raw.push(vec![EncodingStep::BooleanBitpack]);
            raw.push(vec![EncodingStep::Plain]);
        }
        LogicalType::Enum(_) => {
            raw.push(vec![EncodingStep::EnumBitpack]);
            raw.push(vec![
                EncodingStep::DictionaryEncoding,
                EncodingStep::BitPacking,
            ]);
            raw.push(vec![EncodingStep::Plain]);
            if stats
                .run_length
                .as_ref()
                .map(|r| r.avg_run_length >= 2.0)
                .unwrap_or(false)
            {
                raw.push(vec![
                    EncodingStep::DictionaryEncoding,
                    EncodingStep::RunLengthEncoding,
                ]);
            }
        }
        LogicalType::CategoricalString(_) => {
            raw.push(vec![
                EncodingStep::DictionaryEncoding,
                EncodingStep::BitPacking,
            ]);
            raw.push(vec![EncodingStep::DictionaryEncoding, EncodingStep::Varint]);
            raw.push(vec![EncodingStep::Plain]);
            if stats
                .run_length
                .as_ref()
                .map(|r| r.avg_run_length >= 2.0)
                .unwrap_or(false)
            {
                raw.push(vec![
                    EncodingStep::DictionaryEncoding,
                    EncodingStep::RunLengthEncoding,
                ]);
            }
        }
        LogicalType::Binary(_) => {
            raw.push(vec![EncodingStep::Plain]);
        }
        LogicalType::UnsignedInteger(_) => {
            push_unsigned_candidates(&mut raw, stats);
        }
        LogicalType::SignedInteger(_)
        | LogicalType::Timestamp(_)
        | LogicalType::FixedDecimal(_) => {
            push_signed_candidates(&mut raw, stats);
        }
    }

    let mut out = Vec::new();
    let mut seen = Vec::new();
    for steps in raw {
        let Ok(plan) = EncodingPlan::new(steps) else {
            continue;
        };
        if validate_plan_for_type(&stats.name, logical_type, &plan).is_err() {
            continue;
        }
        if seen.iter().any(|p: &EncodingPlan| p == &plan) {
            continue;
        }
        seen.push(plan.clone());
        out.push(plan);
    }

    if out.is_empty() {
        // Always have a fallback.
        out.push(EncodingPlan::plain());
    }
    Ok(out)
}

fn push_unsigned_candidates(raw: &mut Vec<Vec<EncodingStep>>, stats: &ColumnStats) {
    raw.push(vec![EncodingStep::Plain]);
    raw.push(vec![EncodingStep::BitPacking]);
    raw.push(vec![EncodingStep::Varint]);
    raw.push(vec![
        EncodingStep::FrameOfReference,
        EncodingStep::BitPacking,
    ]);
    raw.push(vec![
        EncodingStep::Delta,
        EncodingStep::ZigZag,
        EncodingStep::BitPacking,
    ]);
    raw.push(vec![
        EncodingStep::Delta,
        EncodingStep::ZigZag,
        EncodingStep::Varint,
    ]);
    raw.push(vec![EncodingStep::RunLengthEncoding]);

    if prefer_delta_of_delta(stats) {
        raw.push(vec![
            EncodingStep::DeltaOfDelta,
            EncodingStep::ZigZag,
            EncodingStep::BitPacking,
        ]);
    }
    if prefer_rle(stats) {
        raw.push(vec![
            EncodingStep::Delta,
            EncodingStep::ZigZag,
            EncodingStep::RunLengthEncoding,
        ]);
    }
}

fn push_signed_candidates(raw: &mut Vec<Vec<EncodingStep>>, stats: &ColumnStats) {
    raw.push(vec![EncodingStep::Plain]);
    raw.push(vec![EncodingStep::ZigZag, EncodingStep::BitPacking]);
    raw.push(vec![EncodingStep::ZigZag, EncodingStep::Varint]);
    raw.push(vec![
        EncodingStep::FrameOfReference,
        EncodingStep::BitPacking,
    ]);
    raw.push(vec![
        EncodingStep::Delta,
        EncodingStep::ZigZag,
        EncodingStep::BitPacking,
    ]);
    raw.push(vec![
        EncodingStep::DeltaOfDelta,
        EncodingStep::ZigZag,
        EncodingStep::BitPacking,
    ]);
    raw.push(vec![EncodingStep::ZigZag, EncodingStep::RunLengthEncoding]);

    if prefer_delta_of_delta(stats) {
        raw.push(vec![
            EncodingStep::DeltaOfDelta,
            EncodingStep::ZigZag,
            EncodingStep::Varint,
        ]);
    }
    if prefer_rle(stats) {
        raw.push(vec![
            EncodingStep::Delta,
            EncodingStep::ZigZag,
            EncodingStep::RunLengthEncoding,
        ]);
    }
    // Narrow local ranges favor FOR over delta.
    if stats
        .local_block_ranges
        .as_ref()
        .map(|l| l.label == "narrow")
        .unwrap_or(false)
    {
        raw.push(vec![EncodingStep::FrameOfReference, EncodingStep::Varint]);
    }
}

fn prefer_delta_of_delta(stats: &ColumnStats) -> bool {
    let mono = stats.monotonicity.unwrap_or(0.0);
    let low_var = matches!(
        stats.delta_variance_label,
        Some(DeltaVarianceLabel::Low) | Some(DeltaVarianceLabel::None)
    );
    mono >= 0.95 && low_var
}

fn prefer_rle(stats: &ColumnStats) -> bool {
    stats
        .run_length
        .as_ref()
        .map(|r| r.avg_run_length >= 4.0 || r.max_run_fraction >= 0.25)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_analyze::{AnalysisReport, AnalyzeOptions};
    use strata_core::{Field, FieldValue, Record, Schema, TimestampUnit};

    #[test]
    fn timestamp_candidates_include_dod() {
        let schema = Schema::new(
            "t",
            vec![Field::required(
                "ts",
                LogicalType::timestamp(TimestampUnit::Nanoseconds),
            )],
        )
        .unwrap();
        let records: Vec<_> = (0..100)
            .map(|i| Record::new().with("ts", FieldValue::Timestamp(1_000 + i * 100)))
            .collect();
        let report = AnalysisReport::from_records(
            "x",
            &schema,
            &records,
            &AnalyzeOptions { block_rows: 64 },
            1,
        )
        .unwrap();
        let stats = &report.columns[0];
        let plans = candidate_plans(&schema.fields[0].logical_type, stats).unwrap();
        assert!(plans.iter().any(|p| p.steps
            == [
                EncodingStep::DeltaOfDelta,
                EncodingStep::ZigZag,
                EncodingStep::BitPacking
            ]));
    }
}
