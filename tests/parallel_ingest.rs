//! Parallel ingest pipeline tests.

use std::sync::Arc;

use strata_codec::{compressor_for, CompressionCodec};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_metrics::MetricsHandle;
use strata_storage::{
    write_records_parallel, DatasetReader, DatasetWriterOptions, IngestPipeline, PipelineOptions,
};

fn make_records(n: u64) -> Vec<Record> {
    (0..n)
        .map(|i| {
            Record::new()
                .with("seq", FieldValue::UnsignedInteger(i))
                .with(
                    "ts",
                    FieldValue::Timestamp(1_700_000_000_000_000 + i as i64 * 1_000),
                )
                .with(
                    "symbol",
                    FieldValue::CategoricalString(format!("S{}", i % 16)),
                )
                .with(
                    "side",
                    FieldValue::Enum(if i % 2 == 0 {
                        "buy".into()
                    } else {
                        "sell".into()
                    }),
                )
                .with("price", FieldValue::FixedDecimal(10_000 + (i % 100) as i64))
        })
        .collect()
}

fn schema() -> Schema {
    Schema::new(
        "parallel_ticks",
        vec![
            Field::required("seq", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Microseconds)),
            Field::required("symbol", LogicalType::categorical_string(Some(8))),
            Field::required(
                "side",
                LogicalType::enum_type(vec!["buy".into(), "sell".into()]),
            ),
            Field::required(
                "price",
                LogicalType::fixed_decimal(18, 4, IntegerWidth::Bits64),
            ),
        ],
    )
    .unwrap()
}

#[test]
fn parallel_matches_sequential_order() {
    let schema = schema();
    let records = make_records(5_000);

    let root_seq =
        std::env::temp_dir().join(format!("strata-par-seq-{}-{}", std::process::id(), 1));
    let root_par =
        std::env::temp_dir().join(format!("strata-par-par-{}-{}", std::process::id(), 4));
    let _ = std::fs::remove_dir_all(&root_seq);
    let _ = std::fs::remove_dir_all(&root_par);

    let writer_opts = DatasetWriterOptions {
        block_row_count: 256,
        segment_target_size_bytes: 8 * 1024 * 1024,
        compressor: Arc::from(compressor_for(CompressionCodec::Lz4, Some(0))),
        ..Default::default()
    };

    // Sequential via single-worker pipeline.
    let mut opts1 = PipelineOptions::with_workers(1);
    opts1.writer = writer_opts.clone();
    write_records_parallel(&root_seq, schema.clone(), &records, opts1).unwrap();

    let mut opts4 = PipelineOptions::with_workers(4);
    opts4.writer = writer_opts;
    write_records_parallel(&root_par, schema.clone(), &records, opts4).unwrap();

    let a = DatasetReader::open(&root_seq)
        .unwrap()
        .read_all_records()
        .unwrap();
    let b = DatasetReader::open(&root_par)
        .unwrap()
        .read_all_records()
        .unwrap();
    assert_eq!(a, records);
    assert_eq!(b, records);
    assert_eq!(a, b);
}

#[test]
fn pipeline_reports_queue_and_worker_metrics() {
    let schema = schema();
    let records = make_records(2_048);
    let metrics = MetricsHandle::new();
    let root = std::env::temp_dir().join(format!("strata-par-metrics-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let mut opts = PipelineOptions::with_workers(2);
    opts.writer = DatasetWriterOptions {
        block_row_count: 128,
        segment_target_size_bytes: 4 * 1024 * 1024,
        metrics: Some(metrics.clone()),
        compressor: Arc::from(compressor_for(CompressionCodec::None, None)),
        ..Default::default()
    };

    let mut pipeline = IngestPipeline::start(&root, schema, opts).unwrap();
    for chunk in records.chunks(128) {
        pipeline.submit(chunk.to_vec()).unwrap();
    }
    pipeline.finish().unwrap();

    let snap = metrics.finish();
    assert_eq!(snap.records_processed, 2_048);
    assert!(snap.blocks_completed >= 16);
    assert_eq!(snap.worker_count, 2);
    assert!(snap.blocks_per_sec.get() > 0.0);
    assert!(snap.encode_mb_per_sec.get() >= 0.0);
    assert!(snap.write_mb_per_sec.get() >= 0.0);
    // Utilization is measured; may be low on tiny runs but must be finite.
    assert!(snap.worker_utilization.is_finite());
    assert!(snap.to_cli_summary().contains("Worker utilization:"));
    assert!(snap.to_cli_summary().contains("Blocks/sec:"));
}
