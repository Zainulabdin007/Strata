//! Fair storage comparison suite (same logical dataset → multiple formats).

use std::fs;
use std::time::Instant;

use anyhow::Result;
use serde_json::json;
use strata_storage::{DatasetReader, IngestPipeline, PipelineOptions};

use crate::baselines::csv_baseline::CsvWriter;
use crate::baselines::fixed::{decode_fixed_typed, FixedWriter};
use crate::baselines::jsonl::{decode_jsonl, JsonlWriter};
use crate::baselines::logical::logical_bytes_of_record;
use crate::baselines::{dir_size, elapsed_ms, file_size, rps, BaselinePaths, FormatMeasurement};
use crate::config::{per_sec, BenchConfig};
use crate::gen::generate_chunk;
use crate::report::BenchReport;

#[cfg(feature = "parquet")]
use crate::baselines::parquet_baseline::{decode_parquet_zstd, ParquetZstdWriter};

use crate::baselines::csv_baseline::decode_csv;

/// Methodology string embedded in every compare report (no fabricated numbers).
pub const METHODOLOGY: &str = r#"
Fair comparison methodology (strata-bench compare):
1. One deterministic generator (workload + seed + size) produces the logical records.
2. The same chunks are written, in separate sequential runs, to each baseline and to Strata.
3. Sizes are on-disk bytes after a full encode/close (Strata = dataset directory bytes
   and separately semantic_encoded / compressed block totals from segment metadata).
4. Raw logical size is the sum of decoded field payloads (schema widths / UTF-8 lengths)
   with no keys, separators, dictionaries, headers, or compression.
5. Encode/decode throughput uses wall-clock for a full write or full scan of that format.
6. This does NOT compare Strata to SQLite, DuckDB, or a tuned Parquet lakehouse stack.
7. Parquet+Zstd (when enabled) is a single-file columnar baseline, not a claim that
   Strata is universally better or worse than Parquet.
8. Do not treat CSV/JSONL/fixed as optimized storage engines — they are controlled baselines.
"#;

