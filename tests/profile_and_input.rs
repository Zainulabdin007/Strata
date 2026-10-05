//! Profile loading, discovery, and parser integration tests.

use std::fs;
use std::path::{Path, PathBuf};

use strata_core::{load_profile, parse_profile_str, Error as CoreError, SemanticHint};
use strata_input::{
    discover_inputs, open_parser, CsvParser, Error as InputError, JsonlParser, ParseOptions,
    RecordParser,
};

fn fixtures_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples")
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "strata-it-{label}-{}-{}",
        std::process::id(),
        label.len()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn valid_stock_market_profile() {
    let path = fixtures_root().join("profiles/stock-market.toml");
    let profile = load_profile(&path).expect("load stock-market.toml");
    assert_eq!(profile.name, "stock-market");
    assert_eq!(profile.columns.len(), 8);
    assert!(profile.columns[0].has_hint(SemanticHint::Monotonic));
    let schema = profile.schema().unwrap();
    assert_eq!(schema.id.as_str(), "stock_ticks_v1");
    let plans = strata_codec::resolve_profile_plans(&profile).expect("resolve encodings");
    assert_eq!(
        plans["ts"].steps,
        vec![
            strata_codec::EncodingStep::DeltaOfDelta,
            strata_codec::EncodingStep::ZigZag,
            strata_codec::EncodingStep::BitPacking
        ]
    );
    assert_eq!(
        plans["symbol"].steps,
        vec![
            strata_codec::EncodingStep::DictionaryEncoding,
            strata_codec::EncodingStep::BitPacking
        ]
    );
}

#[test]
fn rejects_incompatible_encoding_pipeline() {
    let err = strata_codec::resolve_profile_plans(
        &parse_profile_str(
            r#"
name = "bad"
input_format = "csv"
[schema]
id = "x"
[[columns]]
name = "symbol"
logical_type = "categorical_string"
encoding = ["varint"]
"#,
            "bad-enc.toml",
        )
        .unwrap(),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("incompatible") || err.to_string().contains("terminal"),
        "{err}"
    );
}

#[test]
fn valid_telemetry_profile() {
    let path = fixtures_root().join("profiles/telemetry.toml");
    let profile = load_profile(&path).expect("load telemetry.toml");
    assert_eq!(profile.input_format.as_str(), "jsonl");
    assert_eq!(profile.block.row_count, 32768);
}

