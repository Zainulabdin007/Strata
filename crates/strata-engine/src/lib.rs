//! Shared local orchestration. Storage, codecs, queries and statistics remain in their engine crates.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use strata_codec::{compressor_for, resolve_profile_plans, CompressionCodec};
use strata_core::{CompressionProfile, DurabilityMode};
use strata_metrics::{MetricsHandle, MetricsSnapshot, StorageMetrics};
use strata_storage::{DatasetReader, DatasetWriterOptions, IngestPipeline, PipelineOptions};

/// Read bounded JSON/TOML metadata.
pub fn read_small(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        bail!("metadata exceeds 16 MiB limit");
    }
    Ok(String::from_utf8(bytes)?)
}
/// Atomic replacement for local reports (not a durability promise).
pub fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(value)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}
/// Settings shared by CLI and API ingest.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IngestOptions {
    pub workers: usize,
    pub parse_queue: Option<usize>,
    pub encode_queue: Option<usize>,
    pub durability: Option<String>,
    pub sync_every_blocks: Option<u32>,
    /// Resume only fully committed files, verifying their source fingerprints.
    pub resume: bool,
    /// Continue past per-file parse errors; write `ingest-errors.json` and keep going.
    pub continue_on_error: bool,
    /// Extra include globs (merged with profile).
    pub include_globs: Vec<String>,
    /// Extra exclude globs (merged with profile).
    pub exclude_globs: Vec<String>,
    /// Optional extension override.
    pub extensions: Vec<String>,
}
impl Default for IngestOptions {
    fn default() -> Self {
        Self {
            workers: 2,
            parse_queue: None,
            encode_queue: None,
            durability: None,
            sync_every_blocks: None,
            resume: false,
            continue_on_error: false,
            include_globs: Vec::new(),
            exclude_globs: Vec::new(),
            extensions: Vec::new(),
        }
    }
}

fn ingest_file(
    input: &Path,
    output: &Path,
    profile: &CompressionProfile,
    options: &IngestOptions,
    metrics: MetricsHandle,
) -> Result<strata_storage::DatasetManifest> {
    let schema = profile.schema()?;
    let mut opts = PipelineOptions::with_workers(options.workers);
    for cap in [options.parse_queue, options.encode_queue]
        .into_iter()
        .flatten()
    {
        if !(1..=128).contains(&cap) {
            bail!("queue capacity must be 1..128");
        }
    }
    opts.parse_queue_capacity = options.parse_queue.unwrap_or(opts.parse_queue_capacity);
    opts.encode_queue_capacity = options.encode_queue.unwrap_or(opts.encode_queue_capacity);
    let codec = CompressionCodec::parse_name(
        profile
            .compression
            .effective_codec()
            .map_err(anyhow::Error::msg)?
            .as_str(),
    )
    .context("unknown codec")?;
    opts.writer = DatasetWriterOptions {
        block_row_count: profile.block.row_count,
        segment_target_size_bytes: profile.segment.target_size_bytes,
        metrics: Some(metrics.clone()),
        encoding_plans: resolve_profile_plans(profile)?,
        compressor: Arc::from(compressor_for(codec, profile.compression.level)),
        durability: match options.durability.as_deref() {
            Some(s) => DurabilityMode::parse(s).context("invalid durability")?,
            None => profile.durability.mode,
        },
        sync_every_blocks: options
            .sync_every_blocks
            .unwrap_or(profile.durability.sync_every_blocks),
        key_fields: profile.key_fields().to_vec(),
    };
    let mut pipeline = IngestPipeline::start(output, schema.clone(), opts)?;
    let mut parser = strata_input::open_parser(profile)?;
    metrics.begin_input_file(input.display().to_string(), std::fs::metadata(input)?.len());
    let started = Instant::now();
    let parsed = parser.for_each_batch(
        input,
        &schema,
        profile.block.row_count as usize,
        &mut |batch| {
            metrics.add_records_read(batch.len() as u64);
            pipeline
                .submit(batch.records)
                .map_err(|e| strata_input::Error::MalformedCsv {
                    path: input.to_path_buf(),
                    line: 0,
                    message: e.to_string(),
                })
        },
    );
    metrics.add_parse_time(started.elapsed());
    if let Err(e) = parsed {
        drop(pipeline);
        return Err(e.into());
    }
    let manifest = pipeline.finish()?;
    metrics.finish_input_file();
    Ok(manifest)
}

