//! Query latency / throughput suite.

use std::fs;
use std::time::Instant;

use anyhow::Result;
use serde_json::json;
use strata_query::{execute_with_options, parse_query, QueryOptions};
use strata_storage::rebuild_indexes;

use crate::config::{ingest_streaming, per_sec, percentile, BenchConfig};
use crate::gen::Workload;
use crate::report::BenchReport;

pub fn run_query(config: &BenchConfig) -> Result<BenchReport> {
    let root = config.dataset_path("query");
    let _ = fs::remove_dir_all(&root);
    let _ = ingest_streaming(config, &root, false, None)?;
    rebuild_indexes(&root)?;

    let queries = query_texts(config.workload);
    let mut results = Vec::new();

    for (name, text) in queries {
        let mut samples_ms = Vec::new();
        let mut last_rows = 0u64;
        let mut last_blocks_scanned = 0u64;
        let mut last_blocks_skipped = 0u64;
        // Warmup
        {
            let q = parse_query(text)?;
            let mut stream = execute_with_options(
                &root,
                q,
                QueryOptions {
                    use_indexes: true,
                    force_full_scan: false,
                    segment_read_mode: config.segment_read_mode,
                },
            )?;
            let _ = stream.collect_records()?;
        }
        for _ in 0..11 {
            let q = parse_query(text)?;
            let t0 = Instant::now();
            let mut stream = execute_with_options(
                &root,
                q,
                QueryOptions {
                    use_indexes: true,
                    force_full_scan: false,
                    segment_read_mode: config.segment_read_mode,
                },
            )?;
            let rows = stream.collect_records()?;
            let elapsed = t0.elapsed();
            samples_ms.push(elapsed.as_secs_f64() * 1000.0);
            last_rows = rows.len() as u64;
            last_blocks_scanned = stream.stats().blocks_scanned;
            last_blocks_skipped = stream.stats().blocks_skipped;
        }
        // Drop first sample after warmup already done; use remaining 11? We did warmup separately.
        samples_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let scan_rps = if let Some(med) = percentile(&samples_ms, 50.0) {
            if med > 0.0 {
                last_rows as f64 / (med / 1000.0)
            } else {
                0.0
            }
        } else {
            0.0
        };

        results.push(json!({
            "name": name,
            "query": text,
            "rows": last_rows,
            "blocks_scanned": last_blocks_scanned,
            "blocks_skipped": last_blocks_skipped,
            "latency_ms_p50": percentile(&samples_ms, 50.0),
            "latency_ms_p95": percentile(&samples_ms, 95.0),
            "latency_ms_p99": percentile(&samples_ms, 99.0),
            "scan_rows_per_sec_at_p50": scan_rps,
            "samples": samples_ms.len(),
        }));
    }

    // Full scan baseline (one timed run).
    let full = parse_query("select *")?;
    let t0 = Instant::now();
    let mut stream = execute_with_options(
        &root,
        full,
        QueryOptions {
            use_indexes: false,
            force_full_scan: true,
            segment_read_mode: config.segment_read_mode,
        },
    )?;
    let full_rows = stream.collect_records()?.len() as u64;
    let full_elapsed = t0.elapsed();

    let metrics = json!({
        "queries": results,
        "full_scan": {
            "elapsed_ms": full_elapsed.as_millis() as u64,
            "records": full_rows,
            "records_per_sec": per_sec(full_rows, full_elapsed),
        }
    });

    Ok(BenchReport {
        suite: "query".into(),
        meta: config.meta(),
        metrics,
    })
}

fn query_texts(workload: Workload) -> Vec<(&'static str, &'static str)> {
    match workload {
        Workload::Market => vec![
            ("symbol_eq", "select timestamp, price, quantity where symbol == AAPL"),
            (
                "symbol_time",
                "select timestamp, price where symbol == AAPL and timestamp between 1700000000000000000 and 1700000000100000000",
            ),
            (
                "sequence_range",
                "select sequence, symbol, price where sequence between 1000 and 5000",
            ),
        ],
        Workload::Telemetry => vec![
            ("device_eq", "select ts, metric, value where device_id == \"dev-001\""),
            (
                "metric_range",
                "select event_id, value where metric == cpu and value between 0 and 5000",
            ),
        ],
        Workload::Logs => vec![
            ("service_eq", "select ts, message, latency_us where service == api"),
            (
                "status_range",
                "select request_id, status where status between 400 and 599",
            ),
        ],
    }
}
