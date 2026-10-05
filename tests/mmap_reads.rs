//! Sealed-segment mmap vs buffered read correctness.

use std::sync::Arc;

use strata_codec::{compressor_for, CompressionCodec};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_storage::{
    DatasetReader, DatasetWriter, DatasetWriterOptions, SegmentBytesBackend, SegmentPath,
    SegmentReadMode, SegmentReader,
};

fn schema() -> Schema {
    Schema::new(
        "mmap_ticks",
        vec![
            Field::required("seq", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required("symbol", LogicalType::categorical_string(Some(8))),
        ],
    )
    .unwrap()
}

fn records(n: u64) -> Vec<Record> {
    (0..n)
        .map(|i| {
            Record::new()
                .with("seq", FieldValue::UnsignedInteger(i))
                .with("ts", FieldValue::Timestamp(i as i64 * 1000))
                .with(
                    "symbol",
                    FieldValue::CategoricalString(format!("S{}", i % 4)),
                )
        })
        .collect()
}

fn write_dataset(root: &std::path::Path, n: u64) {
    let _ = std::fs::remove_dir_all(root);
    let mut writer = DatasetWriter::create(
        root,
        schema(),
        DatasetWriterOptions {
            block_row_count: 64,
            segment_target_size_bytes: 32 * 1024,
            compressor: Arc::from(compressor_for(CompressionCodec::None, None)),
            ..Default::default()
        },
    )
    .unwrap();
    writer.write_records(&records(n)).unwrap();
    writer.finish().unwrap();
}

#[test]
fn buffered_and_mmap_decode_identical() {
    let root = std::env::temp_dir().join(format!("strata-mmap-eq-{}", std::process::id()));
    write_dataset(&root, 1_500);

    let buffered = DatasetReader::open_with_mode(&root, SegmentReadMode::Buffered).unwrap();
    let mmap = DatasetReader::open_with_mode(&root, SegmentReadMode::Mmap).unwrap();

    assert_eq!(buffered.segment_count(), mmap.segment_count());
    for i in 0..buffered.segment_count() {
        let sb = buffered.open_segment(i).unwrap();
        let sm = mmap.open_segment(i).unwrap();
        assert_eq!(sb.bytes_backend(), SegmentBytesBackend::Buffered);
        #[cfg(unix)]
        assert_eq!(sm.bytes_backend(), SegmentBytesBackend::Mmap);
        assert_eq!(
            sb.read_all_records().unwrap(),
            sm.read_all_records().unwrap()
        );
        // Borrowed block views share the same content.
        for b in 0..sb.block_count() {
            assert_eq!(sb.block_bytes(b).unwrap(), sm.block_bytes(b).unwrap());
        }
    }
    assert_eq!(
        buffered.read_all_records().unwrap(),
        mmap.read_all_records().unwrap()
    );
}

#[test]
fn open_mmap_on_sealed_segment_file() {
    let root = std::env::temp_dir().join(format!("strata-mmap-seal-{}", std::process::id()));
    write_dataset(&root, 400);
    let reader = DatasetReader::open(&root).unwrap();
    let path = SegmentPath::new(
        root.join("segments")
            .join(&reader.manifest().segments[0].file),
    );
    let seg = SegmentReader::open_mmap(path).unwrap();
    #[cfg(unix)]
    assert_eq!(seg.bytes_backend(), SegmentBytesBackend::Mmap);
    assert!(!seg.as_bytes().is_empty());
    assert!(!seg.read_all_records().unwrap().is_empty());
}

#[test]
fn auto_mode_opens_readable_segments() {
    let root = std::env::temp_dir().join(format!("strata-mmap-auto-{}", std::process::id()));
    write_dataset(&root, 200);
    let reader = DatasetReader::open_with_mode(&root, SegmentReadMode::Auto).unwrap();
    let seg = reader.open_segment(0).unwrap();
    assert!(!seg.read_all_records().unwrap().is_empty());
}
