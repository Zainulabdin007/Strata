//! Scaling benchmark for parallel ingest (workers = 1,2,4,8,12).
//!
//! Records real wall-clock and phase metrics. Does not assume more workers
//! are faster — results are measured.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use strata_codec::{compressor_for, CompressionCodec};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_metrics::MetricsHandle;
use strata_storage::{write_records_parallel, DatasetWriterOptions, PipelineOptions};

fn schema() -> Schema {
    Schema::new(
        "bench_ticks",
        vec![
            Field::required("seq", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required("symbol", LogicalType::categorical_string(Some(8))),
            Field::required(
                "side",
                LogicalType::enum_type(vec!["buy".into(), "sell".into()]),
            ),
            Field::required(
                "price",
                LogicalType::fixed_decimal(18, 4, IntegerWidth::Bits64),
            ),
            Field::required("size", LogicalType::unsigned_integer(IntegerWidth::Bits32)),
        ],
    )
    .unwrap()
}

fn make_records(n: u64) -> Vec<Record> {
    (0..n)
        .map(|i| {
            Record::new()
                .with("seq", FieldValue::UnsignedInteger(i))
                .with(
                    "ts",
                    FieldValue::Timestamp(1_000_000_000_000 + i as i64 * 382),
                )
                .with(
                    "symbol",
                    FieldValue::CategoricalString(format!("S{}", i % 32)),
                )
                .with(
                    "side",
                    FieldValue::Enum(if i % 2 == 0 {
                        "buy".into()
                    } else {
                        "sell".into()
                    }),
                )
                .with(
                    "price",
                    FieldValue::FixedDecimal(100_000 + (i % 500) as i64),
                )
                .with("size", FieldValue::UnsignedInteger(1 + (i % 64)))
        })
        .collect()
}

#[derive(Debug, Clone)]
struct Row {
    workers: usize,
    elapsed_ms: u64,
    records: u64,
    blocks: u64,
    records_per_sec: f64,
    blocks_per_sec: f64,
    encode_mb_s: f64,
    compress_mb_s: f64,
    write_mb_s: f64,
    physical_mb_s: f64,
    worker_util: f64,
}

fn run_once(workers: usize, records: &[Record], schema: &Schema) -> Row {
    let metrics = MetricsHandle::new();
    let root = std::env::temp_dir().join(format!("strata-scale-w{workers}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let mut opts = PipelineOptions::with_workers(workers);
    opts.writer = DatasetWriterOptions {
        block_row_count: 4_096,
        segment_target_size_bytes: 64 * 1024 * 1024,
        metrics: Some(metrics.clone()),
        compressor: Arc::from(compressor_for(CompressionCodec::Lz4, Some(0))),
        ..Default::default()
    };

    let wall = Instant::now();
    write_records_parallel(&root, schema.clone(), records, opts).unwrap();
    let elapsed = wall.elapsed();
    let snap = metrics.finish();
    let _ = std::fs::remove_dir_all(&root);

    Row {
        workers,
        elapsed_ms: elapsed.as_millis() as u64,
        records: snap.records_processed,
        blocks: snap.blocks_completed,
        records_per_sec: snap.records_per_sec.get(),
        blocks_per_sec: snap.blocks_per_sec.get(),
        encode_mb_s: snap.encode_mb_per_sec.get(),
        compress_mb_s: snap.compress_mb_per_sec.get(),
        write_mb_s: snap.write_mb_per_sec.get(),
        physical_mb_s: snap.physical_mb_per_sec.get(),
        worker_util: snap.worker_utilization,
    }
}

#[test]
fn ingest_scaling_benchmark_records_real_results() {
    let schema = schema();
    // Large enough to exercise the pipeline; keep CI runtime reasonable.
    let records = make_records(100_000);
    let worker_counts = [1usize, 2, 4, 8, 12];

    // Warmup (not recorded).
    let _ = run_once(1, &records[..8_192], &schema);

    let mut rows = Vec::new();
    for &w in &worker_counts {
        rows.push(run_once(w, &records, &schema));
    }

    let mut md = String::new();
    md.push_str("# Ingest scaling benchmark (real results)\n\n");
    md.push_str(&format!(
        "- Records: {}\n- Block rows: 4096\n- Compression: lz4\n- Host workers available: {}\n\n",
        records.len(),
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    ));
    md.push_str(
        "| workers | elapsed_ms | records/s | blocks/s | encode MB/s | compress MB/s | write MB/s | disk MB/s | util % |\n",
    );
    md.push_str(
        "|--------:|-----------:|----------:|---------:|------------:|--------------:|-----------:|----------:|-------:|\n",
    );
    for r in &rows {
        md.push_str(&format!(
            "| {w} | {ms} | {rps:.0} | {bps:.1} | {enc:.1} | {comp:.1} | {wr:.1} | {disk:.1} | {util:.1} |\n",
            w = r.workers,
            ms = r.elapsed_ms,
            rps = r.records_per_sec,
            bps = r.blocks_per_sec,
            enc = r.encode_mb_s,
            comp = r.compress_mb_s,
            wr = r.write_mb_s,
            disk = r.physical_mb_s,
            util = r.worker_util * 100.0,
        ));
    }
    md.push_str(
        "\nNotes:\n\
         - Results are measured wall-clock / phase timers — not invented.\n\
         - More threads are **not** assumed faster; I/O and encode mix dominate.\n\
         - `util %` is encode-worker busy time / (wall × workers).\n",
    );

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../benchmarks");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out_path = out_dir.join("ingest_scaling_results.md");
    std::fs::write(&out_path, &md).unwrap();

    // Also print for CI logs.
    println!("{md}");

    // Sanity: every configuration completed all records.
    for r in &rows {
        assert_eq!(r.records, records.len() as u64);
        assert!(r.blocks > 0);
        assert!(r.elapsed_ms > 0);
    }

    assert!(out_path.exists());
}