mod folder;
pub use folder::ingest;

/// Storage inventory backed by parsed segment metadata and real file lengths.
pub fn inspect(dataset: &Path) -> Result<Value> {
    let reader = DatasetReader::open(dataset)?;
    let mut logical = 0u64;
    let mut blocks = 0u64;
    let mut segments = Vec::new();
    let mut columns: HashMap<String, (u64, u64)> = HashMap::new();
    let mut row_start = 0u64;
    for i in 0..reader.segment_count() {
        let segment = reader.open_segment(i)?;
        let physical = std::fs::metadata(segment.path())?.len();
        let mut segment_logical = 0;
        for block in &segment.meta().blocks {
            logical += block.logical_bytes;
            segment_logical += block.logical_bytes;
            blocks += 1;
            for col in &block.columns {
                let size = columns.entry(col.field_name.clone()).or_default();
                size.0 += col.logical_bytes;
                size.1 += col.encoded_bytes;
            }
        }
        let rows = segment.meta().header.record_count;
        segments.push(json!({
            "ordinal":i,
            "file":reader.manifest().segments[i].file,
            "metadata":segment.meta(),
            "metrics":StorageMetrics::observed(rows, segment_logical, physical, 1, segment.block_count() as u64),
            "record_start":row_start,
            "record_end_exclusive":row_start+rows,
            "health":"segment CRC verified; block decode not checked"
        }));
        row_start += rows;
    }
    let physical = storage_bytes(dataset)?;
    let column_sizes: Vec<Value> = reader
        .schema()
        .fields
        .iter()
        .map(|f| {
            let (raw, encoded) = columns.get(&f.name).copied().unwrap_or_default();
            json!({"name":f.name,"logical_type":f.logical_type,"logical_bytes":raw,"semantic_encoded_bytes":encoded})
        })
        .collect();
    Ok(json!({
        "manifest":reader.manifest(),
        "metrics":StorageMetrics::observed(reader.manifest().record_count,logical,physical,reader.segment_count() as u64,blocks),
        "segments":segments,
        "columns":column_sizes,
        "indexes_present":dataset.join("indexes").exists()
    }))
}

/// Count persisted engine files; report artifacts are excluded by definition.
pub fn storage_bytes(dataset: &Path) -> Result<u64> {
    let mut total = std::fs::metadata(dataset.join("manifest.json"))?.len();
    let mut dirs = vec![dataset.join("segments"), dataset.join("indexes")];
    while let Some(dir) = dirs.pop() {
        if !dir.exists() {
            continue;
        }
        for entry in std::fs::read_dir(dir)? {
            let e = entry?;
            let ty = e.file_type()?;
            if ty.is_dir() {
                dirs.push(e.path());
            } else if ty.is_file() {
                total += e.metadata()?.len();
            }
        }
    }
    Ok(total)
}

/// Analyze a bounded, explicitly labeled sample of a persisted dataset.
pub fn sample(dataset: &Path, limit: usize) -> Result<(DatasetReader, Vec<strata_core::Record>)> {
    let reader = DatasetReader::open(dataset)?;
    let mut records = Vec::new();
    let mut sample_bytes = 0u64;
    'segments: for i in 0..reader.segment_count() {
        let seg = reader.open_segment(i)?;
        for b in 0..seg.block_count() {
            let batch = seg.read_block(b)?;
            for record in batch.records.into_iter().take(limit - records.len()) {
                let bytes: u64 = reader
                    .schema()
                    .fields
                    .iter()
                    .map(|f| {
                        strata_core::logical_value_bytes(
                            &f.logical_type,
                            record
                                .get(&f.name)
                                .unwrap_or(&strata_core::FieldValue::Null),
                        )
                    })
                    .sum();
                if sample_bytes + bytes > 64 * 1024 * 1024 {
                    break 'segments;
                }
                sample_bytes += bytes;
                records.push(record);
            }
            if records.len() >= limit {
                break 'segments;
            }
        }
    }
    Ok((reader, records))
}

