//! Query engine benchmarks: full scan, indexed scan, projection, range, symbol/time.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use strata_codec::{compressor_for, CompressionCodec};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_query::{execute, execute_with_options, parse_query, QueryOptions};
use strata_storage::{rebuild_indexes, DatasetWriter, DatasetWriterOptions};

fn schema() -> Schema {
    Schema::new(
        "bench_query",
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
    let symbols = [
        "AAPL", "MSFT", "GOOG", "AMZN", "META", "NVDA", "TSLA", "AMD",
    ];
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
                .with("price", FieldValue::FixedDecimal(10_000 + (i % 500) as i64))
                .with("size", FieldValue::UnsignedInteger(1 + (i % 64)))
        })
        .collect()
}

#[derive(Debug)]
struct BenchRow {
    name: &'static str,
    elapsed_ms: u64,
    rows: u64,
    blocks_scanned: u64,
    blocks_skipped: u64,
    rows_per_sec: f64,
}

fn run_case(root: &std::path::Path, name: &'static str, query: &str, full_scan: bool) -> BenchRow {
    let q = parse_query(query).unwrap();
    let opts = QueryOptions {
        use_indexes: !full_scan,
        force_full_scan: full_scan,
        ..Default::default()
    };
    let t0 = Instant::now();
    let mut stream = execute_with_options(root, q, opts).unwrap();
    let rows = stream.collect_records().unwrap();
    let elapsed = t0.elapsed();
    let stats = stream.stats().clone();
    let ms = elapsed.as_millis() as u64;
    let secs = elapsed.as_secs_f64().max(1e-9);
    BenchRow {
        name,
        elapsed_ms: ms,
        rows: rows.len() as u64,
        blocks_scanned: stats.blocks_scanned,
        blocks_skipped: stats.blocks_skipped,
        rows_per_sec: rows.len() as f64 / secs,
    }
}

#[test]
fn query_benchmarks() {
    let root = std::env::temp_dir().join(format!("strata-q-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let n = 50_000u64;
    let mut writer = DatasetWriter::create(
        &root,
        schema(),
        DatasetWriterOptions {
            block_row_count: 512,
            segment_target_size_bytes: 256 * 1024,
            compressor: Arc::from(compressor_for(CompressionCodec::Lz4, Some(0))),
            ..Default::default()
        },
    )
    .unwrap();
    writer.write_records(&make_records(n)).unwrap();
    writer.finish().unwrap();
    rebuild_indexes(&root).unwrap();

    let t1 = 1_700_000_000_000_000_000i64 + 10_000 * 1_000;
    let t2 = 1_700_000_000_000_000_000i64 + 15_000 * 1_000;

    let rows = vec![
        run_case(&root, "full_scan", "select *", true),
        run_case(&root, "indexed_scan_all", "select *", false),
        run_case(&root, "projection", "select ts, price, size", false),
        run_case(
            &root,
            "range_seq",
            "select seq, price where seq between 10000 and 15000",
            false,
        ),
        run_case(
            &root,
            "symbol_time",
            &format!("select ts, price, size where symbol == AAPL and ts between {t1} and {t2}"),
            false,
        ),
        run_case(
            &root,
            "symbol_time_full_scan",
            &format!("select ts, price, size where symbol == AAPL and ts between {t1} and {t2}"),
            true,
        ),
    ];

    // Sanity: selective queries skip blocks.
    let selective = rows.iter().find(|r| r.name == "symbol_time").unwrap();
    let full = rows
        .iter()
        .find(|r| r.name == "symbol_time_full_scan")
        .unwrap();
    assert!(
        selective.blocks_scanned < full.blocks_scanned,
        "indexed symbol/time should scan fewer blocks ({} vs {})",
        selective.blocks_scanned,
        full.blocks_scanned
    );
    assert_eq!(selective.rows, full.rows);

    let mut md = String::new();
    md.push_str("# Strata query benchmarks\n\n");
    md.push_str(&format!("Records: {n}\n\n"));
    md.push_str("| case | ms | rows | blocks_scanned | blocks_skipped | rows/s |\n");
    md.push_str("| --- | ---: | ---: | ---: | ---: | ---: |\n");
    for r in &rows {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.0} |\n",
            r.name, r.elapsed_ms, r.rows, r.blocks_scanned, r.blocks_skipped, r.rows_per_sec
        ));
    }

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../benchmarks");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out = out_dir.join("query_results.md");
    std::fs::write(&out, md).unwrap();
    eprintln!("wrote {}", out.display());

    // Keep `execute` linked for API surface in benches.
    let _ = execute(
        &root,
        parse_query("select seq where seq == 0 limit 1").unwrap(),
    );
}
