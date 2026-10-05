//! Deterministic profiler / analyze integration tests.

use strata_analyze::{format_human, format_json, AnalysisReport, AnalyzeOptions};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_input::{discover_inputs, open_parser, ParseOptions, RecordParser};

fn examples() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples")
}

#[test]
fn deterministic_fixture_stats() {
    let schema = Schema::new(
        "fixture",
        vec![
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required("symbol", LogicalType::categorical_string(Some(16))),
            Field::required(
                "price",
                LogicalType::fixed_decimal(18, 4, IntegerWidth::Bits64),
            ),
            Field::required(
                "side",
                LogicalType::enum_type(vec!["buy".into(), "sell".into()]),
            ),
        ],
    )
    .unwrap();

    let records: Vec<Record> = (0..1000)
        .map(|i| {
            Record::new()
                .with(
                    "ts",
                    FieldValue::Timestamp(1_000_000 + i * 382), // constant delta
                )
                .with(
                    "symbol",
                    FieldValue::CategoricalString(format!("S{}", i % 8)),
                )
                .with(
                    "price",
                    // Tight local windows, wide global span.
                    FieldValue::FixedDecimal(match i / 100 {
                        0 => 1000 + (i % 100),
                        1 => 5000 + (i % 100),
                        _ => 9000 + (i % 100),
                    }),
                )
                .with(
                    "side",
                    FieldValue::Enum(if i % 2 == 0 {
                        "buy".into()
                    } else {
                        "sell".into()
                    }),
                )
        })
        .collect();

    let options = AnalyzeOptions { block_rows: 100 };
    let report = AnalysisReport::from_records("fixture", &schema, &records, &options, 1).unwrap();

    let ts = report.columns.iter().find(|c| c.name == "ts").unwrap();
    assert_eq!(ts.present_count, 1000);
    assert!((ts.monotonicity.unwrap() - 1.0).abs() < f64::EPSILON);
    assert_eq!(ts.median_delta, Some(382.0));
    assert_eq!(
        ts.delta_variance_label,
        Some(strata_analyze::DeltaVarianceLabel::Low)
    );

    let symbol = report.columns.iter().find(|c| c.name == "symbol").unwrap();
    assert_eq!(symbol.cardinality, 8);
    assert_eq!(symbol.dictionary_size, Some(8));
    assert!((symbol.cardinality_ratio - 0.008).abs() < 1e-9);

    let price = report.columns.iter().find(|c| c.name == "price").unwrap();
    let local = price.local_block_ranges.as_ref().unwrap();
    assert_eq!(local.label, "narrow");

    let side = report.columns.iter().find(|c| c.name == "side").unwrap();
    assert_eq!(side.cardinality, 2);

    let text = format_human(&report);
    let text2 = format_human(&report);
    assert_eq!(text, text2);
    assert!(text.contains("ts\n"));
    assert!(text.contains("monotonicity: 100.00%"));
    assert!(text.contains("median delta: 382"));
    assert!(text.contains("symbol\n"));
    assert!(text.contains("cardinality: 8"));
    assert!(text.contains("side\n"));
    assert!(text.contains("cardinality: 2"));

    let json = format_json(&report).unwrap();
    let json2 = format_json(&report).unwrap();
    assert_eq!(json, json2);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["record_count"], 1000);
    assert_eq!(parsed["columns"].as_array().unwrap().len(), 4);
}

#[test]
fn analyzes_example_market_csv() {
    let profile = strata_core::load_profile(examples().join("profiles/stock-market.toml")).unwrap();
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

    let report = AnalysisReport::from_records(
        "market.csv",
        &schema,
        &batch.records,
        &AnalyzeOptions {
            block_rows: profile.block.row_count,
        },
        1,
    )
    .unwrap();

    assert_eq!(report.record_count, 3);
    let seq = report.columns.iter().find(|c| c.name == "seq").unwrap();
    assert_eq!(seq.min, Some(strata_analyze::StatScalar::U64(1)));
    assert_eq!(seq.max, Some(strata_analyze::StatScalar::U64(3)));
    assert!((seq.monotonicity.unwrap() - 1.0).abs() < f64::EPSILON);

    let side = report.columns.iter().find(|c| c.name == "side").unwrap();
    assert_eq!(side.cardinality, 2);

    let symbol = report.columns.iter().find(|c| c.name == "symbol").unwrap();
    assert_eq!(symbol.cardinality, 2); // AAPL, MSFT
    assert_eq!(symbol.dictionary_size, Some(2));
}
