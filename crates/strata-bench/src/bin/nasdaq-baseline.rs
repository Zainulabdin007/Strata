//! Real DBN MBO release baseline. All targets consume the same canonical records.
#![recursion_limit = "256"]
use anyhow::{ensure, Context, Result};
use clap::Parser;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Instant, SystemTime},
};
use strata_bench::baselines;
use strata_codec::{compressor_for, resolve_profile_plans, CompressionCodec};
use strata_core::{parse_profile_str, FieldValue, Record};
use strata_input::dbn::{canonical_bytes, mbo_profile, DbnStats, MboReader};
use strata_metrics::{MetricsHandle, StorageMetrics};
use strata_optimize::{
    experiment::{experiment, ExperimentOptions},
    OptimizationGoal,
};
use strata_storage::{DatasetReader, DatasetWriterOptions, IngestPipeline, PipelineOptions};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "benchmarks/nasdaq/sources")]
    input: PathBuf,
    #[arg(long, default_value = "benchmarks/nasdaq")]
    out: PathBuf,
    #[arg(long)]
    prepare: bool,
    #[arg(long, default_value = "validation")]
    tier: String,
    #[arg(long, default_value = "manual")]
    profile: String,
    #[arg(long, default_value = "strata")]
    target: String,
    #[arg(long, default_value_t = 2)]
    workers: usize,
    #[arg(long, default_value = "baseline")]
    tag: String,
}
fn files(input: &Path) -> Result<Vec<PathBuf>> {
    let profile = parse_profile_str(&mbo_profile(false), "DBN discovery")?;
    Ok(strata_input::discover_inputs(input, &profile)?.files)
}
fn save(path: impl AsRef<Path>, value: &impl serde::Serialize) -> Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
fn verify_source(a: &Args, source: &DbnStats) -> Result<()> {
    ensure!(source.complete, "source did not reach verified EOF");
    let inventory: serde_json::Value =
        serde_json::from_slice(&std::fs::read(a.out.join(if source.zstd {
            "source-inventory.json"
        } else {
            "uncompressed/verification.json"
        }))?)?;
    let entries = if source.zstd {
        &inventory["source_files"]
    } else {
        &inventory
    };
    let expected = entries
        .as_array()
        .context("source inventory missing")?
        .iter()
        .find(|f| {
            f[if source.zstd { "path" } else { "output" }]
                .as_str()
                .is_some_and(|p| Path::new(p) == source.source)
        })
        .context("source absent from frozen inventory")?;
    ensure!(
        expected[if source.zstd {
            "bytes"
        } else {
            "decompressed_bytes"
        }]
        .as_u64()
            == Some(source.source_file_bytes),
        "source size changed"
    );
    ensure!(
        expected[if source.zstd { "sha256" } else { "dbn_sha256" }].as_str()
            == source.source_sha256.as_deref(),
        "source hash differs from verified inventory"
    );
    let census: Vec<DbnStats> =
        serde_json::from_slice(&std::fs::read(a.out.join("source-census.json"))?)?;
    let original = census
        .iter()
        .find(|f| {
            f.source
                .file_name()
                .unwrap()
                .to_string_lossy()
                .trim_end_matches(".zst")
                == source
                    .source
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .trim_end_matches(".zst")
        })
        .context("source absent from census")?;
    ensure!(
        original.records == source.records
            && original.canonical_sha256 == source.canonical_sha256
            && original.logical_bytes == source.logical_bytes
            && original.dbn_bytes_accepted == source.dbn_bytes_accepted,
        "full source canonical census mismatch"
    );
    Ok(())
}
fn prepare(a: &Args, paths: &[PathBuf]) -> Result<()> {
    let root = a.out.join("profiles");
    std::fs::create_dir_all(&root)?;
    let mut sample = Vec::new();
    let mut windows = Vec::new();
    let mut schema = None;
    let sample_root = a.out.join("samples");
    ensure!(!sample_root.exists(), "frozen samples already exist");
    std::fs::create_dir_all(&sample_root)?;
    let mut full_sources = Vec::new();
    // First 4096 consecutive records after each fixed time boundary. Scan through
    // EOF to measure exact source totals; selection does not depend on compression.
    for (file, path) in paths.iter().enumerate() {
        let mut reader = MboReader::open(path)?;
        let current = reader.schema();
        if let Some(ref s) = schema {
            ensure!(s == &current, "mixed canonical MBO schemas");
        } else {
            schema = Some(current);
        }
        let metadata = reader.stats().metadata;
        let start: u64 = metadata["start"]
            .as_str()
            .context("missing start")?
            .parse()?;
        let end: u64 = metadata["end"].as_str().context("missing end")?.parse()?;
        ensure!(end > start, "invalid source time interval");
        let boundaries: Vec<_> = (0..4).map(|i| start + (end - start) / 4 * i).collect();
        let raw_metadata = reader.raw_metadata().to_vec();
        let ts_out = reader.schema().field("ts_out").is_some();
        std::fs::write(
            root.join(format!("source-{file}.metadata.dbn")),
            &raw_metadata,
        )?;
        let mut offset = 0u64;
        let mut window = 0usize;
        let mut selected = Vec::new();
        let mut window_begin = 0;
        let mut progress = Instant::now();
        loop {
            let rows = reader.next_batch(16384)?;
            if rows.is_empty() {
                break;
            }
            for row in rows {
                let ts = match row.get("ts_event") {
                    Some(FieldValue::UnsignedInteger(n)) => *n,
                    _ => anyhow::bail!("missing ts_event"),
                };
                if window < boundaries.len()
                    && (!selected.is_empty() || (ts != u64::MAX && ts >= boundaries[window]))
                {
                    if selected.is_empty() {
                        window_begin = offset;
                    }
                    selected.push(row);
                    if selected.len() == 4096 {
                        let sample_path =
                            sample_root.join(format!("source-{file}-window-{window}.dbn"));
                        let mut bytes = raw_metadata.clone();
                        for r in &selected {
                            bytes.extend(canonical_bytes(r, ts_out)?);
                        }
                        std::fs::write(&sample_path, bytes)?;
                        let times: Vec<u64> = selected
                            .iter()
                            .filter_map(|r| match r.get("ts_event") {
                                Some(FieldValue::UnsignedInteger(n)) if *n != u64::MAX => Some(*n),
                                _ => None,
                            })
                            .collect();
                        windows.push(json!({"source":path,"sample":sample_path,"boundary_ts_event":boundaries[window].to_string(),"start_record":window_begin,"records":selected.len(),"min_ts_event":times.iter().min().map(|n|n.to_string()),"max_ts_event":times.iter().max().map(|n|n.to_string())}));
                        sample.append(&mut selected);
                        window += 1;
                    }
                }
                offset += 1;
            }
            if progress.elapsed().as_secs() >= 5 {
                eprintln!(
                    "source census: file {}/{}, {offset} records; {window}/4 windows",
                    file + 1,
                    paths.len()
                );
                progress = Instant::now();
            }
        }
        ensure!(
            window == boundaries.len(),
            "source lacks records for all declared time windows"
        );
        let stats = reader.stats();
        verify_source(a, &stats)?;
        save(a.out.join(format!("source-{file}-census.json")), &stats)?;
        full_sources.push(stats);
    }
    save(a.out.join("source-census.json"), &full_sources)?;
    let schema = schema.context("no source schema")?;
    let ts_out = schema.field("ts_out").is_some();
    let text = mbo_profile(ts_out);
    let profile = parse_profile_str(&text, "MBO manual")?;
    std::fs::write(root.join("manual.toml"), &text)?;
    save(root.join("schema.json"), &schema)?;
    save(root.join("sampling.json"), &windows)?;
    let analysis = strata_analyze::AnalysisReport::from_records(
        "four 4096-record windows at source time quartiles per file; complete source census",
        &schema,
        &sample,
        &Default::default(),
        paths.len() as u64,
    )?;
    save(root.join("analysis.json"), &analysis)?;
    for goal in [
        OptimizationGoal::Balanced,
        OptimizationGoal::MaxCompression,
        OptimizationGoal::MaxIngest,
        OptimizationGoal::MaxDecode,
    ] {
        let scratch = a.out.join(format!("optimizer-{}", goal.as_str()));
        ensure!(!scratch.exists(), "optimizer scratch exists");
        std::fs::create_dir_all(&scratch)?;
        let report = experiment(
            &profile,
            &sample,
            goal,
            &ExperimentOptions {
                workers: Some(a.workers),
                ..Default::default()
            },
            &scratch,
        )?;
        ensure!(
            report.failures.is_empty(),
            "candidate failures: {:?}",
            report.failures
        );
        std::fs::write(
            root.join(format!("{}.toml", goal.as_str())),
            &report.profile_toml,
        )?;
        save(root.join(format!("{}.json", goal.as_str())), &report)?;
        std::fs::remove_dir_all(scratch)?;
        println!("PROFILE {}: {}", goal.as_str(), report.reason);
    }
    Ok(())
}
struct Validator<'a> {
    sources: &'a [DbnStats],
    file: usize,
    in_file: u64,
    total: u64,
    hash: Sha256,
    ts_out: bool,
}
impl<'a> Validator<'a> {
    fn new(sources: &'a [DbnStats], ts_out: bool) -> Self {
        Self {
            sources,
            file: 0,
            in_file: 0,
            total: 0,
            hash: Sha256::new(),
            ts_out,
        }
    }
    fn record(&mut self, r: &Record) -> Result<()> {
        while self.file < self.sources.len() && self.sources[self.file].records == 0 {
            ensure!(
                self.sources[self.file].canonical_sha256
                    == format!("{:x}", Sha256::new().finalize()),
                "empty source hash mismatch"
            );
            self.file += 1;
        }
        ensure!(
            self.file < self.sources.len(),
            "target contains extra records"
        );
        self.hash.update(canonical_bytes(r, self.ts_out)?);
        self.in_file += 1;
        self.total += 1;
        let expected = &self.sources[self.file];
        if self.in_file == expected.records {
            ensure!(
                format!("{:x}", self.hash.clone().finalize()) == expected.canonical_sha256,
                "lossless canonical SHA-256 mismatch in {}",
                expected.source.display()
            );
            self.hash = Sha256::new();
            self.file += 1;
            self.in_file = 0;
        }
        Ok(())
    }
    fn canonical_batch(&mut self, bytes: &[u8]) -> Result<()> {
        let width = if self.ts_out { 64 } else { 56 };
        ensure!(bytes.len() % width == 0, "unaligned canonical batch");
        let mut offset = 0;
        while offset < bytes.len() {
            while self.file < self.sources.len() && self.sources[self.file].records == 0 {
                self.file += 1;
            }
            let expected = self
                .sources
                .get(self.file)
                .context("extra decoded records")?;
            let rows =
                ((bytes.len() - offset) / width).min((expected.records - self.in_file) as usize);
            self.hash.update(&bytes[offset..offset + rows * width]);
            offset += rows * width;
            self.in_file += rows as u64;
            self.total += rows as u64;
            if self.in_file == expected.records {
                ensure!(
                    format!("{:x}", self.hash.clone().finalize()) == expected.canonical_sha256,
                    "canonical batch hash mismatch"
                );
                self.hash = Sha256::new();
                self.file += 1;
                self.in_file = 0;
            }
        }
        Ok(())
    }
    fn columns(&mut self, columns: &[Vec<FieldValue>], rows: usize) -> Result<()> {
        let width = if self.ts_out { 64 } else { 56 };
        ensure!(
            columns.len() == if self.ts_out { 16 } else { 15 },
            "canonical column count mismatch"
        );
        let mut bytes = vec![0u8; rows * width];
        let mut offset = 0;
        for ((name, signed, bits), column) in strata_input::dbn::MBO_FIELDS
            .iter()
            .copied()
            .chain(self.ts_out.then_some(("ts_out", false, 64)))
            .zip(columns)
        {
            ensure!(column.len() == rows, "column length mismatch");
            let n = usize::from(bits / 8);
            for (i, value) in column.iter().enumerate() {
                let word = match (value, signed) {
                    (FieldValue::SignedInteger(v), true)
                        if bits == 64
                            || (*v >= -(1i64 << (bits - 1)) && *v < (1i64 << (bits - 1))) =>
                    {
                        v.to_le_bytes()
                    }
                    (FieldValue::UnsignedInteger(v), false)
                        if bits == 64 || *v < (1u64 << bits) =>
                    {
                        v.to_le_bytes()
                    }
                    _ => anyhow::bail!("invalid canonical value in {name}"),
                };
                bytes[i * width + offset..i * width + offset + n].copy_from_slice(&word[..n]);
            }
            offset += n;
        }
        self.canonical_batch(&bytes)
    }
    fn finish(&self) -> Result<()> {
        ensure!(
            self.total == self.sources.iter().map(|s| s.records).sum::<u64>() && self.in_file == 0,
            "decoded record count mismatch"
        );
        Ok(())
    }
}
fn hash_projection(hash: &mut Sha256, record: &Record) -> Result<()> {
    for name in ["instrument_id", "ts_event", "price"] {
        match record.get(name) {
            Some(FieldValue::UnsignedInteger(n)) => hash.update(n.to_le_bytes()),
            Some(FieldValue::SignedInteger(n)) => hash.update(n.to_le_bytes()),
            _ => anyhow::bail!("missing projected field {name}"),
        }
    }
    Ok(())
}
fn main() -> Result<()> {
    ensure!(
        !cfg!(debug_assertions),
        "real-data benchmark requires --release"
    );
    let a = Args::parse();
    ensure!((1..=32).contains(&a.workers), "workers must be 1..32");
    let paths = files(&a.input)?;
    ensure!(!paths.is_empty(), "no source files");
    std::fs::create_dir_all(&a.out)?;
    if a.prepare {
        return prepare(&a, &paths);
    }
    ensure!(
        ["strata", "parquet", "fixed"].contains(&a.target.as_str()),
        "unknown target"
    );
    let first = MboReader::open(&paths[0])?;
    let schema = first.schema();
    let ts_out = schema.field("ts_out").is_some();
    drop(first);
    let record_bytes = if ts_out { 64u64 } else { 56 };
    let limit = match a.tier.as_str() {
        "validation" => Some(100_000),
        "1gb" => Some(1_000_000_000 / paths.len() as u64 / record_bytes),
        "10gb" => Some(10_000_000_000 / paths.len() as u64 / record_bytes),
        "full" | "windows" => None,
        _ => anyhow::bail!("tier must be validation, 1gb, 10gb, full, windows"),
    };
    let label = if a.target == "strata" {
        a.profile.clone()
    } else {
        a.target.clone()
    };
    let name = format!("{}-{}-{}-w{}", a.tag, a.tier, label, a.workers);
    let root = a.out.join("datasets").join(&name);
    ensure!(!root.exists(), "output exists; use a fresh --tag");
    std::fs::create_dir_all(&root)?;
    let profiles = a.out.join("profiles");
    let profile_text = std::fs::read_to_string(profiles.join(format!("{}.toml", a.profile)))?;
    let profile = parse_profile_str(&profile_text, "frozen MBO profile")?;
    ensure!(
        profile.schema()? == schema,
        "profile drops or changes MBO fields"
    );
    let metrics = MetricsHandle::new();
    let start = Instant::now();
    let wall_start = SystemTime::now();
    let mut pipe = None;
    let mut parquet = None;
    let mut fixed = None;
    match a.target.as_str() {
        "strata" => {
            let codec = CompressionCodec::parse_name(
                profile
                    .compression
                    .effective_codec()
                    .map_err(anyhow::Error::msg)?
                    .as_str(),
            )
            .context("codec")?;
            let mut opts = PipelineOptions::with_workers(a.workers);
            opts.writer = DatasetWriterOptions {
                block_row_count: profile.block.row_count,
                segment_target_size_bytes: profile.segment.target_size_bytes,
                encoding_plans: resolve_profile_plans(&profile)?,
                compressor: Arc::from(compressor_for(codec, profile.compression.level)),
                metrics: Some(metrics.clone()),
                durability: profile.durability.mode,
                ..Default::default()
            };
            pipe = Some(IngestPipeline::start(
                root.join("strata"),
                schema.clone(),
                opts,
            )?);
        }
        "parquet" => {
            parquet = Some(baselines::parquet_baseline::ParquetZstdWriter::create(
                &root.join("data.parquet"),
                &schema,
            )?)
        }
        "fixed" => {
            fixed = Some(baselines::fixed::FixedWriter::create(
                &root.join("data.fixed"),
                &schema,
            )?)
        }
        _ => unreachable!(),
    }
    let mut sources = Vec::new();
    let mut expected_query = Vec::new();
    let mut records = 0u64;
    let mut filter_instrument = None;
    let mut expected_filter_hash = Sha256::new();
    let mut expected_filter_count = 0u64;
    let mut last_progress = Instant::now();
    metrics.set_total_files(paths.len() as u64);
    for (index, path) in paths.iter().enumerate() {
        let before = std::fs::metadata(path)?;
        let mut reader = MboReader::open(path)?;
        ensure!(reader.schema() == schema, "source schema mismatch");
        std::fs::write(
            root.join(format!("source-{index}.metadata.dbn")),
            reader.raw_metadata(),
        )?;
        metrics.begin_input_file(path.display().to_string(), before.len());
        let mut count = 0u64;
        loop {
            let n = limit.map_or(u64::from(profile.block.row_count), |limit| {
                (limit - count).min(u64::from(profile.block.row_count))
            }) as usize;
            if n == 0 {
                break;
            }
            let t = Instant::now();
            let rows = if n <= 65536 {
                reader.next_batch(n)?
            } else {
                let mut rows = Vec::with_capacity(n);
                while rows.len() < n {
                    let mut next = reader.next_batch((n - rows.len()).min(65536))?;
                    if next.is_empty() {
                        break;
                    }
                    rows.append(&mut next);
                }
                rows
            };
            metrics.add_parse_time(t.elapsed());
            if rows.is_empty() {
                break;
            }
            count += rows.len() as u64;
            records += rows.len() as u64;
            metrics.add_records_read(rows.len() as u64);
            if expected_query.len() < 1000 {
                expected_query.extend(rows.iter().take(1000 - expected_query.len()).cloned());
            }
            for row in &rows {
                let instrument = match row.get("instrument_id") {
                    Some(FieldValue::UnsignedInteger(n)) => *n,
                    _ => anyhow::bail!("missing instrument_id"),
                };
                let selected = *filter_instrument.get_or_insert(instrument);
                if instrument == selected {
                    hash_projection(&mut expected_filter_hash, row)?;
                    expected_filter_count += 1;
                }
            }
            if let Some(ref mut p) = pipe {
                p.submit(rows)?;
            } else {
                if let Some(ref mut w) = parquet {
                    w.write_chunk(&rows)?;
                }
                if let Some(ref mut w) = fixed {
                    w.write_chunk(&rows)?;
                }
                metrics.with_mut(|m| {
                    m.add_records_processed(rows.len() as u64);
                    m.add_logical_bytes(rows.len() as u64 * record_bytes);
                });
            }
            if last_progress.elapsed().as_secs() >= 5 {
                eprintln!(
                    "{name}: file {}/{}, {} records",
                    index + 1,
                    paths.len(),
                    records
                );
                last_progress = Instant::now();
            }
        }
        if let Some(limit) = limit {
            ensure!(
                count == limit,
                "source ended before deterministic tier boundary"
            );
        }
        let source = reader.stats();
        if a.tier == "full" {
            verify_source(&a, &source)?;
        }
        let after = std::fs::metadata(path)?;
        ensure!(
            before.len() == after.len() && before.modified()? == after.modified()?,
            "source changed during run"
        );
        sources.push(source);
        metrics.finish_input_file();
    }
    let submit_seconds = start.elapsed().as_secs_f64();
    let drain_start = Instant::now();
    if let Some(p) = pipe {
        p.finish()?;
    }
    if let Some(w) = parquet {
        ensure!(w.finish()? == records, "Parquet lost records");
    }
    if let Some(w) = fixed {
        ensure!(w.finish()? == records, "fixed lost records");
    }
    let drain_seconds = drain_start.elapsed().as_secs_f64();
    let physical = match a.target.as_str() {
        "strata" => baselines::dir_size(&root.join("strata"))?,
        "parquet" => std::fs::metadata(root.join("data.parquet"))?.len(),
        _ => std::fs::metadata(root.join("data.fixed"))?.len(),
    };
    metrics.with_mut(|m| m.set_physical_bytes(physical));
    let snapshot = metrics.finish();
    let encode_seconds = start.elapsed().as_secs_f64();
    let encode_wall_seconds = wall_start.elapsed()?.as_secs_f64();
    ensure!(
        snapshot.records_processed == records
            && snapshot.logical_input_bytes.get()
                == sources.iter().map(|s| s.logical_bytes).sum::<u64>(),
        "canonical accounting mismatch"
    );
    // Persist measured ingest immediately; incomplete decode is never a successful report.
    save(
        root.join("ingest.json"),
        &json!({"status":"awaiting full decode validation","sources":sources,"canonical_metrics":snapshot,"encode_seconds":encode_seconds,"encode_wall_seconds":encode_wall_seconds}),
    )?;
    eprintln!("{name}: ingest complete, verifying every decoded record");
    let t = Instant::now();
    let decode_wall = SystemTime::now();
    let mut validator = Validator::new(&sources, ts_out);
    let mut block_row_histogram = std::collections::BTreeMap::<u32, u64>::new();
    match a.target.as_str() {
        "strata" => {
            let reader = DatasetReader::open(root.join("strata"))?;
            for s in 0..reader.segment_count() {
                let seg = reader.open_segment(s)?;
                for b in 0..seg.block_count() {
                    let (meta, columns) = seg.read_block_columns(b)?;
                    *block_row_histogram.entry(meta.row_count).or_default() += 1;
                    validator.columns(&columns, meta.row_count as usize)?;
                    std::hint::black_box(columns);
                }
            }
        }
        "parquet" => {
            baselines::parquet_baseline::decode_parquet_integer_bytes(
                &root.join("data.parquet"),
                &schema,
                &mut |bytes| validator.canonical_batch(bytes),
            )?;
        }
        "fixed" => {
            baselines::fixed::decode_fixed_records(&root.join("data.fixed"), &schema, &mut |r| {
                validator.record(&r)
            })?;
        }
        _ => unreachable!(),
    }
    validator.finish()?;
    let verified = validator.total;
    let decode_seconds = t.elapsed().as_secs_f64();
    let decode_wall_seconds = decode_wall.elapsed()?.as_secs_f64();
    // Separate count-only scan: all columns decoded and consumed, no record
    // reconstruction or SHA hashing. Validation above remains mandatory.
    let scan_start = Instant::now();
    let scan_wall = SystemTime::now();
    let scan_records = match a.target.as_str() {
        "strata" => Some(DatasetReader::open(root.join("strata"))?.scan_columns(
            |meta, columns| {
                std::hint::black_box((meta, columns));
                Ok(())
            },
        )?),
        "parquet" => Some(baselines::parquet_baseline::scan_parquet_columns(
            &root.join("data.parquet"),
        )?),
        _ => None,
    };
    let columnar_scan = if let Some(count) = scan_records {
        ensure!(count == records, "columnar scan count mismatch");
        let seconds = scan_start.elapsed().as_secs_f64();
        let wall = scan_wall.elapsed()?.as_secs_f64();
        Some(
            json!({"records":count,"seconds":seconds,"wall_seconds":wall,
            "records_per_sec":count as f64 / seconds,"logical_gb_s":count as f64 * record_bytes as f64 / 1e9 / seconds,
            "timing_valid":(seconds-wall).abs()<seconds*0.05+1.0,
            "method":"all columns decoded; arrays consumed with black_box; bounded batches; CRC/type checks enabled; no rows or hash"}),
        )
    } else {
        None
    };
    let parallel_start = Instant::now();
    let parallel_wall = SystemTime::now();
    let parallel_records = match a.target.as_str() {
        "strata" => Some(
            DatasetReader::open(root.join("strata"))?.scan_columns_parallel(
                a.workers,
                |meta, columns| {
                    std::hint::black_box((meta, columns));
                    Ok(())
                },
            )?,
        ),
        "parquet" => Some(baselines::parquet_baseline::scan_parquet_columns_parallel(
            &root.join("data.parquet"),
            a.workers,
        )?),
        _ => None,
    };
    let parallel_columnar_scan = if let Some(count) = parallel_records {
        ensure!(count == records, "parallel scan count mismatch");
        let seconds = parallel_start.elapsed().as_secs_f64();
        let wall = parallel_wall.elapsed()?.as_secs_f64();
        Some(
            json!({"records":count,"seconds":seconds,"wall_seconds":wall,"workers":a.workers,
            "records_per_sec":count as f64 / seconds,"logical_gb_s":count as f64 * record_bytes as f64 / 1e9 / seconds,
            "timing_valid":(seconds-wall).abs()<seconds*0.05+1.0,
            "method":"bounded parallel all-column scan; Strata blocks / Parquet row groups; no rows or hash; checks enabled"}),
        )
    } else {
        None
    };
    let mut query = None;
    let mut scan_query = None;
    if a.target == "strata" {
        let t = Instant::now();
        let mut stream = strata_query::execute(
            root.join("strata"),
            strata_query::parse_query("select * limit 1000")?,
        )?;
        let rows = stream.collect_records()?;
        let elapsed = t.elapsed().as_secs_f64();
        ensure!(rows == expected_query, "query mismatch");
        query = Some(
            json!({"text":"select * limit 1000","records":rows.len(),"elapsed_seconds":elapsed,"stats":stream.stats(),"verified_against_source":true}),
        );
    }
    if a.target == "strata" {
        let text = format!(
            "select instrument_id, ts_event, price where instrument_id == {}",
            filter_instrument.context("no query instrument")?
        );
        eprintln!("{name}: verifying unbounded filtered query against source hash");
        let t = Instant::now();
        let wall = SystemTime::now();
        let mut stream =
            strata_query::execute(root.join("strata"), strata_query::parse_query(&text)?)?;
        let mut hash = Sha256::new();
        let mut count = 0u64;
        while let Some(batch) = stream.next_batch()? {
            for row in &batch.records {
                hash_projection(&mut hash, row)?;
                count += 1;
            }
        }
        let elapsed = t.elapsed().as_secs_f64();
        let wall_elapsed = wall.elapsed()?.as_secs_f64();
        ensure!(
            count == expected_filter_count && hash.finalize() == expected_filter_hash.finalize(),
            "full filtered query differs from source records"
        );
        scan_query = Some(
            json!({"text":text,"records":count,"elapsed_seconds":elapsed,"wall_seconds":wall_elapsed,"timing_valid":(elapsed-wall_elapsed).abs()<elapsed*0.05+1.0,"stats":stream.stats(),"verified_against_source":true,"indexes_built":false}),
        );
    }
    let mut instruments = BTreeSet::new();
    for s in &sources {
        instruments.extend(s.instrument_ids.iter().copied());
    }
    let logical = snapshot.logical_input_bytes.get();
    let sizes = StorageMetrics::observed(
        records,
        logical,
        physical,
        snapshot.segments_completed,
        snapshot.blocks_completed,
    );
    let timing_valid = parallel_columnar_scan
        .as_ref()
        .is_none_or(|q| q["timing_valid"] == true)
        && columnar_scan
            .as_ref()
            .is_none_or(|q| q["timing_valid"] == true)
        && (encode_wall_seconds - encode_seconds).abs() < encode_seconds * 0.05 + 1.0
        && (decode_wall_seconds - decode_seconds).abs() < decode_seconds * 0.05 + 1.0
        && scan_query
            .as_ref()
            .is_none_or(|q| q["timing_valid"] == true);
    let vendor_inventory: serde_json::Value =
        serde_json::from_slice(&std::fs::read(a.out.join("source-inventory.json"))?)?;
    let vendor_bytes = vendor_inventory["vendor_transport_bytes"]
        .as_u64()
        .context("missing vendor size")?;
    let report = json!({"release_build":true,"corpus":"Databento XNAS.ITCH MBO","tier":a.tier,"profile":label,"target":a.target,"tag":a.tag,"records":records,"verified_records":verified,"verification":"per-source ordered canonical SHA-256; every field reconstructed; Strata/Parquet validation uses bounded column arrays, fixed uses bounded rows","timing_valid":timing_valid,"canonical_schema":schema,"sources":sources,"source_order":paths,"records_per_source_limit":limit,"effective_block_row_target":profile.block.row_count,"block_row_histogram":block_row_histogram,"instrument_count":instruments.len(),"min_ts_event":sources.iter().filter_map(|s|s.min_ts_event).min().map(|n|n.to_string()),"max_ts_event":sources.iter().filter_map(|s|s.max_ts_event).max().map(|n|n.to_string()),"dbn_bytes_observed":sources.iter().map(|s|s.decompressed_bytes_observed).sum::<u64>(),"dbn_bytes_accepted":sources.iter().map(|s|s.dbn_bytes_accepted).sum::<u64>(),"shared_dbn_metadata_bytes":sources.iter().map(|s|s.metadata_bytes).sum::<u64>(),"target_plus_dbn_metadata_bytes":physical+sources.iter().map(|s|s.metadata_bytes).sum::<u64>(),"storage_accounting_note":"Target physical bytes include target schema and framing; exact original DBN metadata is retained as shared sidecars, reported separately and identically for all targets. Original transport files are preserved outside target size.","vendor_transport_full_file_bytes":vendor_bytes,"vendor_transport_note":"Original full vendor files; do not compare their full sizes to a partial tier", "sizes":sizes,"canonical_metrics":snapshot,"semantic_encoded_bytes":if a.target=="strata"{Some(snapshot.semantic_encoded_bytes.get())}else{None},"phase_metrics_note":"Semantic encoding/compression phase counters are instrumented for Strata only; zero default phase fields in baseline snapshots mean unavailable, not measured zero work.","profile_toml":profile_text,"source_representation":if sources.iter().all(|s|s.zstd){"streaming vendor DBN.zst"}else{"verified uncompressed DBN"},"submit_seconds":submit_seconds,"drain_seconds":drain_seconds,"columnar_scan":columnar_scan,"parallel_columnar_scan":parallel_columnar_scan,"encode_seconds":encode_seconds,"encode_wall_seconds":encode_wall_seconds,"decode_seconds":decode_seconds,"decode_wall_seconds":decode_wall_seconds,"query":query,"scan_query":scan_query,
 "metrics":{"compression_ratio":sizes.compression_ratio,"bits_per_value":physical as f64*8.0/(records as f64*schema.fields.len() as f64),"records_per_sec":snapshot.records_per_sec.get(),"logical_gb_s":snapshot.logical_mb_per_sec.get()/1000.0,"disk_mb_s":snapshot.physical_mb_per_sec.get(),"decode_records_per_sec":records as f64/decode_seconds,"decode_gb_s":logical as f64/1e9/decode_seconds,"decode_mb_s":logical as f64/1e6/decode_seconds,"query_ms":scan_query.as_ref().map(|q|q["elapsed_seconds"].as_f64().unwrap()*1000.0),"workers":if a.target=="strata"{Some(a.workers)}else{None}}});
    std::fs::create_dir_all(a.out.join("results"))?;
    save(a.out.join("results").join(format!("{name}.json")), &report)?;
    ensure!(
        timing_valid,
        "clock discrepancy: possible sleep; retained sizes and validation, timings must be rerun"
    );
    println!("PASS {name}: {records} records, {logical} logical bytes, {physical} physical bytes; every record verified");
    Ok(())
}
