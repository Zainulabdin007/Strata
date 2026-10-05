//! Benchmarks: buffered reads vs mmap sequential scans vs mmap selective queries.
//!
//! Does not assume mmap wins. Writes measured results and records the faster
//! default recommendation for this machine/run.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use strata_codec::{compressor_for, CompressionCodec};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_query::{execute_with_options, parse_query, QueryOptions};
use strata_storage::{
    rebuild_indexes, DatasetReader, DatasetWriter, DatasetWriterOptions, SegmentReadMode,
};

fn schema() -> Schema {
    Schema::new(
        "mmap_bench",
        vec![
            Field::required("seq", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required("symbol", LogicalType::categorical_string(Some(8))),
            Field::required(
                "price",
                LogicalType::fixed_decimal(18, 4, IntegerWidth::Bits64),
            ),
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
                    FieldValue::Timestamp(1_700_000_000_000_000_000i64 + i as i64 * 1_000),
                )
                .with(
                    "symbol",
                    FieldValue::CategoricalString(symbols[(i as usize) % symbols.len()].into()),
                )
                .with("price", FieldValue::FixedDecimal(10_000 + (i % 500) as i64))
        })
        .collect()
}

#[derive(Debug, Clone)]
struct Row {
    name: &'static str,
    mode: &'static str,
    elapsed_ms: u64,
    rows: u64,
    rows_per_sec: f64,
}

fn sequential_scan(root: &std::path::Path, mode: SegmentReadMode, name: &'static str) -> Row {
    let mode_s = mode.as_str();
    // Warm-ish: open once discarded.
    let _ = DatasetReader::open_with_mode(root, mode)
        .unwrap()
        .read_all_records()
        .unwrap();

    let t0 = Instant::now();
    let reader = DatasetReader::open_with_mode(root, mode).unwrap();
    let rows = reader.read_all_records().unwrap();
    let elapsed = t0.elapsed();
    let secs = elapsed.as_secs_f64().max(1e-9);
    Row {
        name,
        mode: mode_s,
        elapsed_ms: elapsed.as_millis() as u64,
        rows: rows.len() as u64,
        rows_per_sec: rows.len() as f64 / secs,
    }
}

fn selective_query(root: &std::path::Path, mode: SegmentReadMode, name: &'static str) -> Row {
    let t1 = 1_700_000_000_000_000_000i64 + 5_000 * 1_000;
    let t2 = 1_700_000_000_000_000_000i64 + 12_000 * 1_000;
    let q = parse_query(&format!(
        "select ts, price where symbol == AAPL and ts between {t1} and {t2}"
    ))
    .unwrap();
    let opts = QueryOptions {
        use_indexes: true,
        force_full_scan: false,
        segment_read_mode: mode,
    };
    let t0 = Instant::now();
    let mut stream = execute_with_options(root, q, opts).unwrap();
    let rows = stream.collect_records().unwrap();
    let elapsed = t0.elapsed();
    let secs = elapsed.as_secs_f64().max(1e-9);
    Row {
        name,
        mode: mode.as_str(),
        elapsed_ms: elapsed.as_millis() as u64,
        rows: rows.len() as u64,
        rows_per_sec: rows.len() as f64 / secs,
    }
}

#[test]
fn mmap_vs_buffered_benchmarks() {
    let root = std::env::temp_dir().join(format!("strata-mmap-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let n = 80_000u64;
    let mut writer = DatasetWriter::create(
        &root,
        schema(),
        DatasetWriterOptions {
            block_row_count: 512,
            segment_target_size_bytes: 512 * 1024,
            compressor: Arc::from(compressor_for(CompressionCodec::Lz4, Some(0))),
            ..Default::default()
        },
    )
    .unwrap();
    writer.write_records(&make_records(n)).unwrap();
    writer.finish().unwrap();
    rebuild_indexes(&root).unwrap();

    let rows = vec![
        sequential_scan(&root, SegmentReadMode::Buffered, "buffered_sequential"),
        sequential_scan(&root, SegmentReadMode::Mmap, "mmap_sequential"),
        selective_query(&root, SegmentReadMode::Buffered, "buffered_selective"),
        selective_query(&root, SegmentReadMode::Mmap, "mmap_selective"),
    ];

    // Correctness across backends for selective path.
    assert_eq!(rows[2].rows, rows[3].rows);
    assert_eq!(rows[0].rows, n);
    assert_eq!(rows[1].rows, n);

    let seq_buf = &rows[0];
    let seq_mmap = &rows[1];
    let sel_buf = &rows[2];
    let sel_mmap = &rows[3];

    let seq_winner = if seq_mmap.elapsed_ms <= seq_buf.elapsed_ms {
        "mmap"
    } else {
        "buffered"
    };
    let sel_winner = if sel_mmap.elapsed_ms <= sel_buf.elapsed_ms {
        "mmap"
    } else {
        "buffered"
    };
    // Default remains Auto (try mmap, fall back). Document measured winners.
    let recommendation = format!(
        "Keep SegmentReadMode::Auto. Measured winners this run: sequential={seq_winner}, selective={sel_winner}."
    );

    let mut md = String::new();
    md.push_str("# Strata mmap vs buffered benchmarks\n\n");
    md.push_str(&format!("Records: {n}\n\n"));
    md.push_str(&format!("{recommendation}\n\n"));
    md.push_str("| case | mode | ms | rows | rows/s |\n");
    md.push_str("| --- | --- | ---: | ---: | ---: |\n");
    for r in &rows {
        md.push_str(&format!(
            "| {} | {} | {} | {} | {:.0} |\n",
            r.name, r.mode, r.elapsed_ms, r.rows, r.rows_per_sec
        ));
    }

    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../benchmarks");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out = out_dir.join("mmap_results.md");
    std::fs::write(&out, &md).unwrap();
    eprintln!("wrote {}\n{md}", out.display());
}