#[test]
fn invalid_logical_type() {
    let err = parse_profile_str(
        r#"
name = "bad"
input_format = "csv"
[schema]
id = "x"
[[columns]]
name = "a"
logical_type = "varchar"
"#,
        "bad-type.toml",
    )
    .unwrap_err();
    match err {
        CoreError::ProfileInvalid { message, .. } => {
            assert!(message.contains("unknown logical type"), "{message}");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn missing_columns() {
    let err = parse_profile_str(
        r#"
name = "bad"
input_format = "csv"
columns = []

[schema]
id = "x"
"#,
        "missing-cols.toml",
    )
    .unwrap_err();
    assert!(err.to_string().contains("at least one column"), "{err}");
}

#[test]
fn invalid_profile_file() {
    let path = temp_dir("missing-profile").join("nope.toml");
    let err = load_profile(&path).unwrap_err();
    assert!(matches!(err, CoreError::ProfileIo { .. }), "{err}");
}

#[test]
fn directory_ingestion_discovery() {
    let root = temp_dir("discover");
    fs::create_dir_all(root.join("nested")).unwrap();
    fs::write(root.join("a.csv"), "id\n1\n").unwrap();
    fs::write(root.join("nested/b.csv"), "id\n2\n").unwrap();
    fs::write(root.join("nested/c.txt"), "ignore\n").unwrap();

    let profile = parse_profile_str(
        r#"
name = "t"
input_format = "csv"
[input]
recursive = true
[schema]
id = "s"
[[columns]]
name = "id"
logical_type = "unsigned_integer"
width = "bits64"
"#,
        "t.toml",
    )
    .unwrap();

    let found = discover_inputs(&root, &profile).unwrap();
    assert_eq!(found.files.len(), 2);
}

#[test]
fn unsupported_extension() {
    let root = temp_dir("bad-ext");
    let file = root.join("ticks.parquet");
    fs::write(&file, "not parquet").unwrap();

    let profile = parse_profile_str(
        r#"
name = "t"
input_format = "csv"
[schema]
id = "s"
[[columns]]
name = "id"
logical_type = "unsigned_integer"
width = "bits64"
"#,
        "t.toml",
    )
    .unwrap();

    let err = discover_inputs(&file, &profile).unwrap_err();
    assert!(
        matches!(err, InputError::UnsupportedExtension { .. }),
        "{err}"
    );
}

#[test]
fn malformed_csv() {
    let dir = temp_dir("malformed-csv");
    let path = dir.join("bad.csv");
    fs::write(&path, "id,symbol\n1\n").unwrap();

    let profile = parse_profile_str(
        r#"
name = "t"
input_format = "csv"
[schema]
id = "s"
[[columns]]
name = "id"
logical_type = "unsigned_integer"
width = "bits64"
[[columns]]
name = "symbol"
logical_type = "categorical_string"
"#,
        "t.toml",
    )
    .unwrap();
    let schema = profile.schema().unwrap();
    let err = CsvParser::new()
        .parse_file(&path, &schema, &ParseOptions::default())
        .unwrap_err();
    assert!(matches!(err, InputError::MalformedCsv { .. }), "{err}");
}

#[test]
fn malformed_jsonl() {
    let dir = temp_dir("malformed-jsonl");
    let path = dir.join("bad.jsonl");
    fs::write(&path, "{ok:false}\n").unwrap();

    let profile = parse_profile_str(
        r#"
name = "t"
input_format = "jsonl"
[schema]
id = "s"
[[columns]]
name = "ok"
logical_type = "boolean"
"#,
        "t.toml",
    )
    .unwrap();
    let schema = profile.schema().unwrap();
    let err = JsonlParser::new()
        .parse_file(&path, &schema, &ParseOptions::default())
        .unwrap_err();
    assert!(matches!(err, InputError::MalformedJsonl { .. }), "{err}");
}

#[test]
fn parses_example_market_csv() {
    let profile = load_profile(fixtures_root().join("profiles/stock-market.toml")).unwrap();
    let schema = profile.schema().unwrap();
    let path = fixtures_root().join("data/market.csv");
    let batch = open_parser(&profile)
        .unwrap()
        .parse_file(&path, &schema, &ParseOptions::default())
        .unwrap();
    assert_eq!(batch.len(), 3);
}

#[test]
fn parses_example_telemetry_jsonl() {
    let profile = load_profile(fixtures_root().join("profiles/telemetry.toml")).unwrap();
    let schema = profile.schema().unwrap();
    let path = fixtures_root().join("data/telemetry.jsonl");
    let batch = open_parser(&profile)
        .unwrap()
        .parse_file(&path, &schema, &ParseOptions::default())
        .unwrap();
    assert_eq!(batch.len(), 3);
}

#[test]
fn clear_error_for_missing_csv_column() {
    let dir = temp_dir("missing-col");
    let path = dir.join("partial.csv");
    fs::write(&path, "id\n1\n").unwrap();

    let profile = parse_profile_str(
        r#"
name = "t"
input_format = "csv"
[schema]
id = "s"
[[columns]]
name = "id"
logical_type = "unsigned_integer"
width = "bits64"
[[columns]]
name = "symbol"
logical_type = "categorical_string"
"#,
        "t.toml",
    )
    .unwrap();
    let schema = profile.schema().unwrap();
    let err = CsvParser::new()
        .parse_file(&path, &schema, &ParseOptions::default())
        .unwrap_err();
    assert!(matches!(err, InputError::MissingColumn { .. }), "{err}");
}
