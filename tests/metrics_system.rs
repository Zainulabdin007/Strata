//! Canonical metrics system tests.

use std::time::Duration;

use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema};
use strata_metrics::{MetricsHandle, MetricsReport};
use strata_storage::{DatasetWriter, DatasetWriterOptions};

#[test]
fn live_and_final_snapshots_share_shape() {
    let handle = MetricsHandle::new();
    handle.begin_input_file("a.csv", 128);
    handle.add_parse_time(Duration::from_millis(3));
    handle.add_records_read(10);
    handle.record_block_sizes(
        1000,
        400,
        200,
        Duration::from_millis(4),
        Duration::from_millis(1),
    );
    handle.record_write(400, Duration::from_millis(2));
    handle.record_block_completed();
    handle.add_records_processed(10);
    handle.finish_input_file();
    handle.record_segment_completed();

    let live = handle.snapshot();
    assert!(live.live);
    assert_eq!(live.records_processed, 10);
    assert_eq!(live.logical_input_bytes.get(), 1000);
    assert_eq!(live.semantic_encoded_bytes.get(), 400);
    assert_eq!(live.compressed_bytes.get(), 200);
    assert!(live.to_cli_live().contains("[live]"));

    let final_snap = handle.finish();
    assert!(!final_snap.live);
    let text = MetricsReport::new("ingest", final_snap).to_cli_summary();
    assert!(text.contains("Raw logical:"));
    assert!(text.contains("Semantic encoded:"));
    assert!(text.contains("Final compressed:"));
    assert!(!text.contains("0 ms (not enabled)"));
}

#[test]
fn storage_emits_real_encode_and_write_metrics() {
    let schema = Schema::new(
        "m",
        vec![
            Field::required("id", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::nullable("note", LogicalType::categorical_string(None)),
        ],
    )
    .unwrap();

    let records: Vec<_> = (0..20)
        .map(|i| {
            Record::new()
                .with("id", FieldValue::UnsignedInteger(i))
                .with(
                    "note",
                    if i % 5 == 0 {
                        FieldValue::Null
                    } else {
                        FieldValue::CategoricalString(format!("n{i}"))
                    },
                )
        })
        .collect();

    let handle = MetricsHandle::new();
    let root = std::env::temp_dir().join(format!("strata-metrics-ds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let mut writer = DatasetWriter::create(
        &root,
        schema,
        DatasetWriterOptions {
            block_row_count: 5,
            segment_target_size_bytes: 1024 * 1024,
            metrics: Some(handle.clone()),
            ..Default::default()
        },
    )
    .unwrap();
    writer.write_records(&records).unwrap();
    writer.finish().unwrap();

    let snap = handle.finish();
    assert_eq!(snap.records_processed, 20);
    assert!(snap.blocks_completed >= 4);
    assert_eq!(snap.segments_completed, 1);
    assert!(snap.logical_input_bytes.get() > 0);
    assert!(snap.physical_bytes_written.get() > 0);
    // Encode and write times are measured; compression remains zero.
    assert_eq!(snap.compression_time.nanos, 0);
    // Physical includes framing so ratio is a real measured value.
    assert!(snap.compression_ratio.get().is_finite());
}
