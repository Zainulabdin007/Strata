//! Correctness tests for the typed Strata query engine.

use std::sync::Arc;

use strata_codec::{compressor_for, CompressionCodec};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_query::{execute, execute_with_options, parse_query, QueryOptions};
use strata_storage::{rebuild_indexes, DatasetWriter, DatasetWriterOptions};

fn schema() -> Schema {
    Schema::new(
        "query_ticks",
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
    let symbols = ["AAPL", "MSFT", "GOOG", "AMZN"];
    (0..n)
        .map(|i| {
            Record::new()
                .with("seq", FieldValue::UnsignedInteger(i))
                .with(
                    "ts",
                    FieldValue::Timestamp(1_700_000_000_000_000_000 + i as i64 * 1_000),
                )
                .with(
                    "symbol",
                    FieldValue::CategoricalString(symbols[(i as usize) % symbols.len()].into()),
                )
                .with(
                    "side",
                    FieldValue::Enum(if i % 2 == 0 {
                        "buy".into()
                    } else {
                        "sell".into()
                    }),
                )
                .with("price", FieldValue::FixedDecimal(10_000 + (i % 200) as i64))
                .with("size", FieldValue::UnsignedInteger(1 + (i % 10)))
        })
        .collect()
}

fn write_and_index(root: &std::path::Path, n: u64, block_rows: u32) {
    let _ = std::fs::remove_dir_all(root);
    let mut writer = DatasetWriter::create(
        root,
        schema(),
        DatasetWriterOptions {
            block_row_count: block_rows,
            segment_target_size_bytes: 16 * 1024,
            compressor: Arc::from(compressor_for(CompressionCodec::None, None)),
            ..Default::default()
        },
    )
    .unwrap();
    writer.write_records(&make_records(n)).unwrap();
    writer.finish().unwrap();
    rebuild_indexes(root).unwrap();
}

#[test]
fn projection_and_limit() {
    let root = std::env::temp_dir().join(format!("strata-q-proj-{}", std::process::id()));
    write_and_index(&root, 500, 50);
    let q = parse_query("select ts, price where symbol == AAPL limit 5").unwrap();
    let mut stream = execute(&root, q).unwrap();
    let rows = stream.collect_records().unwrap();
    assert_eq!(rows.len(), 5);
    for r in &rows {
        assert!(r.get("ts").is_some());
        assert!(r.get("price").is_some());
        assert!(r.get("symbol").is_none());
        assert!(r.get("seq").is_none());
    }
}

#[test]
fn categorical_and_time_range() {
    let root = std::env::temp_dir().join(format!("strata-q-sym-{}", std::process::id()));
    write_and_index(&root, 2_000, 64);
    let t1 = 1_700_000_000_000_000_000 + 100 * 1_000;
    let t2 = 1_700_000_000_000_000_000 + 400 * 1_000;
    let q = parse_query(&format!(
        "select seq, ts, symbol, price where symbol == AAPL and ts between {t1} and {t2}"
    ))
    .unwrap();
    let mut stream = execute(&root, q).unwrap();
    assert!(stream.explain().used_indexes);
    assert!(stream.explain().selected_blocks < stream.explain().total_blocks);

    let rows = stream.collect_records().unwrap();
    let expect: Vec<_> = make_records(2_000)
        .into_iter()
        .filter(|r| {
            r.get("symbol") == Some(&FieldValue::CategoricalString("AAPL".into()))
                && matches!(r.get("ts"), Some(FieldValue::Timestamp(t)) if *t >= t1 && *t <= t2)
        })
        .collect();
    assert_eq!(rows.len(), expect.len());
    for (got, exp) in rows.iter().zip(expect.iter()) {
        assert_eq!(got.get("seq"), exp.get("seq"));
        assert_eq!(got.get("ts"), exp.get("ts"));
        assert_eq!(got.get("symbol"), exp.get("symbol"));
        assert_eq!(got.get("price"), exp.get("price"));
    }
}

#[test]
fn sequence_range_and_in_list() {
    let root = std::env::temp_dir().join(format!("strata-q-seq-{}", std::process::id()));
    write_and_index(&root, 1_500, 40);
    let q =
        parse_query("select seq, symbol where seq between 100 and 250 and symbol in (AAPL, MSFT)")
            .unwrap();
    let rows = execute(&root, q).unwrap().collect_records().unwrap();
    for r in &rows {
        let seq = match r.get("seq") {
            Some(FieldValue::UnsignedInteger(s)) => *s,
            _ => panic!("seq"),
        };
        assert!((100..=250).contains(&seq));
        let sym = match r.get("symbol") {
            Some(FieldValue::CategoricalString(s)) => s.as_str(),
            _ => panic!("symbol"),
        };
        assert!(sym == "AAPL" || sym == "MSFT");
    }
    let expect = make_records(1_500)
        .into_iter()
        .filter(|r| {
            matches!(r.get("seq"), Some(FieldValue::UnsignedInteger(s)) if (100..=250).contains(s))
                && matches!(
                    r.get("symbol"),
                    Some(FieldValue::CategoricalString(s)) if s == "AAPL" || s == "MSFT"
                )
        })
        .count();
    assert_eq!(rows.len(), expect);
}

#[test]
fn numeric_range_price() {
    let root = std::env::temp_dir().join(format!("strata-q-price-{}", std::process::id()));
    write_and_index(&root, 800, 50);
    let q = parse_query("select seq, price where price between 10050 and 10080").unwrap();
    let rows = execute(&root, q).unwrap().collect_records().unwrap();
    let expect = make_records(800)
        .into_iter()
        .filter(|r| {
            matches!(r.get("price"), Some(FieldValue::FixedDecimal(p)) if (10050..=10080).contains(p))
        })
        .count();
    assert_eq!(rows.len(), expect);
}

#[test]
fn full_scan_matches_indexed_results() {
    let root = std::env::temp_dir().join(format!("strata-q-scan-{}", std::process::id()));
    write_and_index(&root, 1_200, 60);
    let text = "select seq, symbol where symbol == GOOG and seq >= 200 and seq <= 900";
    let q1 = parse_query(text).unwrap();
    let q2 = parse_query(text).unwrap();
    let indexed = execute(&root, q1).unwrap().collect_records().unwrap();
    let scanned = execute_with_options(
        &root,
        q2,
        QueryOptions {
            use_indexes: false,
            force_full_scan: true,
            ..Default::default()
        },
    )
    .unwrap()
    .collect_records()
    .unwrap();
    assert_eq!(indexed, scanned);
}

#[test]
fn decode_columns_exclude_unused() {
    let root = std::env::temp_dir().join(format!("strata-q-decode-{}", std::process::id()));
    write_and_index(&root, 300, 50);
    let q = parse_query("select price where symbol == AAPL").unwrap();
    let stream = execute(&root, q).unwrap();
    let decode = &stream.explain().decode_columns;
    assert!(decode.contains(&"price".to_string()));
    assert!(decode.contains(&"symbol".to_string()));
    assert!(!decode.contains(&"size".to_string()));
    assert!(!decode.contains(&"side".to_string()));
}

#[test]
fn stale_corrupt_or_missing_indexes_fall_back_to_scan() {
    let root =
        std::env::temp_dir().join(format!("strata-q-index-integrity-{}", std::process::id()));
    write_and_index(&root, 500, 25);
    let q = parse_query("select seq where seq between 100 and 199").unwrap();
    let catalog = root.join("indexes/manifest.json");
    let original = std::fs::read(&catalog).unwrap();
    for mode in 0..3 {
        let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
        match mode {
            0 => value["dataset_crc"] = serde_json::json!(0),
            1 => value["artifacts"] = serde_json::json!([]),
            _ => value["checksums"] = serde_json::json!({}),
        }
        std::fs::write(&catalog, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut stream = execute(&root, q.clone()).unwrap();
        let rows = stream.collect_records().unwrap();
        assert_eq!(rows.len(), 100, "index failure mode {mode}");
        for (i, row) in rows.iter().enumerate() {
            assert_eq!(
                row.get("seq"),
                Some(&FieldValue::UnsignedInteger(100 + i as u64))
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
