//! Compression size suite (ingest then measure on-disk sizes).

use std::fs;

use anyhow::Result;
use serde_json::json;
use strata_storage::DatasetReader;

use crate::config::{ingest_streaming, BenchConfig};
use crate::report::BenchReport;

pub fn run_compression(config: &BenchConfig) -> Result<BenchReport> {
    let root = config.dataset_path("compression");
    let _ = fs::remove_dir_all(&root);
    let _ = ingest_streaming(config, &root, false, None)?;

    let reader = DatasetReader::open(&root)?;
    let mut logical = 0u64;
    let mut semantic = 0u64;
    let mut compressed = 0u64;
    let mut rows = 0u64;
    let mut columns = 0u64;

    for i in 0..reader.segment_count() {
        let seg = reader.open_segment(i)?;
        for block in &seg.meta().blocks {
            logical += block.logical_bytes;
            semantic += block.semantic_encoded_bytes;
            compressed += block.compressed_bytes;
            rows += u64::from(block.row_count);
            columns += block.columns.len() as u64;
        }
    }

    let records = config.size.records();
    let values = rows.saturating_mul(reader.schema().len().max(1) as u64);
    let bits_per_value = if values == 0 {
        0.0
    } else {
        (compressed as f64 * 8.0) / values as f64
    };
    let physical = crate::baselines::dir_size(&root)?;
    let ratio = if physical == 0 {
        0.0
    } else {
        logical as f64 / physical as f64
    };

    let metrics = json!({
        "records": records,
        "raw_logical_bytes": logical,
        "encoded_semantic_bytes": semantic,
        "compressed_bytes": compressed,
        "bits_per_value": bits_per_value,
        "compression_ratio": ratio,
        "physical_bytes": physical,
        "segments": reader.segment_count(),
        "schema_columns": reader.schema().len(),
        "column_blocks_observed": columns,
    });

    Ok(BenchReport {
        suite: "compression".into(),
        meta: config.meta(),
        metrics,
    })
}
