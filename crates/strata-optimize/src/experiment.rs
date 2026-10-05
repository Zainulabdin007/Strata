//! Real storage experiments: candidates use the production writer and decoder.
use crate::{optimize_dataset, OptimizationGoal, OptimizeOptions};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path, sync::Arc, time::Instant};
use strata_codec::{compressor_for, CompressionCodec, EncodingPlan, EncodingStep};
use strata_core::{CompressionProfile, Record};
use strata_metrics::{MetricsHandle, MetricsSnapshot};
use strata_storage::{
    write_records_parallel, DatasetReader, DatasetWriterOptions, PipelineOptions,
};

/// User overrides, applied to every measured candidate.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExperimentOptions {
    /// Rows per stored block.
    pub block_rows: Option<u32>,
    /// Physical segment target.
    pub segment_bytes: Option<u64>,
    /// Encode workers, measured through the real pipeline.
    pub workers: Option<usize>,
    /// Restrict general compression to none/lz4/zstd; omitted tests all three.
    pub codec: Option<String>,
    /// Explicit semantic step names per column.
    pub pipelines: HashMap<String, Vec<String>>,
}
/// One actually written, closed, decoded and equality-checked candidate dataset.
#[derive(Debug, Clone, Serialize)]
pub struct ExperimentTrial {
    /// Candidate family and general compressor.
    pub name: String,
    /// Canonical ingest observations (physical bytes include framing/manifest).
    pub metrics: MetricsSnapshot,
    /// Wall-clock full decode throughput, decimal MB/s.
    pub decode_mb_s: f64,
    /// Number of decoded rows checked against input.
    pub verified_records: u64,
    /// Complete valid profile used for this candidate.
    pub profile_toml: String,
}
/// Measured experiment results; no extrapolation to full-dataset size.
#[derive(Debug, Clone, Serialize)]
pub struct ExperimentReport {
    /// Whether this binary used release settings.
    pub release_build: bool,
    /// Actual input sample row count.
    pub sample_records: usize,
    /// Explicit configuration used in the experiment.
    pub options: ExperimentOptions,
    /// Every successfully measured candidate.
    pub trials: Vec<ExperimentTrial>,
    /// Candidate errors (never silently hidden).
    pub failures: Vec<String>,
    /// Index of winner in trials.
    pub winner: usize,
    /// Winner's reusable profile.
    pub profile_toml: String,
    /// Measured selection reason.
    pub reason: String,
}
/// Run experiments under a caller-owned empty scratch directory.
pub fn experiment(
    base: &CompressionProfile,
    records: &[Record],
    goal: OptimizationGoal,
    options: &ExperimentOptions,
    scratch: &Path,
) -> anyhow::Result<ExperimentReport> {
    anyhow::ensure!(!records.is_empty(), "no records to analyze");
    let mut base = base.clone();
    base.block.row_count = options.block_rows.unwrap_or(base.block.row_count);
    base.segment.target_size_bytes = options
        .segment_bytes
        .unwrap_or(base.segment.target_size_bytes);
    let workers = options.workers.unwrap_or(2);
    anyhow::ensure!((1..=32).contains(&workers), "workers must be 1..32");
    anyhow::ensure!(
        (1..=1_048_576).contains(&base.block.row_count),
        "block rows must be 1..1048576"
    );
    anyhow::ensure!(
        (1024..=1024 * 1024 * 1024).contains(&base.segment.target_size_bytes),
        "segment bytes must be 1024..1073741824"
    );
    base.validate()?;
    let schema = base.schema()?;
    let mut overrides = HashMap::new();
    for (name, steps) in &options.pipelines {
        let field = schema
            .field(name)
            .ok_or_else(|| anyhow::anyhow!("unknown column {name}"))?;
        let plan = EncodingPlan::new(
            steps
                .iter()
                .map(|s| {
                    EncodingStep::parse_name(s)
                        .ok_or_else(|| anyhow::anyhow!("unknown encoding {s}"))
                })
                .collect::<anyhow::Result<Vec<_>>>()?,
        )?;
        strata_codec::validate_plan_for_type(name, &field.logical_type, &plan)?;
        overrides.insert(name.clone(), plan);
    }
    let automatic = optimize_dataset(
        "experiment sample",
        &base,
        records,
        1,
        &OptimizeOptions {
            goal,
            ..Default::default()
        },
    )?;
    let codecs = if let Some(codec) = &options.codec {
        vec![CompressionCodec::parse_name(codec).ok_or_else(|| anyhow::anyhow!("unknown codec"))?]
    } else {
        vec![
            CompressionCodec::None,
            CompressionCodec::Lz4,
            CompressionCodec::Zstd,
        ]
    };
    let mut trials = Vec::new();
    let mut failures = Vec::new();
    for family in [
        "Plain",
        "Manual",
        "Automatic",
        "Delta + BitPack",
        "DeltaOfDelta + BitPack",
    ] {
        for codec in &codecs {
            let mut decisions = automatic.columns.clone();
            let manual = strata_codec::resolve_profile_plans(&base)?;
            for decision in &mut decisions {
                let ty = &schema.field(&decision.name).unwrap().logical_type;
                let proposed = match family {
                    "Plain" => Some(EncodingPlan::plain()),
                    "Manual" => manual.get(&decision.name).cloned(),
                    "Delta + BitPack" | "DeltaOfDelta + BitPack" => EncodingPlan::new(vec![
                        if family.starts_with("DeltaOfDelta") {
                            EncodingStep::DeltaOfDelta
                        } else {
                            EncodingStep::Delta
                        },
                        EncodingStep::ZigZag,
                        EncodingStep::BitPacking,
                    ])
                    .ok()
                    .filter(|p| {
                        strata_codec::validate_plan_for_type(&decision.name, ty, p).is_ok()
                    }),
                    _ => None,
                };
                if let Some(plan) = overrides.get(&decision.name).cloned().or(proposed) {
                    decision.selected.plan = plan;
                }
            }
            base.compression.codec =
                strata_core::CompressionCodecName::parse_name(codec.as_str()).unwrap();
            base.compression.enable_general = false;
            let text = crate::emit_profile_toml(
                &base,
                &decisions,
                &format!("{}-{}-{}", base.name, family, codec.as_str()),
            )?;
            let name = format!("{family} + {}", codec.as_str());
            let root = scratch.join(format!("candidate-{}-{}", trials.len(), failures.len()));
            let measured = (|| -> anyhow::Result<ExperimentTrial> {
                let profile = strata_core::parse_profile_str(&text, "experiment.toml")?;
                let metrics = MetricsHandle::new();
                let mut opts = PipelineOptions::with_workers(workers);
                opts.writer = DatasetWriterOptions {
                    block_row_count: base.block.row_count,
                    segment_target_size_bytes: base.segment.target_size_bytes,
                    encoding_plans: strata_codec::resolve_profile_plans(&profile)?,
                    compressor: Arc::from(compressor_for(*codec, base.compression.level)),
                    metrics: Some(metrics.clone()),
                    durability: base.durability.mode,
                    sync_every_blocks: base.durability.sync_every_blocks,
                    ..Default::default()
                };
                write_records_parallel(&root, schema.clone(), records, opts)?;
                let metrics = metrics.finish();
                let t = Instant::now();
                let reader = DatasetReader::open(&root)?;
                let mut offset = 0;
                for i in 0..reader.segment_count() {
                    let seg = reader.open_segment(i)?;
                    for b in 0..seg.block_count() {
                        let batch = seg.read_block(b)?;
                        anyhow::ensure!(
                            batch.records == records[offset..offset + batch.records.len()],
                            "candidate roundtrip mismatch"
                        );
                        offset += batch.records.len();
                        std::hint::black_box(&batch);
                    }
                }
                anyhow::ensure!(offset == records.len(), "candidate lost records");
                let decode_mb_s = metrics.logical_input_bytes.get() as f64
                    / 1_000_000.0
                    / t.elapsed().as_secs_f64();
                Ok(ExperimentTrial {
                    name: name.clone(),
                    metrics,
                    decode_mb_s,
                    verified_records: offset as u64,
                    profile_toml: text,
                })
            })();
            match measured {
                Ok(t) => trials.push(t),
                Err(e) => failures.push(format!("{name}: {e:#}")),
            }
            if root.exists() {
                std::fs::remove_dir_all(&root)?;
            }
        }
    }
    anyhow::ensure!(
        !trials.is_empty(),
        "all candidates failed: {}",
        failures.join("; ")
    );
    let min_size = trials
        .iter()
        .map(|t| t.metrics.physical_bytes_written.get())
        .min()
        .unwrap() as f64;
    let max_ingest = trials
        .iter()
        .map(|t| t.metrics.records_per_sec.get())
        .fold(0.0, f64::max);
    let max_decode = trials.iter().map(|t| t.decode_mb_s).fold(0.0, f64::max);
    let cost = |t: &ExperimentTrial| match goal {
        OptimizationGoal::MaxCompression => t.metrics.physical_bytes_written.get() as f64,
        OptimizationGoal::MaxIngest => -t.metrics.records_per_sec.get(),
        OptimizationGoal::MaxDecode => -t.decode_mb_s,
        OptimizationGoal::Balanced => {
            0.4 * t.metrics.physical_bytes_written.get() as f64 / min_size.max(1.0)
                + 0.4 * max_ingest / t.metrics.records_per_sec.get().max(1e-9)
                + 0.2 * max_decode / t.decode_mb_s.max(1e-9)
        }
    };
    let winner = (0..trials.len())
        .min_by(|a, b| cost(&trials[*a]).total_cmp(&cost(&trials[*b])))
        .unwrap();
    Ok(ExperimentReport{release_build:!cfg!(debug_assertions),sample_records:records.len(),options:options.clone(),profile_toml:trials[winner].profile_toml.clone(),reason:format!("{} selected by {} using actual closed dataset bytes, wall-clock ingest and equality-checked decode on {} observed records",trials[winner].name,goal.as_str(),records.len()),trials,failures,winner})
}
