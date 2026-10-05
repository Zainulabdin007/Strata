//! Human-readable and JSON report formatters.

use crate::error::{Error, Result};
use crate::report::AnalysisReport;
use crate::stats::{ColumnStats, DeltaVarianceLabel, StatScalar};

/// Serializes a report as pretty JSON.
pub fn format_json(report: &AnalysisReport) -> Result<String> {
    serde_json::to_string_pretty(report).map_err(|e| Error::Invalid {
        message: format!("failed to serialize analysis JSON: {e}"),
    })
}

/// Formats a human-readable CLI report.
pub fn format_human(report: &AnalysisReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("strata analyze: {}\n", report.source));
    out.push_str(&format!("  schema={}\n", report.schema_id));
    out.push_str(&format!("  records={}\n", report.record_count));
    if report.file_count > 0 {
        out.push_str(&format!("  files={}\n", report.file_count));
    }
    out.push_str(&format!("  block_rows={}\n", report.block_rows));
    out.push('\n');

    for col in &report.columns {
        out.push_str(&format_column(col));
        out.push('\n');
    }
    out
}

fn format_column(col: &ColumnStats) -> String {
    let mut lines = Vec::new();
    lines.push(col.name.clone());

    lines.push(format!("  logical type: {}", col.logical_type));
    lines.push(format!(
        "  rows: {} (nulls: {}, present: {})",
        col.row_count, col.null_count, col.present_count
    ));

    if let (Some(min), Some(max)) = (&col.min, &col.max) {
        lines.push(format!("  min: {min}"));
        lines.push(format!("  max: {max}"));
    }

    lines.push(format!("  cardinality: {}", col.cardinality));
    lines.push(format!(
        "  cardinality ratio: {}",
        format_ratio_pct(col.cardinality_ratio)
    ));

    if let Some(m) = col.monotonicity {
        lines.push(format!("  monotonicity: {}", format_pct(m)));
    }
    if let Some(s) = col.sortedness {
        lines.push(format!("  sortedness: {}", format_pct(s)));
    }

    if let Some(median) = col.median_delta {
        lines.push(format!(
            "  median delta: {}",
            format_delta(median, col.timestamp_unit.as_deref())
        ));
    }
    if let Some(avg) = col.avg_delta {
        lines.push(format!(
            "  average delta: {}",
            format_delta(avg, col.timestamp_unit.as_deref())
        ));
    }
    if let Some(label) = col.delta_variance_label {
        if label != DeltaVarianceLabel::None {
            let var = col
                .delta_variance
                .map(|v| format!(" ({v:.4})"))
                .unwrap_or_default();
            lines.push(format!("  delta variance: {}{var}", label.as_str()));
        }
    }

    if let Some(rl) = &col.run_length {
        lines.push(format!(
            "  run-length: runs={}, avg={:.2}, max={}",
            rl.run_count, rl.avg_run_length, rl.max_run_length
        ));
    }

    if let Some(w) = col.min_bit_width {
        lines.push(format!("  min bit width: {w}"));
    }
    if let Some(w) = col.absolute_bit_width {
        lines.push(format!("  absolute bit width: {w}"));
    }

    if let Some(local) = &col.local_block_ranges {
        lines.push(format!(
            "  local range: {} (avg={:.2}, max={:.2}, global={:.2})",
            local.label, local.avg_range, local.max_range, local.global_range
        ));
    }

    if let Some(dict) = col.dictionary_size {
        lines.push(format!("  string dictionary size: {dict}"));
    }

    if let Some(rep) = &col.repetition {
        lines.push(format!(
            "  repetition: mode={} count={} freq={}",
            format_scalar(&rep.mode),
            rep.mode_count,
            format_pct(rep.mode_frequency)
        ));
    }

    lines.join("\n") + "\n"
}

fn format_pct(v: f64) -> String {
    format!("{:.2}%", v * 100.0)
}

fn format_ratio_pct(v: f64) -> String {
    // cardinality / rows — show as percentage of present rows that are unique.
    format!("{:.3}%", v * 100.0)
}

fn format_delta(v: f64, timestamp_unit: Option<&str>) -> String {
    let rounded = if (v - v.round()).abs() < 1e-9 {
        format!("{:.0}", v.round())
    } else if v.abs() >= 1_000.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.3}")
    };
    match timestamp_unit {
        Some(unit) => format!("{rounded}{unit}"),
        None => rounded,
    }
}

fn format_scalar(v: &StatScalar) -> String {
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::{AnalysisReport, AnalyzeOptions};
    use strata_core::{Field, FieldValue, LogicalType, Record, Schema, TimestampUnit};

    #[test]
    fn human_and_json_are_deterministic() {
        let schema = Schema::new(
            "demo",
            vec![
                Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
                Field::required(
                    "side",
                    LogicalType::enum_type(vec!["buy".into(), "sell".into()]),
                ),
            ],
        )
        .unwrap();
        let records = vec![
            Record::new()
                .with("ts", FieldValue::Timestamp(100))
                .with("side", FieldValue::Enum("buy".into())),
            Record::new()
                .with("ts", FieldValue::Timestamp(200))
                .with("side", FieldValue::Enum("sell".into())),
            Record::new()
                .with("ts", FieldValue::Timestamp(300))
                .with("side", FieldValue::Enum("buy".into())),
        ];
        let report = AnalysisReport::from_records(
            "fixture",
            &schema,
            &records,
            &AnalyzeOptions { block_rows: 64 },
            1,
        )
        .unwrap();
        let a = format_human(&report);
        let b = format_human(&report);
        assert_eq!(a, b);
        assert!(a.contains("ts\n"));
        assert!(a.contains("monotonicity: 100.00%"));
        assert!(a.contains("side\n"));
        assert!(a.contains("cardinality: 2"));

        let j1 = format_json(&report).unwrap();
        let j2 = format_json(&report).unwrap();
        assert_eq!(j1, j2);
        assert!(j1.contains("\"schema_id\": \"demo\""));
    }
}
