//! Decode / full-scan suite.

use std::fs;
use std::time::Instant;

use anyhow::Result;
use serde_json::json;
use strata_storage::DatasetReader;

use crate::config::{gb_per_sec, ingest_streaming, per_sec, BenchConfig};
use crate::report::BenchReport;

pub fn run_decode(config: &BenchConfig) -> Result<BenchReport> {
    let root = config.dataset_path("decode");
    let _ = fs::remove_dir_all(&root);
    let _ = ingest_streaming(config, &root, false, None)?;

    let reader = DatasetReader::open_with_mode(&root, config.segment_read_mode)?;
    let t0 = Instant::now();
    let mut n = 0;
    for i in 0..reader.segment_count() {
        let seg = reader.open_segment(i)?;
        for b in 0..seg.block_count() {
            let batch = seg.read_block(b)?;
            n += batch.len() as u64;
            std::hint::black_box(batch);
        }
    }
    anyhow::ensure!(n == config.size.records(), "decode lost rows");
    let elapsed = t0.elapsed();

    let mut logical = 0u64;
    for i in 0..reader.segment_count() {
        let seg = reader.open_segment(i)?;
        for block in &seg.meta().blocks {
            logical += block.logical_bytes;
        }
    }

    let metrics = json!({
        "elapsed_ms": elapsed.as_millis() as u64,
        "records": n,
        "records_per_sec": per_sec(n, elapsed),
        "logical_bytes": logical,
        "logical_gb_per_sec": gb_per_sec(logical, elapsed),
        "segment_read_mode": config.segment_read_mode.as_str(),
    });

    Ok(BenchReport {
        suite: "decode".into(),
        meta: config.meta(),
        metrics,
    })
}
