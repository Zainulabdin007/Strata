//! Workspace-level smoke tests.

use strata_codec::{ColumnDecoder, ColumnEncoder, Compressor, IdentityCompressor, PlainEncoder};
use strata_core::{
    load_profile, FieldValue, IntegerWidth, LogicalType, Record, RecordBatch, TimestampUnit,
};
use strata_format::{BlockEncoding, BlockId, BlockMeta, ColumnBlockMeta, SegmentId, SegmentMeta};
use strata_input::{discover_inputs, open_parser, ParseOptions, RecordParser};
use strata_metrics::{MetricsCollector, MetricsReport};
use strata_storage::{DatasetReader, DatasetWriter, DatasetWriterOptions};

fn examples() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples")
}

#[test]
fn end_to_end_ingest_roundtrip_smoke() {
    let profile = load_profile(examples().join("profiles/stock-market.toml")).unwrap();
    let encoding_plans = strata_codec::resolve_profile_plans(&profile).unwrap();
    let discovered =
        discover_inputs(examples().join("data/market.csv"), &profile).expect("discover");
    let schema = profile.schema().unwrap();
    let mut parser = open_parser(&profile).unwrap();
    let batch = RecordParser::parse_file(
        parser.as_mut(),
        &discovered.files[0],
        &schema,
        &ParseOptions::default(),
    )
    .unwrap();
    assert_eq!(batch.len(), 3);

    let root = std::env::temp_dir().join(format!("strata-smoke-ds-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let mut writer = DatasetWriter::create(
        &root,
        schema.clone(),
        DatasetWriterOptions {
            block_row_count: 2,
            segment_target_size_bytes: 1024 * 1024,
            encoding_plans,
            ..Default::default()
        },
    )
    .unwrap();
    writer.write_batch(&batch).unwrap();
    writer.finish().unwrap();

    let decoded = DatasetReader::open(&root)
        .unwrap()
        .read_all_records()
        .unwrap();
    assert_eq!(decoded, batch.records);

    let enc = PlainEncoder;
    let col: Vec<FieldValue> = batch
        .records
        .iter()
        .map(|r| r.get("seq").unwrap().clone())
        .collect();
    let encoded = enc
        .encode(&LogicalType::unsigned_integer(IntegerWidth::Bits64), &col)
        .unwrap();
    assert_eq!(
        enc.decode(
            &LogicalType::unsigned_integer(IntegerWidth::Bits64),
            col.len() as u32,
            &encoded
        )
        .unwrap(),
        col
    );
    assert_eq!(IdentityCompressor.compress(&encoded).unwrap(), encoded);
    assert_eq!(
        IdentityCompressor
            .decompress(&encoded, encoded.len())
            .unwrap(),
        encoded
    );

    let mut meta = SegmentMeta::empty(schema.id.clone(), SegmentId::new("seg-0"));
    let mut block = BlockMeta::new(BlockId(0), batch.len() as u32);
    block.push_column(ColumnBlockMeta {
        field_name: "seq".into(),
        encoding: BlockEncoding::Plain,
        data_rel_offset: 0,
        data_len: encoded.len() as u32,
        logical_bytes: 8 * batch.len() as u64,
        encoded_bytes: encoded.len() as u64,
    });
    meta.push_block(block);

    let mut metrics = MetricsCollector::start();
    metrics.add_records_read(batch.len() as u64);
    metrics.add_records_processed(batch.len() as u64);
    metrics.add_bytes(100, 50);
    let report = MetricsReport::new("ingest", metrics.finish());
    assert!(report.to_cli_summary().contains("Records:"));
    assert!(report.metrics.records_processed == 3);
}

#[test]
fn record_batch_still_validates() {
    let schema = strata_core::Schema::new(
        "ticks_v1",
        vec![
            strata_core::Field::required(
                "seq",
                LogicalType::unsigned_integer(IntegerWidth::Bits64),
            ),
            strata_core::Field::required("ts", LogicalType::timestamp(TimestampUnit::Microseconds)),
        ],
    )
    .unwrap();

    let record = Record::new()
        .with("seq", FieldValue::UnsignedInteger(1))
        .with("ts", FieldValue::Timestamp(1));
    let batch = RecordBatch::try_new(schema, vec![record]).unwrap();
    assert_eq!(batch.len(), 1);
}
