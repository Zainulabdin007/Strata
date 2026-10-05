//! Ingest throughput suite.

use std::fs;

use anyhow::Result;
use serde_json::json;
use strata_metrics::MetricsHandle;

use crate::config::{gb_per_sec, ingest_streaming, mb_per_sec, per_sec, BenchConfig};
use crate::report::BenchReport;

pub fn run_ingest(config: &BenchConfig) -> Result<BenchReport> {
    let root = config.dataset_path("ingest");
    let _ = fs::remove_dir_all(&root);
    let metrics = MetricsHandle::new();
    let elapsed = ingest_streaming(config, &root, false, Some(metrics.clone()))?;
    let snap = metrics.finish();

    let records = config.size.records();
    let logical = snap.logical_input_bytes.get();
    let physical = snap.physical_bytes_written.get();

    let metrics = json!({
        "elapsed_ms": elapsed.as_millis() as u64,
        "records": records,
        "records_per_sec": per_sec(records, elapsed),
        "logical_bytes": logical,
        "logical_gb_per_sec": gb_per_sec(logical, elapsed),
        "physical_bytes": physical,
        "physical_mb_per_sec": mb_per_sec(physical, elapsed),
        "metrics_records_per_sec": snap.records_per_sec.0,
    });

    Ok(BenchReport {
        suite: "ingest".into(),
        meta: config.meta(),
        metrics,
    })
}
