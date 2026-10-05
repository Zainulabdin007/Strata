//! Reproducible release baseline. Refuses debug builds and verifies every Strata row.
use anyhow::{ensure, Result};
use clap::Parser;
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::Instant};
use strata_bench::{
    baselines,
    gen::{generate_chunk, Workload},
};
use strata_codec::{compressor_for, resolve_profile_plans, CompressionCodec};
use strata_core::{parse_profile_str, FieldValue, Record};
use strata_metrics::{MetricsHandle, StorageMetrics};
use strata_storage::{DatasetReader, DatasetWriterOptions, IngestPipeline, PipelineOptions};
#[derive(Parser)]
struct Args {
    #[arg(long)]
    prepare: bool,
    #[arg(long, default_value_t = 100_000)]
    records: u64,
    #[arg(long, default_value_t = 2)]
    workers: usize,
    #[arg(long, default_value = "manual")]
    profile: String,
    #[arg(long, default_value = "strata")]
    format: String,
    #[arg(long, default_value = "benchmarks/verified")]
    out: PathBuf,
    #[arg(long, default_value = "benchmarks/work")]
    work: PathBuf,
    #[arg(long, default_value = "baseline")]
    tag: String,
}
const BASE: &str = include_str!("../../../../examples/profiles/benchmark-market.toml");
fn main() -> Result<()> {
    ensure!(
        !cfg!(debug_assertions),
        "baseline requires --release; no work was measured"
    );
    let a = Args::parse();
    ensure!(
        a.records > 0 && (1..=32).contains(&a.workers),
        "invalid records/workers"
    );
    std::fs::create_dir_all(&a.out)?;
    let profiles = a.out.join("profiles");
    std::fs::create_dir_all(&profiles)?;
    if a.prepare {
        std::fs::write(profiles.join("manual.toml"), BASE)?;
        let plain = BASE
            .lines()
            .map(|l| {
                if l.starts_with("encoding =") {
                    "encoding = [\"plain\"]"
                } else if l == "codec = \"zstd\"" {
                    "codec = \"none\""
                } else {
                    l
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(profiles.join("plain.toml"), plain)?;
        let base = parse_profile_str(BASE, "baseline")?;
        let mut sample = Vec::new();
        generate_chunk(Workload::Market, 42, 0, 16_384, &mut sample);
        for goal in [
            strata_optimize::OptimizationGoal::Balanced,
            strata_optimize::OptimizationGoal::MaxCompression,
            strata_optimize::OptimizationGoal::MaxIngest,
        ] {
            let scratch = a.work.join(format!("prepare-{}", goal.as_str()));
            ensure!(
                !scratch.exists(),
                "prepare scratch already exists: {}",
                scratch.display()
            );
            std::fs::create_dir_all(&scratch)?;
            let report = strata_optimize::experiment::experiment(
                &base,
                &sample,
                goal,
                &strata_optimize::experiment::ExperimentOptions {
                    workers: Some(a.workers),
                    ..Default::default()
                },
                &scratch,
            )?;
            ensure!(
                report.failures.is_empty(),
                "optimizer candidate failures: {:?}",
                report.failures
            );
            std::fs::write(
                profiles.join(format!("{}.toml", goal.as_str())),
                &report.profile_toml,
            )?;
            std::fs::write(
                profiles.join(format!("{}.json", goal.as_str())),
                serde_json::to_vec_pretty(&report)?,
            )?;
            std::fs::remove_dir_all(scratch)?;
        }
        return Ok(());
    }
    let schema = Workload::Market.schema();
    let label = if a.format == "strata" {
        a.profile.clone()
    } else {
        a.format.clone()
    };
    let name = format!("{}-{}-{}-w{}", a.tag, label, a.records, a.workers);
    let root = a.work.join(&name);
    ensure!(
        !root.exists(),
        "benchmark output exists; use a fresh --tag: {}",
        root.display()
    );
    std::fs::create_dir_all(&root)?;
    let dataset = root.join("dataset");
    let source = root.join("data");
    let metrics = MetricsHandle::new();
    let timer = Instant::now();
    let mut logical = 0u64;
    let mut buf = Vec::new();
    let mut feed = |write: &mut dyn FnMut(&[Record]) -> Result<()>| -> Result<()> {
        let mut start = 0;
        while start < a.records {
            let n = 65_536.min(a.records - start);
            generate_chunk(Workload::Market, 42, start, n, &mut buf);
            for rec in &buf {
                for field in &schema.fields {
                    logical += strata_core::logical_value_bytes(
                        &field.logical_type,
                        rec.get(&field.name).unwrap(),
                    );
                }
            }
            write(&buf)?;
            start += n;
        }
        Ok(())
    };
    let mut profile_text = None;
    match a.format.as_str() {
        "strata" => {
            let text = std::fs::read_to_string(profiles.join(format!("{}.toml", a.profile)))?;
            let p = parse_profile_str(&text, "baseline profile")?;
            let codec = CompressionCodec::parse_name(
                p.compression
                    .effective_codec()
                    .map_err(anyhow::Error::msg)?
                    .as_str(),
            )
            .unwrap();
            let mut opts = PipelineOptions::with_workers(a.workers);
            opts.writer = DatasetWriterOptions {
                block_row_count: p.block.row_count,
                segment_target_size_bytes: p.segment.target_size_bytes,
                encoding_plans: resolve_profile_plans(&p)?,
                compressor: Arc::from(compressor_for(codec, p.compression.level)),
                metrics: Some(metrics.clone()),
                durability: p.durability.mode,
                ..Default::default()
            };
            let mut pipe = IngestPipeline::start(&dataset, schema.clone(), opts)?;
            // Move each bounded chunk into the pipeline: no clone of every record.
            let mut start = 0;
            while start < a.records {
                let n = u64::from(p.block.row_count).min(a.records - start);
                let mut rows = Vec::new();
                generate_chunk(Workload::Market, 42, start, n, &mut rows);
                pipe.submit(rows)?;
                start += n;
            }
            pipe.finish()?;
            profile_text = Some(text);
        }
        "jsonl" => {
            let mut w = baselines::jsonl::JsonlWriter::create(&source)?;
            feed(&mut |rows| w.write_chunk(&schema, rows))?;
            w.finish()?;
        }
        "fixed" => {
            let mut w = baselines::fixed::FixedWriter::create(&source, &schema)?;
            feed(&mut |rows| w.write_chunk(rows))?;
            w.finish()?;
        }
        #[cfg(feature = "parquet")]
        "parquet" => {
            let mut w = baselines::parquet_baseline::ParquetZstdWriter::create(&source, &schema)?;
            feed(&mut |rows| w.write_chunk(rows))?;
            w.finish()?;
        }
        _ => anyhow::bail!("unsupported format (Parquet requires its cargo feature)"),
    }
    let physical = if a.format == "strata" {
        baselines::dir_size(&dataset)?
    } else {
        std::fs::metadata(&source)?.len()
    };
    if a.format != "strata" {
        metrics.with_mut(|m| {
            m.add_records_processed(a.records);
            m.add_logical_bytes(logical);
            m.set_physical_bytes(physical);
        });
    }
    let snapshot = metrics.finish();
    let encode_secs = timer.elapsed().as_secs_f64();
    ensure!(
        snapshot.records_processed == a.records
            && snapshot.physical_bytes_written.get() == physical,
        "canonical accounting mismatch"
    );
    logical = snapshot.logical_input_bytes.get();
    let decode_timer = Instant::now();
    let decoded = if a.format == "strata" {
        let reader = DatasetReader::open(&dataset)?;
        let mut count = 0;
        for s in 0..reader.segment_count() {
            let seg = reader.open_segment(s)?;
            for b in 0..seg.block_count() {
                let batch = seg.read_block(b)?;
                count += batch.len() as u64;
                std::hint::black_box(batch);
            }
        }
        count
    } else {
        match a.format.as_str() {
            "jsonl" => baselines::jsonl::decode_jsonl(&source)?.0,
            "fixed" => baselines::fixed::decode_fixed_typed(&source, &schema)?.0,
            #[cfg(feature = "parquet")]
            "parquet" => baselines::parquet_baseline::decode_parquet_zstd(&source)?.0,
            _ => unreachable!(),
        }
    };
    let decode_secs = decode_timer.elapsed().as_secs_f64();
    ensure!(
        decoded == a.records,
        "decoded {decoded}, expected {}",
        a.records
    );
    let mut query_result = None;
    let mut verified = 0u64;
    if a.format == "strata" {
        // Outside decode timer: equality-check every field of every decoded record.
        let reader = DatasetReader::open(&dataset)?;
        let mut expected = Vec::new();
        for s in 0..reader.segment_count() {
            let seg = reader.open_segment(s)?;
            for b in 0..seg.block_count() {
                let batch = seg.read_block(b)?;
                generate_chunk(
                    Workload::Market,
                    42,
                    verified,
                    batch.len() as u64,
                    &mut expected,
                );
                ensure!(
                    batch.records == expected,
                    "lossless verification failed at {verified}"
                );
                verified += batch.len() as u64;
            }
        }
        ensure!(verified == a.records, "verification lost rows");
        let low = a.records / 2;
        let high = (low + 999).min(a.records - 1);
        let text = format!("select sequence where sequence between {low} and {high}");
        let t = Instant::now();
        let mut stream = strata_query::execute(&dataset, strata_query::parse_query(&text)?)?;
        let rows = stream.collect_records()?;
        let elapsed = t.elapsed().as_secs_f64() * 1000.0;
        ensure!(rows.len() as u64 == high - low + 1, "query count mismatch");
        for (i, row) in rows.iter().enumerate() {
            ensure!(
                row.get("sequence") == Some(&FieldValue::UnsignedInteger(low + i as u64)),
                "wrong query value"
            );
        }
        query_result = Some(
            json!({"text":text,"rows":rows.len(),"first":low,"last":high,"elapsed_ms":elapsed,"stats":stream.stats()}),
        );
    }
    let sizes = StorageMetrics::observed(
        a.records,
        logical,
        physical,
        snapshot.segments_completed,
        snapshot.blocks_completed,
    );
    let report = json!({"release_build":true,"generator":"market-v2","seed":42,"profile":label,"records":a.records,"format":a.format,"tag":a.tag,
        "canonical_metrics":snapshot,"sizes":sizes,"encode_wall_seconds":encode_secs,"decode_wall_seconds":decode_secs,"decoded_records":decoded,"verified_records":verified,"query":query_result,"profile_toml":profile_text,
        "metrics":{"compression_ratio":sizes.compression_ratio,"records_per_sec":snapshot.records_per_sec.get(),"logical_gb_s":snapshot.logical_mb_per_sec.get()/1000.0,"disk_mb_s":snapshot.physical_mb_per_sec.get(),"encode_mb_s":snapshot.encode_mb_per_sec.get(),"decode_mb_s":logical as f64/1e6/decode_secs,"query_ms":query_result.as_ref().map(|q|q["elapsed_ms"].clone()),"workers":a.workers}});
    std::fs::write(
        a.out.join(format!("{name}.json")),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "{name}: {physical} bytes, {} rows decoded, {verified} rows losslessly verified",
        a.records
    );
    Ok(())
}
