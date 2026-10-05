//! Top-level optimize orchestration and report types.

use serde::Serialize;
use strata_analyze::{AnalyzeOptions, ColumnStats, Profiler};
use strata_codec::EncodingPlan;
use strata_core::{CompressionProfile, FieldValue, Record, SemanticHint};

use crate::bench::{bench_plan, BenchOptions};
use crate::candidates::candidate_plans;
use crate::emit::emit_profile_toml;
use crate::error::{Error, Result};
use crate::goal::{OptimizationGoal, OptimizeOptions};
use crate::reason::explain_selection;
use crate::select::{satisfies, select_plan};

/// One benched candidate (JSON-friendly).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PipelineTrial {
    /// Pipeline steps (canonical names).
    pub steps: Vec<String>,
    /// Encoded sample size.
    pub encoded_bytes: u64,
    /// Logical sample size.
    pub logical_bytes: u64,
    /// Observed sample logical / encoded payload bytes.
    pub compression_ratio: f64,
    /// Encode MB/s.
    pub encode_mb_s: f64,
    /// Decode MB/s.
    pub decode_mb_s: f64,
    /// Estimated peak memory.
    pub peak_memory_bytes: u64,
    /// Whether this trial satisfied constraints.
    pub feasible: bool,
    /// Underlying plan.
    #[serde(skip)]
    pub plan: EncodingPlan,
}

/// Winning plan summary.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SelectedPlan {
    /// Pipeline steps (canonical names).
    pub steps: Vec<String>,
    /// Encoded sample size.
    pub encoded_bytes: u64,
    /// Observed sample logical / encoded payload bytes.
    pub compression_ratio: f64,
    /// Encode MB/s.
    pub encode_mb_s: f64,
    /// Decode MB/s.
    pub decode_mb_s: f64,
    /// Estimated peak memory.
    pub peak_memory_bytes: u64,
    /// Underlying plan.
    #[serde(skip)]
    pub plan: EncodingPlan,
}

/// Per-column optimize decision with WHY text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ColumnDecision {
    /// Column name.
    pub name: String,
    /// Logical type name.
    pub logical_type: String,
    /// Selected plan summary.
    pub selected: SelectedPlan,
    /// Human WHY explanation.
    pub reason: String,
    /// All trials considered.
    pub trials: Vec<PipelineTrial>,
    /// Semantic hints derived / preserved for the emitted profile.
    pub hints: Vec<SemanticHint>,
}

/// Full optimize report.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OptimizeReport {
    /// Input path / label.
    pub source: String,
    /// Schema id.
    pub schema_id: String,
    /// Goal used.
    pub goal: OptimizationGoal,
    /// Records observed while profiling.
    pub record_count: u64,
    /// Files analyzed.
    pub file_count: u64,
    /// Sample rows used for encode benches.
    pub sample_rows: u32,
    /// Column decisions in schema order.
    pub columns: Vec<ColumnDecision>,
    /// Generated CompressionProfile TOML.
    pub profile_toml: String,
}

