# Fair storage comparison methodology

This document describes how `strata-bench --suite compare` measures Strata
against controlled baseline formats. **It does not claim Strata is universally
better than Parquet, SQLite, DuckDB, or any other system.**

## What is compared

The **same** deterministic logical dataset (workload + seed + size) is written
in lockstep to:

| Format | Role |
| --- | --- |
| JSONL | Text baseline (field names repeated per row) |
| CSV | Flat tabular text baseline (when schema is tabular — all current workloads) |
| Naive fixed-width binary | Simple little-endian packed rows (not a production format) |
| Parquet + Zstd | Optional columnar compressed file (`--features parquet`, default on) |
| Strata | Segment dataset with configured codec (e.g. lz4 / zstd) |

## Reported sizes

| Metric | Definition |
| --- | --- |
| Raw logical size | Sum of decoded field payloads (numeric widths + UTF-8 / byte lengths). No keys, separators, dictionaries, headers, or compression. |
| JSON size | On-disk JSONL file length |
| CSV size | On-disk CSV file length |
| Fixed binary size | On-disk `.fixed` file length (header + rows + trailer) |
| Parquet/Zstd size | On-disk `.parquet` file length (when feature enabled) |
| Strata semantic encoded size | Sum of `semantic_encoded_bytes` (fallback `encoded_bytes`) across blocks |
| Strata final compressed size | Sum of `compressed_bytes` (fallback `block_length`) across blocks |
| Strata dataset dir bytes | Recursive size of the Strata dataset directory (includes manifests, indexes if present, footers) |

## Throughput

For each format, wall-clock **encode** (write all rows) and **decode** (full
scan / parse) are measured separately where practical. Times are not
cross-normalized for CPU cache effects beyond sharing the same generator pass
for encode.

Fairness limits:

- JSONL/CSV decode parse text; they are not storage engines.
- Fixed binary is intentionally naive (padded strings).
- Parquet here is a **single-file** write with ZSTD level 3 via the Rust
  `arrow`/`parquet` crates — not a multi-file lakehouse, not DuckDB/Spark
  tuned settings, and not predicate pushdown benchmarking.
- Strata encode uses the configured ingest pipeline (workers, block size,
  codec). Decode uses `DatasetReader::read_all_records`.

## What this suite is *not*

- Not a bake-off against SQLite or DuckDB.
- Not evidence that Strata always wins on size or speed.
- Not permission to put invented numbers into docs — only measured JSON /
  `RESULTS.md` sections from an actual run.

## Run

```bash
cargo run -p strata-bench --release -- \
  --suite compare --size 100k --workload market --seed 42 --codec lz4
```

Disable Parquet baseline if needed:

```bash
cargo run -p strata-bench --release --no-default-features -- \
  --suite compare --size 100k --workload market
```
