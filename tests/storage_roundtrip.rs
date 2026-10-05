//! Lossless storage round-trip and corruption tests.

use std::fs;
use std::path::PathBuf;

use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_format::{BlockId, SegmentId};
use strata_storage::{
    BlockReader, BlockWriter, DatasetReader, DatasetWriter, DatasetWriterOptions, SegmentPath,
    SegmentReader, SegmentWriter,
};

fn temp_root(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("strata-rt-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn all_types_schema() -> Schema {
    Schema::new(
        "all_types_v1",
        vec![
            Field::required("b", LogicalType::boolean()),
            Field::required("u8", LogicalType::unsigned_integer(IntegerWidth::Bits8)),
            Field::required("u64", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::required("i32", LogicalType::signed_integer(IntegerWidth::Bits32)),
            Field::required("i64", LogicalType::signed_integer(IntegerWidth::Bits64)),
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required(
                "dec",
                LogicalType::fixed_decimal(18, 8, IntegerWidth::Bits64),
            ),
            Field::required("sym", LogicalType::categorical_string(Some(32))),
            Field::required(
                "side",
                LogicalType::enum_type(vec!["buy".into(), "sell".into()]),
            ),
            Field::nullable("bin", LogicalType::binary(Some(64))),
            Field::nullable("opt_u", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
        ],
    )
    .unwrap()
}

fn sample_record(seq: u64) -> Record {
    Record::new()
        .with("b", FieldValue::Boolean(seq.is_multiple_of(2)))
        .with("u8", FieldValue::UnsignedInteger(seq % 250))
        .with(
            "u64",
            FieldValue::UnsignedInteger(if seq == 0 { u64::MAX } else { seq * 1_000_003 }),
        )
        .with(
            "i32",
            FieldValue::SignedInteger(if seq == 1 {
                i32::MIN as i64
            } else {
                -(seq as i64)
            }),
        )
        .with(
            "i64",
            FieldValue::SignedInteger(if seq == 2 {
                i64::MIN
            } else {
                -((seq as i64) * 9)
            }),
        )
        .with(
            "ts",
            FieldValue::Timestamp(1_700_000_000_000_000_000 + seq as i64),
        )
        .with(
            "dec",
            FieldValue::FixedDecimal(190_2500_0000i64 + seq as i64),
        )
        .with("sym", FieldValue::CategoricalString(format!("S{seq}")))
        .with(
            "side",
            FieldValue::Enum(if seq.is_multiple_of(2) { "buy" } else { "sell" }.into()),
        )
        .with(
            "bin",
            if seq.is_multiple_of(3) {
                FieldValue::Null
            } else {
                FieldValue::Binary(vec![seq as u8, 0xAB, 0xCD])
            },
        )
        .with(
            "opt_u",
            if seq.is_multiple_of(4) {
                FieldValue::Null
            } else {
                FieldValue::UnsignedInteger(seq)
            },
        )
}

fn assert_records_eq(a: &[Record], b: &[Record]) {
    assert_eq!(a.len(), b.len(), "record count mismatch");
    for (i, (left, right)) in a.iter().zip(b.iter()).enumerate() {
        assert_eq!(left, right, "record {i} mismatch");
    }
}

#[test]
fn roundtrip_one_row() {
    let schema = all_types_schema();
    let records = vec![sample_record(0)];
    roundtrip_dataset(&schema, &records, 64, 1024 * 1024);
}

#[test]
fn roundtrip_many_rows() {
    let schema = all_types_schema();
    let records: Vec<_> = (0..500).map(sample_record).collect();
    roundtrip_dataset(&schema, &records, 64, 1024 * 1024);
}

#[test]
fn roundtrip_many_blocks() {
    let schema = all_types_schema();
    let records: Vec<_> = (0..100).map(sample_record).collect();
    // Force many blocks with tiny block size.
    roundtrip_dataset(&schema, &records, 3, 1024 * 1024);
}

#[test]
fn roundtrip_many_segments() {
    let schema = all_types_schema();
    let records: Vec<_> = (0..40).map(sample_record).collect();
    // Tiny segment target forces rotation after each small block.
    roundtrip_dataset(&schema, &records, 5, 64);
}

#[test]
fn roundtrip_all_supported_types_and_empties() {
    let schema = all_types_schema();
    let mut records = vec![sample_record(0), sample_record(3), sample_record(4)];
    // Explicit empty categorical string and empty binary.
    records.push(
        Record::new()
            .with("b", FieldValue::Boolean(false))
            .with("u8", FieldValue::UnsignedInteger(0))
            .with("u64", FieldValue::UnsignedInteger(0))
            .with("i32", FieldValue::SignedInteger(0))
            .with("i64", FieldValue::SignedInteger(0))
            .with("ts", FieldValue::Timestamp(0))
            .with("dec", FieldValue::FixedDecimal(0))
            .with("sym", FieldValue::CategoricalString(String::new()))
            .with("side", FieldValue::Enum("buy".into()))
            .with("bin", FieldValue::Binary(Vec::new()))
            .with("opt_u", FieldValue::Null),
    );
    roundtrip_dataset(&schema, &records, 2, 1024 * 1024);
}

#[test]
fn roundtrip_block_writer_reader_direct() {
    let schema = all_types_schema();
    let records: Vec<_> = (0..7).map(sample_record).collect();
    let mut bw = BlockWriter::new(schema.clone());
    bw.extend(records.iter().cloned()).unwrap();
    let encoded = bw.finish(BlockId(0)).unwrap();
    let batch = BlockReader::new()
        .read_batch(&schema, &encoded.bytes)
        .unwrap();
    assert_records_eq(&records, &batch.records);
}

#[test]
fn roundtrip_segment_writer_reader_direct() {
    let schema = all_types_schema();
    let records: Vec<_> = (0..25).map(sample_record).collect();
    let root = temp_root("seg");
    let path = SegmentPath::new(root.join("000000.strata"));
    let mut sw =
        SegmentWriter::create(path.clone(), schema.clone(), SegmentId::new("seg-0"), 4).unwrap();
    sw.write_records(&records).unwrap();
    let meta = sw.finish().unwrap();
    assert!(meta.header.block_count > 1);

    let reader = SegmentReader::open(path).unwrap();
    let decoded = reader.read_all_records().unwrap();
    assert_records_eq(&records, &decoded);
}

#[test]
fn corrupt_block_checksum_errors() {
    let schema = all_types_schema();
    let mut bw = BlockWriter::new(schema.clone());
    bw.push(sample_record(1)).unwrap();
    let mut encoded = bw.finish(BlockId(0)).unwrap();
    let last = encoded.bytes.len() - 1;
    encoded.bytes[last] ^= 0xff;
    let err = BlockReader::new()
        .read_batch(&schema, &encoded.bytes)
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("CRC32") || msg.contains("corrupt"), "{msg}");
}

#[test]
fn corrupt_segment_magic_errors() {
    let schema = all_types_schema();
    let root = temp_root("bad-magic");
    let path = SegmentPath::new(root.join("x.strata"));
    let mut sw = SegmentWriter::create(path.clone(), schema, SegmentId::new("s"), 10).unwrap();
    sw.write_records(&[sample_record(1)]).unwrap();
    sw.finish().unwrap();

    let mut bytes = fs::read(&path.path).unwrap();
    bytes[0] ^= 0xff;
    fs::write(&path.path, bytes).unwrap();

    let err = SegmentReader::open(path).unwrap_err();
    assert!(err.to_string().contains("magic") || err.to_string().contains("corrupt"));
}

#[test]
fn truncated_segment_errors() {
    let schema = all_types_schema();
    let root = temp_root("trunc");
    let path = SegmentPath::new(root.join("x.strata"));
    let mut sw = SegmentWriter::create(path.clone(), schema, SegmentId::new("s"), 10).unwrap();
    sw.write_records(&[sample_record(1)]).unwrap();
    sw.finish().unwrap();

    let bytes = fs::read(&path.path).unwrap();
    fs::write(&path.path, &bytes[..32]).unwrap();

    let err = SegmentReader::open(path).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("corrupt")
            || msg.contains("short")
            || msg.contains("EOF")
            || msg.contains("magic"),
        "{msg}"
    );
}

fn roundtrip_dataset(schema: &Schema, records: &[Record], block_rows: u32, segment_target: u64) {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let root = temp_root(&format!("ds-{block_rows}-{segment_target}-{id}"));
    let options = DatasetWriterOptions {
        block_row_count: block_rows,
        segment_target_size_bytes: segment_target,
        ..Default::default()
    };
    let mut writer = DatasetWriter::create(&root, schema.clone(), options).unwrap();
    writer.write_records(records).unwrap();
    let manifest = writer.finish().unwrap();
    assert_eq!(manifest.record_count, records.len() as u64);

    let reader = DatasetReader::open(&root).unwrap();
    let decoded = reader.read_all_records().unwrap();
    assert_records_eq(records, &decoded);
}