pub fn run_compare(config: &BenchConfig) -> Result<BenchReport> {
    let root = config.dataset_path("compare");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root)?;
    let paths = BaselinePaths::new(root.clone());
    fs::create_dir_all(&paths.strata)?;

    let schema = config.workload.schema();
    let total = config.size.records();
    let chunk = config.block_size.max(1) as u64;

    // Run each format independently. Full wall time includes generation,
    // backpressure, worker completion and close, rather than submit-call time.
    let t_all = Instant::now();
    let feed = |write: &mut dyn FnMut(&[strata_core::Record]) -> Result<()>| -> Result<()> {
        let mut buf = Vec::new();
        let mut start = 0;
        while start < total {
            let n = chunk.min(total - start);
            generate_chunk(config.workload, config.seed, start, n, &mut buf);
            write(&buf)?;
            start += n;
        }
        Ok(())
    };
    let t = Instant::now();
    let mut jsonl = JsonlWriter::create(&paths.jsonl)?;
    let mut raw_logical = 0;
    feed(&mut |rows| {
        for r in rows {
            raw_logical += logical_bytes_of_record(&schema, r);
        }
        jsonl.write_chunk(&schema, rows)
    })?;
    jsonl.finish()?;
    let jsonl_encode = t.elapsed();
    let t = Instant::now();
    let mut csv = CsvWriter::create(&paths.csv)?;
    feed(&mut |rows| csv.write_chunk(&schema, rows))?;
    csv.finish()?;
    let csv_encode = t.elapsed();
    let t = Instant::now();
    let mut fixed = FixedWriter::create(&paths.fixed, &schema)?;
    feed(&mut |rows| fixed.write_chunk(rows))?;
    fixed.finish()?;
    let fixed_encode = t.elapsed();
    #[cfg(feature = "parquet")]
    let parquet_encode = {
        let t = Instant::now();
        let mut parquet = ParquetZstdWriter::create(&paths.parquet, &schema)?;
        feed(&mut |rows| parquet.write_chunk(rows))?;
        parquet.finish()?;
        t.elapsed()
    };
    let t = Instant::now();
    let mut pipe_opts = PipelineOptions::with_workers(config.workers);
    pipe_opts.writer = config.writer_options(None, false);
    let mut strata = IngestPipeline::start(&paths.strata, schema.clone(), pipe_opts)?;
    let mut start = 0;
    let mut buf = Vec::new();
    while start < total {
        let n = chunk.min(total - start);
        generate_chunk(config.workload, config.seed, start, n, &mut buf);
        strata.submit(std::mem::take(&mut buf))?;
        start += n;
    }
    strata.finish()?;
    let strata_encode = t.elapsed();
    let wall = t_all.elapsed();

    // Sizes
    let json_size = file_size(&paths.jsonl)?;
    let csv_size = file_size(&paths.csv)?;
    let fixed_size = file_size(&paths.fixed)?;
    #[cfg(feature = "parquet")]
    let parquet_size = file_size(&paths.parquet)?;
    let strata_dir_size = dir_size(&paths.strata)?;

    let reader = DatasetReader::open(&paths.strata)?;
    let mut semantic = 0u64;
    let mut compressed = 0u64;
    let mut logical_meta = 0u64;
    for i in 0..reader.segment_count() {
        let seg = reader.open_segment(i)?;
        for block in &seg.meta().blocks {
            logical_meta += block.logical_bytes;
            semantic += block.semantic_encoded_bytes;
            compressed += block.compressed_bytes;
        }
    }

    // Decode timings (full scan).
    let (jsonl_n, jsonl_dec) = decode_jsonl(&paths.jsonl)?;
    let (csv_n, csv_dec) = decode_csv(&paths.csv)?;
    let (fixed_n, fixed_dec) = decode_fixed_typed(&paths.fixed, &schema)?;
    #[cfg(feature = "parquet")]
    let (parquet_n, parquet_dec) = decode_parquet_zstd(&paths.parquet)?;

    let t0 = Instant::now();
    let mut strata_rows = 0;
    for i in 0..reader.segment_count() {
        let seg = reader.open_segment(i)?;
        for b in 0..seg.block_count() {
            let batch = seg.read_block(b)?;
            strata_rows += batch.len() as u64;
            std::hint::black_box(batch);
        }
    }
    anyhow::ensure!(
        [jsonl_n, csv_n, fixed_n, strata_rows]
            .iter()
            .all(|n| *n == total),
        "comparison lost rows"
    );
    #[cfg(feature = "parquet")]
    anyhow::ensure!(parquet_n == total, "Parquet lost rows");
    let strata_dec = t0.elapsed();

    let mut formats = vec![
        FormatMeasurement {
            format: "jsonl".into(),
            path: paths.jsonl.display().to_string(),
            size_bytes: json_size,
            encode_elapsed_ms: elapsed_ms(jsonl_encode),
            encode_records_per_sec: rps(total, jsonl_encode),
            decode_elapsed_ms: Some(elapsed_ms(jsonl_dec)),
            decode_records_per_sec: Some(rps(jsonl_n, jsonl_dec)),
            decode_records: Some(jsonl_n),
            notes: "NDJSON; text; field names per row".into(),
        },
        FormatMeasurement {
            format: "csv".into(),
            path: paths.csv.display().to_string(),
            size_bytes: csv_size,
            encode_elapsed_ms: elapsed_ms(csv_encode),
            encode_records_per_sec: rps(total, csv_encode),
            decode_elapsed_ms: Some(elapsed_ms(csv_dec)),
            decode_records_per_sec: Some(rps(csv_n, csv_dec)),
            decode_records: Some(csv_n),
            notes: "flat tabular CSV with header".into(),
        },
        FormatMeasurement {
            format: "fixed_binary".into(),
            path: paths.fixed.display().to_string(),
            size_bytes: fixed_size,
            encode_elapsed_ms: elapsed_ms(fixed_encode),
            encode_records_per_sec: rps(total, fixed_encode),
            decode_elapsed_ms: Some(elapsed_ms(fixed_dec)),
            decode_records_per_sec: Some(rps(fixed_n, fixed_dec)),
            decode_records: Some(fixed_n),
            notes: "naive little-endian fixed slots; not a production format".into(),
        },
    ];

    #[cfg(feature = "parquet")]
    formats.push(FormatMeasurement {
        format: "parquet_zstd".into(),
        path: paths.parquet.display().to_string(),
        size_bytes: parquet_size,
        encode_elapsed_ms: elapsed_ms(parquet_encode),
        encode_records_per_sec: rps(total, parquet_encode),
        decode_elapsed_ms: Some(elapsed_ms(parquet_dec)),
        decode_records_per_sec: Some(rps(parquet_n, parquet_dec)),
        decode_records: Some(parquet_n),
        notes: "Apache Parquet + ZSTD level 3 via arrow/parquet; single file".into(),
    });

    formats.push(FormatMeasurement {
        format: "strata".into(),
        path: paths.strata.display().to_string(),
        size_bytes: strata_dir_size,
        encode_elapsed_ms: elapsed_ms(strata_encode),
        encode_records_per_sec: rps(total, strata_encode),
        decode_elapsed_ms: Some(elapsed_ms(strata_dec)),
        decode_records_per_sec: Some(rps(strata_rows, strata_dec)),
        decode_records: Some(strata_rows),
        notes: format!(
            "Strata dataset dir; codec={}; semantic_encoded and compressed reported separately",
            config.codec.as_str()
        ),
    });

    let parquet_size_json = {
        #[cfg(feature = "parquet")]
        {
            json!(parquet_size)
        }
        #[cfg(not(feature = "parquet"))]
        {
            json!(null)
        }
    };

    let metrics = json!({
        "methodology": METHODOLOGY.trim(),
        "disclaimer": "Do not claim Strata is universally better than Parquet, SQLite, DuckDB, or another system based on this suite alone.",
        "records": total,
        "raw_logical_bytes": raw_logical,
        "strata_block_logical_bytes": logical_meta,
        "json_size_bytes": json_size,
        "csv_size_bytes": csv_size,
        "fixed_binary_size_bytes": fixed_size,
        "parquet_zstd_size_bytes": parquet_size_json,
        "strata_semantic_encoded_bytes": semantic,
        "strata_final_compressed_bytes": compressed,
        "strata_dataset_dir_bytes": strata_dir_size,
        "wall_elapsed_ms": elapsed_ms(wall),
        "formats": formats,
        "sanity": {
            "jsonl_records": jsonl_n,
            "csv_records": csv_n,
            "fixed_records": fixed_n,
            "strata_records": strata_rows,
            "expected_records": total,
        }
    });

    let _ = per_sec(total, wall);

    Ok(BenchReport {
        suite: "compare".into(),
        meta: config.meta(),
        metrics,
    })
}
