//! Human and JSON report formatters.

use strata_codec::EncodingPlan;
use strata_codec::EncodingStep;

use crate::error::{Error, Result};
use crate::report::OptimizeReport;

/// Pretty JSON for machine-readable optimize output.
pub fn format_json(report: &OptimizeReport) -> Result<String> {
    serde_json::to_string_pretty(report).map_err(|e| Error::Invalid {
        message: format!("failed to serialize optimize JSON: {e}"),
    })
}

/// Human-readable optimize report with WHY for every column.
pub fn format_human(report: &OptimizeReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("strata optimize: {}\n", report.source));
    out.push_str(&format!("  schema={}\n", report.schema_id));
    out.push_str(&format!("  goal={}\n", report.goal.as_str()));
    out.push_str(&format!("  records={}\n", report.record_count));
    out.push_str(&format!("  sample_rows={}\n", report.sample_rows));
    out.push_str(&format!("  files={}\n", report.file_count));
    out.push('\n');

    for col in &report.columns {
        out.push_str(&format!("{}:\n", col.name));
        out.push_str(&format!(
            "  selected {}\n",
            format_plan_display(&col.selected.plan)
        ));
        out.push_str("  reason:\n");
        for line in wrap_reason(&col.reason, 72) {
            out.push_str(&format!("    {line}\n"));
        }
        out.push_str(&format!(
            "  sample: {} bytes | encode {:.1} MB/s | decode {:.1} MB/s | peak mem {}\n",
            col.selected.encoded_bytes,
            col.selected.encode_mb_s,
            col.selected.decode_mb_s,
            format_bytes(col.selected.peak_memory_bytes),
        ));
        if col.trials.len() > 1 {
            out.push_str("  candidates considered:\n");
            for trial in &col.trials {
                let marker = if trial.plan == col.selected.plan {
                    "*"
                } else {
                    " "
                };
                out.push_str(&format!(
                    "   {marker} {} — {} bytes, encode {:.1} MB/s, decode {:.1} MB/s{}\n",
                    format_plan_display(&trial.plan),
                    trial.encoded_bytes,
                    trial.encode_mb_s,
                    trial.decode_mb_s,
                    if trial.feasible {
                        ""
                    } else {
                        " [rejected by constraints]"
                    }
                ));
            }
        }
        out.push('\n');
    }
    out
}

/// Display form matching product examples (`DeltaOfDelta → ZigZag → BitPack`).
pub fn format_plan_display(plan: &EncodingPlan) -> String {
    plan.steps
        .iter()
        .map(|s| step_display(*s))
        .collect::<Vec<_>>()
        .join(" → ")
}

fn step_display(step: EncodingStep) -> &'static str {
    match step {
        EncodingStep::Plain => "Plain",
        EncodingStep::Delta => "Delta",
        EncodingStep::DeltaOfDelta => "DeltaOfDelta",
        EncodingStep::ZigZag => "ZigZag",
        EncodingStep::Varint => "Varint",
        EncodingStep::FrameOfReference => "FrameOfReference",
        EncodingStep::BitPacking => "BitPack",
        EncodingStep::RunLengthEncoding => "RunLength",
        EncodingStep::DictionaryEncoding => "Dictionary",
        EncodingStep::BooleanBitpack => "BooleanBitpack",
        EncodingStep::EnumBitpack => "EnumBitpack",
    }
}

fn wrap_reason(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn format_bytes(n: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let v = n as f64;
    if v >= GB {
        format!("{:.2} GB", v / GB)
    } else if v >= MB {
        format!("{:.2} MB", v / MB)
    } else if v >= KB {
        format!("{:.1} KB", v / KB)
    } else {
        format!("{n} B")
    }
}
