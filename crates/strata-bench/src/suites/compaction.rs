//! Compaction suite for keyed datasets.

use std::fs;
use std::time::Instant;

use anyhow::Result;
use serde_json::json;
use strata_storage::{
    compact_dataset, CompactOptions, DatasetAppender, DatasetAppenderOptions, DatasetReader,
};

use crate::config::{ingest_streaming, BenchConfig};
use crate::gen::{generate_chunk, Workload};
use crate::report::BenchReport;

pub fn run_compaction(config: &BenchConfig) -> Result<BenchReport> {
    let root = config.dataset_path("compaction");
    let _ = fs::remove_dir_all(&root);

    // Keyed ingest.
    let _ = ingest_streaming(config, &root, true, None)?;

    // Append updates + deletes (bounded, deterministic).
    let mut app = DatasetAppender::open(&root, DatasetAppenderOptions::default())?;
    let mut buf = Vec::new();
    let mutate_n = config.size.records().min(10_000);
    generate_chunk(
        config.workload,
        config.seed ^ 0xC04D_AC70,
        0,
        mutate_n,
        &mut buf,
    );
    for (i, rec) in buf.iter().enumerate() {
        if i % 3 == 0 {
            app.update(rec.clone())?;
        } else if i % 7 == 0 {
            app.delete(rec)?;
        }
    }
    app.finish()?;

    let before = DatasetReader::open(&root)?;
    let physical_before = before.manifest().record_count;
    let logical_before = before.read_latest_records()?.len() as u64;

    let t0 = Instant::now();
    let report = compact_dataset(
        &root,
        CompactOptions::default().with_compression(config.codec, config.codec_level),
    )?;
    let elapsed = t0.elapsed();

    let after = DatasetReader::open(&root)?;
    let logical_after = after.read_latest_records()?.len() as u64;

    let metrics = json!({
        "elapsed_ms": elapsed.as_millis() as u64,
        "physical_records_before": physical_before,
        "logical_records_before": logical_before,
        "logical_records_after": logical_after,
        "physical_records_after": after.manifest().record_count,
        "segments_after": report.segment_count,
        "compaction_generation": report.compaction_generation,
        "indexes_rebuilt": report.indexes_rebuilt,
        "logical_state_preserved": logical_before == logical_after,
        "workload_keys": match config.workload {
            Workload::Market => "order_id",
            Workload::Telemetry => "device_id+metric",
            Workload::Logs => "request_id",
        },
    });

    Ok(BenchReport {
        suite: "compaction".into(),
        meta: config.meta(),
        metrics,
    })
}
