//! Human-readable WHY text for selected encodings.

use strata_analyze::{ColumnStats, DeltaVarianceLabel};
use strata_codec::EncodingPlan;

use crate::format::format_plan_display;
use crate::select::Selection;

/// Builds the selection reason paragraph for one column.
pub fn explain_selection(
    stats: &ColumnStats,
    plan: &EncodingPlan,
    selection: &Selection,
) -> String {
    let mut facts = Vec::new();

    if let Some(m) = stats.monotonicity {
        if m >= 0.999 {
            facts.push("100% monotonic".to_string());
        } else if m >= 0.9 {
            facts.push(format!("{:.0}% monotonic", m * 100.0));
        }
    }
    if let Some(s) = stats.sortedness {
        if s >= 0.999 {
            facts.push("fully sorted".to_string());
        }
    }
    match stats.delta_variance_label {
        Some(DeltaVarianceLabel::Low) => facts.push("low delta variance".to_string()),
        Some(DeltaVarianceLabel::Medium) => facts.push("medium delta variance".to_string()),
        Some(DeltaVarianceLabel::High) => facts.push("high delta variance".to_string()),
        _ => {}
    }
    if stats.cardinality_ratio <= 0.01 && stats.present_count > 0 {
        facts.push(format!(
            "cardinality {} ({:.3}% ratio)",
            stats.cardinality,
            stats.cardinality_ratio * 100.0
        ));
    } else if stats.cardinality <= 16 && stats.present_count > 0 {
        facts.push(format!("cardinality {}", stats.cardinality));
    }
    if let Some(local) = &stats.local_block_ranges {
        if local.label == "narrow" {
            facts.push("narrow local ranges".to_string());
        }
    }
    if let Some(rl) = &stats.run_length {
        if rl.avg_run_length >= 2.0 {
            facts.push(format!("avg run length {:.1}", rl.avg_run_length));
        }
    }
    if let Some(w) = stats.min_bit_width {
        if w <= 16 {
            facts.push(format!("min bit width {w}"));
        }
    }

    let plan_label = format_plan_display(plan);
    let mut reason = String::new();
    if !facts.is_empty() {
        reason.push_str(&facts.join(", "));
        reason.push_str(", ");
    }
    reason.push_str(&selection.rationale);
    reason.push_str(&format!(
        " [selected {plan_label}; sample {} bytes, encode {:.1} MB/s, decode {:.1} MB/s]",
        selection.result.encoded_bytes, selection.result.encode_mb_s, selection.result.decode_mb_s
    ));
    reason
}