/// Run the real profiler over a bounded sample; never claim sample cardinality is full-dataset cardinality.
pub fn analyze(dataset: &Path) -> Result<Value> {
    let (reader, records) = sample(dataset, 65_536)?;
    let report = strata_analyze::AnalysisReport::from_records(
        dataset.display().to_string(),
        reader.schema(),
        &records,
        &Default::default(),
        reader.segment_count() as u64,
    )?;
    Ok(
        json!({"scope":"first up to 65536 stored records, capped at 64 MiB logical payload", "dataset_records":reader.manifest().record_count, "report":report}),
    )
}

/// Real optimizer on stored, decoded sample records.
pub fn optimize(dataset: &Path, profile: &CompressionProfile, goal: &str) -> Result<Value> {
    let (reader, records) = sample(dataset, 65_536)?;
    if profile.schema()? != *reader.schema() {
        bail!("profile schema does not match dataset");
    }
    let options = strata_optimize::OptimizeOptions {
        goal: strata_optimize::OptimizationGoal::parse(goal)
            .context("invalid optimization goal")?,
        ..Default::default()
    };
    let report = strata_optimize::optimize_dataset(
        dataset.display().to_string(),
        profile,
        &records,
        reader.segment_count() as u64,
        &options,
    )?;
    Ok(
        json!({"scope":"first up to 65536 stored records, capped at 64 MiB logical payload", "dataset_records":reader.manifest().record_count, "report":report}),
    )
}

/// Execute the existing typed query engine with a bounded response.
pub fn query(dataset: &Path, text: &str) -> Result<Value> {
    let mut query = strata_query::parse_query(text)?;
    query.limit = Some(query.limit.unwrap_or(1000).min(1000));
    let t = Instant::now();
    let mut stream = strata_query::execute(dataset, query)?;
    let mut rows = Vec::new();
    let mut bytes = 0;
    while let Some(batch) = stream.next_batch()? {
        for record in batch.records {
            let mut value = serde_json::to_value(record)?;
            preserve_integer_precision(&mut value);
            bytes += serde_json::to_vec(&value)?.len();
            anyhow::ensure!(
                bytes <= 8 * 1024 * 1024,
                "query response exceeds 8 MiB; use a smaller limit/projection"
            );
            rows.push(value);
        }
    }
    Ok(
        json!({"rows": rows, "stats":stream.stats(),"plan":stream.explain(),"elapsed_ms":t.elapsed().as_secs_f64()*1000.0,"response_limit":1000}),
    )
}

/// Canonical path confinement for local API capabilities.
pub fn confined(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = root.join(relative).canonicalize()?;
    if !path.starts_with(root) {
        bail!("path is outside configured root");
    }
    Ok(path)
}

/// Parse and validate the canonical TOML compression profile.
pub fn parse_profile(text: &str) -> Result<CompressionProfile> {
    Ok(strata_core::parse_profile_str(text, "<local-profile>")?)
}

/// Validated settings for real compression experiments.
pub use strata_optimize::experiment::ExperimentOptions;

/// Measure actual storage candidates against a decoded dataset sample.
pub fn experiment(
    dataset: &Path,
    profile: &CompressionProfile,
    goal: &str,
    options: &ExperimentOptions,
) -> Result<Value> {
    let (reader, records) = sample(dataset, 65_536)?;
    if profile.schema()? != *reader.schema() {
        bail!("profile schema differs from stored dataset");
    }
    let goal =
        strata_optimize::OptimizationGoal::parse(goal).context("invalid optimization goal")?;
    let scratch = dataset.join(format!(".experiment-{}", std::process::id()));
    std::fs::create_dir(&scratch)
        .context("experiment scratch already exists; inspect before removing")?;
    let result =
        strata_optimize::experiment::experiment(profile, &records, goal, options, &scratch);
    let cleanup = std::fs::remove_dir_all(&scratch);
    let report = result?;
    cleanup?;
    Ok(json!(report))
}

// JSON clients must not round exact 64-bit logical values through IEEE-754 doubles.
fn preserve_integer_precision(value: &mut Value) {
    match value {
        Value::Number(n)
            if n.as_u64().is_some_and(|n| n > 9_007_199_254_740_991)
                || n.as_i64().is_some_and(|n| n < -9_007_199_254_740_991) =>
        {
            *value = Value::String(n.to_string())
        }
        Value::Array(items) => items.iter_mut().for_each(preserve_integer_precision),
        Value::Object(fields) => fields.values_mut().for_each(preserve_integer_precision),
        _ => {}
    }
}