/// Runs profiling + candidate benches + selection for an in-memory record set.
pub fn optimize_dataset(
    source: impl Into<String>,
    profile: &CompressionProfile,
    records: &[Record],
    file_count: u64,
    options: &OptimizeOptions,
) -> Result<OptimizeReport> {
    let schema = profile.schema()?;
    let block_rows = options.block_rows.unwrap_or(profile.block.row_count);
    let sample_target = options.sample_rows.unwrap_or(block_rows.clamp(64, 4_096));

    let analyze_opts = AnalyzeOptions { block_rows };
    let mut profiler = Profiler::new(source, &schema, &analyze_opts);
    profiler.set_file_count(file_count);
    profiler.push_records(&schema, records);
    let analysis = profiler.finish();

    let sample = build_sample(records, sample_target as usize);
    let sample_rows = sample.len() as u32;
    if sample_rows == 0 {
        return Err(Error::Invalid {
            message: "no records available to optimize".into(),
        });
    }

    let bench_opts = BenchOptions::from(options);
    let mut decisions = Vec::with_capacity(schema.fields.len());

    for field in &schema.fields {
        let stats = analysis
            .columns
            .iter()
            .find(|c| c.name == field.name)
            .ok_or_else(|| Error::Invalid {
                message: format!("missing profiler stats for column `{}`", field.name),
            })?;

        let values = column_values(&sample, &field.name);
        let candidates = candidate_plans(&field.logical_type, stats)?;
        let mut benches = Vec::with_capacity(candidates.len());
        for plan in &candidates {
            if let Ok(result) = bench_plan(&field.logical_type, &values, plan, bench_opts) {
                benches.push(result);
            }
        }
        if benches.is_empty() {
            let plan = EncodingPlan::plain();
            benches.push(bench_plan(&field.logical_type, &values, &plan, bench_opts)?);
        }

        let selection = select_plan(&benches, options.goal, &options.constraints)?;
        let reason = explain_selection(stats, &selection.result.plan, &selection);
        let hints = merge_hints(profile, &field.name, stats);

        let trials: Vec<PipelineTrial> = benches
            .iter()
            .map(|b| PipelineTrial {
                steps: steps_names(&b.plan),
                encoded_bytes: b.encoded_bytes,
                logical_bytes: b.logical_bytes,
                compression_ratio: strata_metrics::CompressionRatio::from_sizes(
                    strata_metrics::Bytes(b.logical_bytes),
                    strata_metrics::Bytes(b.encoded_bytes),
                )
                .get(),
                encode_mb_s: b.encode_mb_s,
                decode_mb_s: b.decode_mb_s,
                peak_memory_bytes: b.peak_memory_bytes,
                feasible: satisfies(b, &options.constraints),
                plan: b.plan.clone(),
            })
            .collect();

        let selected = SelectedPlan {
            steps: steps_names(&selection.result.plan),
            encoded_bytes: selection.result.encoded_bytes,
            compression_ratio: selection.result.compression_ratio(),
            encode_mb_s: selection.result.encode_mb_s,
            decode_mb_s: selection.result.decode_mb_s,
            peak_memory_bytes: selection.result.peak_memory_bytes,
            plan: selection.result.plan.clone(),
        };

        decisions.push(ColumnDecision {
            name: field.name.clone(),
            logical_type: field.logical_type.name().to_string(),
            selected,
            reason,
            trials,
            hints,
        });
    }

    let profile_name = format!("{}-optimized", profile.name);
    let profile_toml = emit_profile_toml(profile, &decisions, &profile_name)?;

    Ok(OptimizeReport {
        source: analysis.source,
        schema_id: analysis.schema_id,
        goal: options.goal,
        record_count: analysis.record_count,
        file_count: analysis.file_count,
        sample_rows,
        columns: decisions,
        profile_toml,
    })
}

fn steps_names(plan: &EncodingPlan) -> Vec<String> {
    plan.steps.iter().map(|s| s.as_str().to_string()).collect()
}

fn column_values(records: &[Record], name: &str) -> Vec<FieldValue> {
    records
        .iter()
        .map(|r| r.get(name).cloned().unwrap_or(FieldValue::Null))
        .collect()
}

fn build_sample(records: &[Record], target_rows: usize) -> Vec<Record> {
    records[..records.len().min(target_rows)].to_vec()
}

fn merge_hints(
    profile: &CompressionProfile,
    column: &str,
    stats: &ColumnStats,
) -> Vec<SemanticHint> {
    let mut hints = profile
        .columns
        .iter()
        .find(|c| c.name == column)
        .map(|c| c.hints.clone())
        .unwrap_or_default();

    let push = |hints: &mut Vec<SemanticHint>, h: SemanticHint| {
        if !hints.contains(&h) {
            hints.push(h);
        }
    };

    if stats.monotonicity.unwrap_or(0.0) >= 0.95 {
        push(&mut hints, SemanticHint::Monotonic);
    }
    if stats.sortedness.unwrap_or(0.0) >= 0.95 {
        push(&mut hints, SemanticHint::Sorted);
    }
    if stats.cardinality_ratio <= 0.05 && stats.present_count > 0 {
        push(&mut hints, SemanticHint::LowCardinality);
    }
    if matches!(
        stats.delta_variance_label,
        Some(strata_analyze::DeltaVarianceLabel::Low)
    ) && stats.monotonicity.unwrap_or(0.0) >= 0.8
    {
        push(&mut hints, SemanticHint::DeltaFriendly);
    }
    if stats.logical_type == "fixed_decimal" {
        push(&mut hints, SemanticHint::FixedPrecision);
    }
    hints
}
