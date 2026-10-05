//! Updates, tombstones, and compaction correctness tests.

use std::collections::BTreeMap;
use std::sync::Arc;

use strata_codec::{compressor_for, CompressionCodec};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_storage::{
    compact_dataset, rebuild_indexes, resolve_latest, CompactOptions, DatasetAppender,
    DatasetAppenderOptions, DatasetIndexes, DatasetReader, DatasetWriter, DatasetWriterOptions,
    TOMBSTONE_FIELD,
};

fn keyed_schema() -> Schema {
    Schema::new(
        "positions",
        vec![
            Field::required("id", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::required("symbol", LogicalType::categorical_string(Some(8))),
            Field::required("qty", LogicalType::signed_integer(IntegerWidth::Bits64)),
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
        ],
    )
    .unwrap()
}

fn rec(id: u64, symbol: &str, qty: i64, ts: i64) -> Record {
    Record::new()
        .with("id", FieldValue::UnsignedInteger(id))
        .with("symbol", FieldValue::CategoricalString(symbol.into()))
        .with("qty", FieldValue::SignedInteger(qty))
        .with("ts", FieldValue::Timestamp(ts))
}

fn writer_opts(keys: &[&str]) -> DatasetWriterOptions {
    DatasetWriterOptions {
        block_row_count: 8,
        segment_target_size_bytes: 4 * 1024,
        compressor: Arc::from(compressor_for(CompressionCodec::Lz4, Some(0))),
        key_fields: keys.iter().map(|s| (*s).to_string()).collect(),
        ..Default::default()
    }
}

fn logical_map(records: &[Record]) -> BTreeMap<u64, i64> {
    let mut m = BTreeMap::new();
    for r in records {
        let id = match r.get("id") {
            Some(FieldValue::UnsignedInteger(i)) => *i,
            _ => panic!("id"),
        };
        let qty = match r.get("qty") {
            Some(FieldValue::SignedInteger(q)) => *q,
            _ => panic!("qty"),
        };
        m.insert(id, qty);
    }
    m
}

#[test]
fn append_only_skips_compaction_requirement() {
    let root = std::env::temp_dir().join(format!("strata-ao-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut w = DatasetWriter::create(&root, keyed_schema(), writer_opts(&[])).unwrap();
    w.write_records(&[rec(1, "AAPL", 10, 1)]).unwrap();
    w.finish().unwrap();
    let reader = DatasetReader::open(&root).unwrap();
    assert!(!reader.is_keyed());
    assert!(compact_dataset(&root, CompactOptions::default()).is_err());
}

#[test]
fn update_delete_resolve_and_compact_preserve_logical_state() {
    let root = std::env::temp_dir().join(format!("strata-upd-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let mut w = DatasetWriter::create(&root, keyed_schema(), writer_opts(&["id"])).unwrap();
    assert!(w.schema().field(TOMBSTONE_FIELD).is_some());
    w.write_records(&[
        rec(1, "AAPL", 10, 1),
        rec(2, "MSFT", 20, 2),
        rec(3, "GOOG", 30, 3),
    ])
    .unwrap();
    w.finish().unwrap();

    let mut app = DatasetAppender::open(&root, DatasetAppenderOptions::default()).unwrap();
    app.update(rec(1, "AAPL", 15, 4)).unwrap(); // update qty
    app.update(rec(4, "AMZN", 40, 5)).unwrap(); // insert
    app.delete(&rec(2, "MSFT", 0, 0)).unwrap(); // delete id=2
    app.finish().unwrap();

    let reader = DatasetReader::open(&root).unwrap();
    let physical = reader.read_all_records().unwrap();
    assert!(physical.len() > 4); // history retained
    let before = reader.read_latest_records().unwrap();
    let before_map = logical_map(&before);
    assert_eq!(before_map.get(&1), Some(&15));
    assert_eq!(before_map.get(&2), None);
    assert_eq!(before_map.get(&3), Some(&30));
    assert_eq!(before_map.get(&4), Some(&40));
    assert_eq!(before.len(), 3);

    // Latest index rebuild.
    let report = rebuild_indexes(&root).unwrap();
    assert!(report.latest_version_written);
    let idx = DatasetIndexes::open(&root).unwrap();
    assert_eq!(idx.latest_version().unwrap().entries.len(), 3);

    // Compact and compare logical state.
    let compact = compact_dataset(
        &root,
        CompactOptions::default().with_compression(CompressionCodec::Lz4, Some(0)),
    )
    .unwrap();
    assert_eq!(compact.logical_records_after, 3);
    assert!(compact.physical_records_before > 3);
    assert!(compact.indexes_rebuilt);

    let after = DatasetReader::open(&root)
        .unwrap()
        .read_latest_records()
        .unwrap();
    assert_eq!(logical_map(&after), before_map);
    assert_eq!(
        DatasetReader::open(&root).unwrap().manifest().record_count,
        3
    );

    // Physical equals logical after compaction (no obsolete versions).
    let physical_after = DatasetReader::open(&root)
        .unwrap()
        .read_all_records()
        .unwrap();
    assert_eq!(physical_after.len(), after.len());
    assert_eq!(
        resolve_latest(&["id".into()], &physical_after)
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn compact_twice_is_stable() {
    let root = std::env::temp_dir().join(format!("strata-c2-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut w = DatasetWriter::create(&root, keyed_schema(), writer_opts(&["id"])).unwrap();
    w.write_records(&[rec(1, "AAPL", 1, 1), rec(2, "MSFT", 2, 2)])
        .unwrap();
    w.finish().unwrap();
    let mut app = DatasetAppender::open(&root, DatasetAppenderOptions::default()).unwrap();
    app.update(rec(1, "AAPL", 9, 3)).unwrap();
    app.finish().unwrap();

    let first = DatasetReader::open(&root)
        .unwrap()
        .read_latest_records()
        .unwrap();
    compact_dataset(&root, CompactOptions::default()).unwrap();
    let mid = DatasetReader::open(&root)
        .unwrap()
        .read_latest_records()
        .unwrap();
    compact_dataset(&root, CompactOptions::default()).unwrap();
    let end = DatasetReader::open(&root)
        .unwrap()
        .read_latest_records()
        .unwrap();
    assert_eq!(logical_map(&first), logical_map(&mid));
    assert_eq!(logical_map(&mid), logical_map(&end));
    assert_eq!(
        DatasetReader::open(&root)
            .unwrap()
            .manifest()
            .compaction_generation,
        2
    );
}
