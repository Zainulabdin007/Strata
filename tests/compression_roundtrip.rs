//! Exact lossless round-trips across semantic encode + none/lz4/zstd block compression.

use std::collections::HashMap;
use std::sync::Arc;

use strata_codec::{compressor_for, plan_examples, CompressionCodec, EncodingPlan, EncodingStep};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_format::BlockId;
use strata_storage::{
    encode_block, BlockReader, DatasetReader, DatasetWriter, DatasetWriterOptions,
};

fn schema() -> Schema {
    Schema::new(
        "compress_fixture_v1",
        vec![
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required(
                "price",
                LogicalType::fixed_decimal(18, 4, IntegerWidth::Bits64),
            ),
            Field::required("symbol", LogicalType::categorical_string(Some(16))),
            Field::required(
                "side",
                LogicalType::enum_type(vec!["buy".into(), "sell".into()]),
            ),
            Field::required("qty", LogicalType::unsigned_integer(IntegerWidth::Bits32)),
            Field::nullable("flag", LogicalType::boolean()),
        ],
    )
    .unwrap()
}

fn plans() -> HashMap<String, EncodingPlan> {
    let mut m = HashMap::new();
    m.insert("ts".into(), plan_examples::timestamp());
    m.insert("price".into(), plan_examples::price());
    m.insert("symbol".into(), plan_examples::symbol());
    m.insert("side".into(), plan_examples::side());
    m.insert(
        "qty".into(),
        EncodingPlan::new(vec![EncodingStep::Varint]).unwrap(),
    );
    m.insert(
        "flag".into(),
        EncodingPlan::new(vec![EncodingStep::BooleanBitpack]).unwrap(),
    );
    m
}

fn sample_records(n: usize) -> Vec<Record> {
    (0..n)
        .map(|i| {
            Record::new()
                .with(
                    "ts",
                    FieldValue::Timestamp(1_700_000_000_000_000_000 + i as i64 * 1_000),
                )
                .with(
                    "price",
                    FieldValue::FixedDecimal(19_02500 + (i as i64 % 50)),
                )
                .with(
                    "symbol",
                    FieldValue::CategoricalString(["AAPL", "MSFT", "GOOG"][i % 3].into()),
                )
                .with(
                    "side",
                    FieldValue::Enum(if i % 2 == 0 {
                        "buy".into()
                    } else {
                        "sell".into()
                    }),
                )
                .with("qty", FieldValue::UnsignedInteger((i as u64 % 100) + 1))
                .with(
                    "flag",
                    if i % 7 == 0 {
                        FieldValue::Null
                    } else {
                        FieldValue::Boolean(i % 2 == 0)
                    },
                )
        })
        .collect()
}

fn assert_records_eq(expected: &[Record], actual: &[Record]) {
    assert_eq!(expected.len(), actual.len());
    for (e, a) in expected.iter().zip(actual) {
        assert_eq!(e, a);
    }
}

fn roundtrip_codec(codec: CompressionCodec, level: Option<i32>) {
    let schema = schema();
    let plans = plans();
    let records = sample_records(128);
    let compressor = compressor_for(codec, level);
    let encoded = encode_block(&schema, &records, BlockId(0), &plans, compressor.as_ref()).unwrap();

    assert_eq!(encoded.meta.compression_codec, codec.tag());
    assert_eq!(
        encoded.meta.semantic_encoded_bytes > 0,
        !records.is_empty() || true
    );
    if codec == CompressionCodec::None {
        assert_eq!(
            encoded.meta.compressed_bytes,
            encoded.meta.semantic_encoded_bytes
        );
        assert_eq!(encoded.compress_duration.as_nanos(), 0);
    } else {
        // Highly repetitive fixture should usually shrink; allow equal for tiny edge cases.
        assert!(encoded.meta.compressed_bytes <= encoded.meta.semantic_encoded_bytes);
    }

    let batch = BlockReader::new()
        .read_batch(&schema, &encoded.bytes)
        .unwrap();
    assert_records_eq(&records, &batch.records);
}

#[test]
fn block_roundtrip_none() {
    roundtrip_codec(CompressionCodec::None, None);
}

#[test]
fn block_roundtrip_lz4() {
    roundtrip_codec(CompressionCodec::Lz4, Some(4));
}

#[test]
fn block_roundtrip_zstd_levels() {
    for level in [1, 3, 10] {
        roundtrip_codec(CompressionCodec::Zstd, Some(level));
    }
}

#[test]
fn dataset_roundtrip_all_codecs() {
    let schema = schema();
    let plans = plans();
    let records = sample_records(64);

    for codec in [
        CompressionCodec::None,
        CompressionCodec::Lz4,
        CompressionCodec::Zstd,
    ] {
        let root = std::env::temp_dir().join(format!(
            "strata-compress-{}-{}",
            codec.as_str(),
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let mut writer = DatasetWriter::create(
            &root,
            schema.clone(),
            DatasetWriterOptions {
                block_row_count: 16,
                segment_target_size_bytes: 1024 * 1024,
                encoding_plans: plans.clone(),
                compressor: Arc::from(compressor_for(codec, Some(3))),
                ..Default::default()
            },
        )
        .unwrap();
        writer.write_records(&records).unwrap();
        writer.finish().unwrap();

        let decoded = DatasetReader::open(&root)
            .unwrap()
            .read_all_records()
            .unwrap();
        assert_records_eq(&records, &decoded);
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[test]
fn empty_block_roundtrip_all_codecs() {
    let schema = schema();
    let plans = plans();
    for codec in [
        CompressionCodec::None,
        CompressionCodec::Lz4,
        CompressionCodec::Zstd,
    ] {
        let encoded = encode_block(
            &schema,
            &[],
            BlockId(0),
            &plans,
            compressor_for(codec, None).as_ref(),
        )
        .unwrap();
        let batch = BlockReader::new()
            .read_batch(&schema, &encoded.bytes)
            .unwrap();
        assert!(batch.records.is_empty());
    }
}
