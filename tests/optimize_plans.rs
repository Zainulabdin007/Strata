//! Deterministic optimize / plan-selection integration tests.

use std::path::PathBuf;

use strata_codec::{resolve_profile_plans, EncodingStep};
use strata_core::{
    load_profile, parse_profile_str, Field, FieldValue, IntegerWidth, LogicalType, Record, Schema,
    TimestampUnit,
};
use strata_input::{discover_inputs, open_parser, ParseOptions, RecordParser};
use strata_optimize::{
    format_human, format_plan_display, optimize_dataset, select_plan, BenchResult,
    OptimizationConstraints, OptimizationGoal, OptimizeOptions,
};

fn examples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples")
}

#[test]
fn balanced_does_not_always_pick_smallest() {
    let a = BenchResult {
        plan: strata_codec::EncodingPlan::new(vec![
            EncodingStep::DeltaOfDelta,
            EncodingStep::ZigZag,
            EncodingStep::BitPacking,
        ])
        .unwrap(),
        encoded_bytes: 48_000_000,
        logical_bytes: 200_000_000,
        encode_mb_s: 800.0,
        decode_mb_s: 2_000.0,
        peak_memory_bytes: 100_000_000,
        encode_iters: 1,
        decode_iters: 1,
    };
    let b = BenchResult {
        plan: strata_codec::EncodingPlan::new(vec![EncodingStep::ZigZag, EncodingStep::BitPacking])
            .unwrap(),
        encoded_bytes: 51_000_000,
        logical_bytes: 200_000_000,
        encode_mb_s: 4_300.0,
        decode_mb_s: 2_500.0,
        peak_memory_bytes: 110_000_000,
        encode_iters: 1,
        decode_iters: 1,
    };
    let sel = select_plan(
        &[a, b],
        OptimizationGoal::Balanced,
        &OptimizationConstraints::default(),
    )
    .unwrap();
    assert_eq!(sel.result.encoded_bytes, 51_000_000);
}

#[test]
fn optimizes_monotonic_timestamps_to_delta_family() {
    let _schema = Schema::new(
        "ticks",
        vec![
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required("symbol", LogicalType::categorical_string(Some(16))),
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
    .unwrap();

    let records: Vec<Record> = (0..2_048)
        .map(|i| {
            Record::new()
                .with("ts", FieldValue::Timestamp(1_000_000 + i * 382))
                .with(
                    "symbol",
                    FieldValue::CategoricalString(format!("S{}", i % 8)),
                )
                .with(
                    "side",
                    FieldValue::Enum(if i % 2 == 0 {
                        "buy".into()
                    } else {
                        "sell".into()
                    }),
                )
                .with("price", FieldValue::FixedDecimal(10_000 + (i % 50)))
        })
        .collect();

    // Minimal profile matching the schema.
    let profile_toml = r#"
name = "fixture"
input_format = "csv"
[schema]
id = "ticks"
[[columns]]
name = "ts"
logical_type = "timestamp"
unit = "nanoseconds"
encoding = "auto"
[[columns]]
name = "symbol"
logical_type = "categorical_string"
max_bytes = 16
encoding = "auto"
[[columns]]
name = "side"
logical_type = "enum"
variants = ["buy", "sell"]
encoding = "auto"
[[columns]]
name = "price"
logical_type = "fixed_decimal"
precision = 18
scale = 4
width = "bits64"
encoding = "auto"
[block]
row_count = 1024
"#;
    let profile = parse_profile_str(profile_toml, "<fixture>").unwrap();

    let report = optimize_dataset(
        "fixture",
        &profile,
        &records,
        1,
        &OptimizeOptions {
            goal: OptimizationGoal::MaxCompression,
            sample_rows: Some(1024),
            block_rows: Some(256),
            min_bench_nanos: 50_000,
            max_bench_iters: 4,
            ..OptimizeOptions::default()
        },
    )
    .unwrap();

    let ts = report.columns.iter().find(|c| c.name == "ts").unwrap();
    assert!(
        ts.selected.plan.steps.contains(&EncodingStep::Delta)
            || ts.selected.plan.steps.contains(&EncodingStep::DeltaOfDelta)
            || ts
                .selected
                .plan
                .steps
                .contains(&EncodingStep::FrameOfReference),
        "expected a transform pipeline for monotonic timestamps, got {:?}",
        ts.selected.steps
    );
    assert!(ts.reason.contains("monotonic") || ts.reason.contains("delta"));
    assert!(format_plan_display(&ts.selected.plan).contains('→') || ts.selected.steps.len() == 1);

    let symbol = report.columns.iter().find(|c| c.name == "symbol").unwrap();
    assert!(
        symbol
            .selected
            .plan
            .steps
            .contains(&EncodingStep::DictionaryEncoding)
            || symbol.selected.plan.steps == [EncodingStep::Plain],
        "got {:?}",
        symbol.selected.steps
    );

    let side = report.columns.iter().find(|c| c.name == "side").unwrap();
    assert!(
        side.selected
            .plan
            .steps
            .contains(&EncodingStep::EnumBitpack)
            || side
                .selected
                .plan
                .steps
                .contains(&EncodingStep::DictionaryEncoding)
    );

    let human = format_human(&report);
    assert!(human.contains("ts:"));
    assert!(human.contains("selected"));
    assert!(human.contains("reason:"));

    // Emitted profile must parse and resolve.
    let emitted = parse_profile_str(&report.profile_toml, "<optimized>").unwrap();
    let plans = resolve_profile_plans(&emitted).unwrap();
    assert_eq!(plans.len(), 4);
    assert!(report.profile_toml.contains("encoding = ["));
}

#[test]
fn optimizes_example_market_csv() {
    let profile = load_profile(examples().join("profiles/stock-market.toml")).unwrap();
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

    let report = optimize_dataset(
        "market.csv",
        &profile,
        &batch.records,
        1,
        &OptimizeOptions {
            goal: OptimizationGoal::Balanced,
            sample_rows: Some(256),
            min_bench_nanos: 50_000,
            max_bench_iters: 4,
            ..OptimizeOptions::default()
        },
    )
    .unwrap();

    assert_eq!(report.columns.len(), schema.fields.len());
    let emitted = parse_profile_str(&report.profile_toml, "<opt>").unwrap();
    resolve_profile_plans(&emitted).unwrap();

    let ts = report.columns.iter().find(|c| c.name == "ts").unwrap();
    assert!(!ts.reason.is_empty());
}
