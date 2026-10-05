# Strata bench results

Measured runs only. Re-run `strata-bench` to refresh.

## ingest / market / 100k

- git: unknown
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)
- profile: release (debug_assertions=false)
- compiler: opt-level>=1 (release/no debug_assertions)
- os/arch: macos/aarch64
- seed: 42
- workers: 18
- block_size: 65536
- segment_size_bytes: 268435456
- codec: lz4
- records: 100000

```json
{
  "elapsed_ms": 113,
  "logical_bytes": 5240195,
  "logical_gb_per_sec": 0.042935184875379284,
  "metrics_records_per_sec": 884955.7522123894,
  "physical_bytes": 786640,
  "physical_mb_per_sec": 6.5999686351933855,
  "records": 100000,
  "records_per_sec": 879763.1332778068
}
```

## compression / market / 100k

- git: unknown
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)
- profile: release (debug_assertions=false)
- compiler: opt-level>=1 (release/no debug_assertions)
- os/arch: macos/aarch64
- seed: 42
- workers: 18
- block_size: 65536
- segment_size_bytes: 268435456
- codec: lz4
- records: 100000

```json
{
  "bits_per_value": 7.83224,
  "column_blocks_observed": 16,
  "compressed_bytes": 783224,
  "compression_ratio_logical_over_compressed": 6.690544467483121,
  "encoded_semantic_bytes": 1650876,
  "raw_logical_bytes": 5240195,
  "records": 100000,
  "schema_columns": 8,
  "segments": 1
}
```

## decode / market / 100k

- git: unknown
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)
- profile: release (debug_assertions=false)
- compiler: opt-level>=1 (release/no debug_assertions)
- os/arch: macos/aarch64
- seed: 42
- workers: 18
- block_size: 65536
- segment_size_bytes: 268435456
- codec: lz4
- records: 100000

```json
{
  "elapsed_ms": 74,
  "logical_bytes": 5240195,
  "logical_gb_per_sec": 0.06544370873697139,
  "records": 100000,
  "records_per_sec": 1340973.898653779,
  "segment_read_mode": "auto"
}
```

## query / market / 100k

- git: unknown
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)
- profile: release (debug_assertions=false)
- compiler: opt-level>=1 (release/no debug_assertions)
- os/arch: macos/aarch64
- seed: 42
- workers: 18
- block_size: 65536
- segment_size_bytes: 268435456
- codec: lz4
- records: 100000

```json
{
  "full_scan": {
    "elapsed_ms": 151,
    "records": 100000,
    "records_per_sec": 659634.9580142349
  },
  "queries": [
    {
      "blocks_scanned": 2,
      "blocks_skipped": 0,
      "latency_ms_p50": 37.421583,
      "latency_ms_p95": 38.990125,
      "latency_ms_p99": 38.990125,
      "name": "symbol_eq",
      "query": "select timestamp, price, quantity where symbol == AAPL",
      "rows": 10007,
      "samples": 11,
      "scan_rows_per_sec_at_p50": 267412.5250126378
    },
    {
      "blocks_scanned": 2,
      "blocks_skipped": 0,
      "latency_ms_p50": 24.406375,
      "latency_ms_p95": 24.947,
      "latency_ms_p99": 24.947,
      "name": "symbol_time",
      "query": "select timestamp, price where symbol == AAPL and timestamp between 1700000000000000000 and 1700000000100000000",
      "rows": 10007,
      "samples": 11,
      "scan_rows_per_sec_at_p50": 410015.8257832226
    },
    {
      "blocks_scanned": 1,
      "blocks_skipped": 1,
      "latency_ms_p50": 15.328375,
      "latency_ms_p95": 15.743125000000001,
      "latency_ms_p99": 15.743125000000001,
      "name": "sequence_range",
      "query": "select sequence, symbol, price where sequence between 1000 and 5000",
      "rows": 4001,
      "samples": 11,
      "scan_rows_per_sec_at_p50": 261019.18827011995
    }
  ]
}
```

## recovery / market / 100k

- git: unknown
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)
- profile: release (debug_assertions=false)
- compiler: opt-level>=1 (release/no debug_assertions)
- os/arch: macos/aarch64
- seed: 42
- workers: 18
- block_size: 65536
- segment_size_bytes: 268435456
- codec: lz4
- records: 100000

```json
{
  "manifest_rebuilt": true,
  "recover_elapsed_ms": 103,
  "recovered_records": 100000,
  "segments_seen": 1,
  "truncate_from_bytes": 784649,
  "truncate_to_bytes": 784601,
  "verify_elapsed_ms": 76,
  "verify_healthy_records": 100000,
  "verify_ok": true
}
```

## compaction / market / 100k

- git: unknown
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)
- profile: release (debug_assertions=false)
- compiler: opt-level>=1 (release/no debug_assertions)
- os/arch: macos/aarch64
- seed: 42
- workers: 18
- block_size: 65536
- segment_size_bytes: 268435456
- codec: lz4
- records: 100000

```json
{
  "compaction_generation": 1,
  "elapsed_ms": 15612,
  "indexes_rebuilt": true,
  "logical_records_after": 49048,
  "logical_records_before": 49048,
  "logical_state_preserved": true,
  "physical_records_after": 49048,
  "physical_records_before": 104286,
  "segments_after": 1,
  "workload_keys": "order_id"
}
```

## compare / market / 100k

- git: unknown
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)
- profile: release (debug_assertions=false)
- compiler: opt-level>=1 (release/no debug_assertions)
- os/arch: macos/aarch64
- seed: 42
- workers: 18
- block_size: 65536
- segment_size_bytes: 268435456
- codec: lz4
- records: 100000

```json
{
  "csv_size_bytes": 5918439,
  "disclaimer": "Do not claim Strata is universally better than Parquet, SQLite, DuckDB, or another system based on this suite alone.",
  "fixed_binary_size_bytes": 5500114,
  "formats": [
    {
      "decode_elapsed_ms": 44,
      "decode_records": 100000,
      "decode_records_per_sec": 2268070.871680647,
      "encode_elapsed_ms": 51,
      "encode_records_per_sec": 1931287.9796230576,
      "format": "jsonl",
      "notes": "NDJSON; text; field names per row",
      "path": "/tmp/strata-bench-compare/compare-market-100k-42/baseline.jsonl",
      "size_bytes": 14918373
    },
    {
      "decode_elapsed_ms": 11,
      "decode_records": 100000,
      "decode_records_per_sec": 9052026.52243771,
      "encode_elapsed_ms": 33,
      "encode_records_per_sec": 3028192.471913515,
      "format": "csv",
      "notes": "flat tabular CSV with header",
      "path": "/tmp/strata-bench-compare/compare-market-100k-42/baseline.csv",
      "size_bytes": 5918439
    },
    {
      "decode_elapsed_ms": 0,
      "decode_records": 100000,
      "decode_records_per_sec": 196963221.0577319,
      "encode_elapsed_ms": 12,
      "encode_records_per_sec": 8329486.498818629,
      "format": "fixed_binary",
      "notes": "naive little-endian fixed slots; not a production format",
      "path": "/tmp/strata-bench-compare/compare-market-100k-42/baseline.fixed",
      "size_bytes": 5500114
    },
    {
      "decode_elapsed_ms": 4,
      "decode_records": 100000,
      "decode_records_per_sec": 20116340.845646713,
      "encode_elapsed_ms": 40,
      "encode_records_per_sec": 2485938.908051335,
      "format": "parquet_zstd",
      "notes": "Apache Parquet + ZSTD level 3 via arrow/parquet; single file",
      "path": "/tmp/strata-bench-compare/compare-market-100k-42/baseline.parquet",
      "size_bytes": 1338721
    },
    {
      "decode_elapsed_ms": 90,
      "decode_records": 100000,
      "decode_records_per_sec": 1103394.5093500325,
      "encode_elapsed_ms": 0,
      "encode_records_per_sec": 9757049468.240805,
      "format": "strata",
      "notes": "Strata dataset dir; codec=lz4; semantic_encoded and compressed reported separately",
      "path": "/tmp/strata-bench-compare/compare-market-100k-42/strata",
      "size_bytes": 786640
    }
  ],
  "json_size_bytes": 14918373,
  "methodology": "Fair comparison methodology (strata-bench compare):\n1. One deterministic generator (workload + seed + size) produces the logical records.\n2. The same chunks are written, in lockstep, to each baseline and to Strata.\n3. Sizes are on-disk bytes after a full encode/close (Strata = dataset directory bytes\n   and separately semantic_encoded / compressed block totals from segment metadata).\n4. Raw logical size is the sum of decoded field payloads (schema widths / UTF-8 lengths)\n   with no keys, separators, dictionaries, headers, or compression.\n5. Encode/decode throughput uses wall-clock for a full write or full scan of that format.\n6. This does NOT compare Strata to SQLite, DuckDB, or a tuned Parquet lakehouse stack.\n7. Parquet+Zstd (when enabled) is a single-file columnar baseline, not a claim that\n   Strata is universally better or worse than Parquet.\n8. Do not treat CSV/JSONL/fixed as optimized storage engines — they are controlled baselines.",
  "parquet_zstd_size_bytes": 1338721,
  "raw_logical_bytes": 4840195,
  "records": 100000,
  "sanity": {
    "csv_records": 100000,
    "expected_records": 100000,
    "fixed_records": 100000,
    "jsonl_records": 100000,
    "strata_records": 100000
  },
  "strata_block_logical_bytes": 5240195,
  "strata_dataset_dir_bytes": 786640,
  "strata_final_compressed_bytes": 783224,
  "strata_semantic_encoded_bytes": 1650876,
  "wall_elapsed_ms": 232
}
```

## compare / market / 100k

- git: unknown
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)
- profile: release (debug_assertions=false)
- compiler: opt-level>=1 (release/no debug_assertions)
- os/arch: macos/aarch64
- seed: 42
- workers: 18
- block_size: 65536
- segment_size_bytes: 268435456
- codec: lz4
- records: 100000

```json
{
  "csv_size_bytes": 5918439,
  "disclaimer": "Do not claim Strata is universally better than Parquet, SQLite, DuckDB, or another system based on this suite alone.",
  "fixed_binary_size_bytes": 5500114,
  "formats": [
    {
      "decode_elapsed_ms": 43,
      "decode_records": 100000,
      "decode_records_per_sec": 2279442.562321384,
      "encode_elapsed_ms": 53,
      "encode_records_per_sec": 1859513.7371577332,
      "format": "jsonl",
      "notes": "NDJSON; text; field names per row",
      "path": "/tmp/strata-bench-compare2/compare-market-100k-42/baseline.jsonl",
      "size_bytes": 14918373
    },
    {
      "decode_elapsed_ms": 9,
      "decode_records": 100000,
      "decode_records_per_sec": 10068042.854020325,
      "encode_elapsed_ms": 33,
      "encode_records_per_sec": 2997489.6024579415,
      "format": "csv",
      "notes": "flat tabular CSV with header",
      "path": "/tmp/strata-bench-compare2/compare-market-100k-42/baseline.csv",
      "size_bytes": 5918439
    },
    {
      "decode_elapsed_ms": 0,
      "decode_records": 100000,
      "decode_records_per_sec": 190703218.11680573,
      "encode_elapsed_ms": 12,
      "encode_records_per_sec": 8078224.354586226,
      "format": "fixed_binary",
      "notes": "naive little-endian fixed slots; not a production format",
      "path": "/tmp/strata-bench-compare2/compare-market-100k-42/baseline.fixed",
      "size_bytes": 5500114
    },
    {
      "decode_elapsed_ms": 5,
      "decode_records": 100000,
      "decode_records_per_sec": 19601597.530198712,
      "encode_elapsed_ms": 51,
      "encode_records_per_sec": 1947490.779848339,
      "format": "parquet_zstd",
      "notes": "Apache Parquet + ZSTD level 3 via arrow/parquet; single file",
      "path": "/tmp/strata-bench-compare2/compare-market-100k-42/baseline.parquet",
      "size_bytes": 1338721
    },
    {
      "decode_elapsed_ms": 91,
      "decode_records": 100000,
      "decode_records_per_sec": 1096326.442512347,
      "encode_elapsed_ms": 21,
      "encode_records_per_sec": 4660963.387573275,
      "format": "strata",
      "notes": "Strata dataset dir; codec=lz4; semantic_encoded and compressed reported separately",
      "path": "/tmp/strata-bench-compare2/compare-market-100k-42/strata",
      "size_bytes": 786640
    }
  ],
  "json_size_bytes": 14918373,
  "methodology": "Fair comparison methodology (strata-bench compare):\n1. One deterministic generator (workload + seed + size) produces the logical records.\n2. The same chunks are written, in lockstep, to each baseline and to Strata.\n3. Sizes are on-disk bytes after a full encode/close (Strata = dataset directory bytes\n   and separately semantic_encoded / compressed block totals from segment metadata).\n4. Raw logical size is the sum of decoded field payloads (schema widths / UTF-8 lengths)\n   with no keys, separators, dictionaries, headers, or compression.\n5. Encode/decode throughput uses wall-clock for a full write or full scan of that format.\n6. This does NOT compare Strata to SQLite, DuckDB, or a tuned Parquet lakehouse stack.\n7. Parquet+Zstd (when enabled) is a single-file columnar baseline, not a claim that\n   Strata is universally better or worse than Parquet.\n8. Do not treat CSV/JSONL/fixed as optimized storage engines — they are controlled baselines.",
  "parquet_zstd_size_bytes": 1338721,
  "raw_logical_bytes": 4840195,
  "records": 100000,
  "sanity": {
    "csv_records": 100000,
    "expected_records": 100000,
    "fixed_records": 100000,
    "jsonl_records": 100000,
    "strata_records": 100000
  },
  "strata_block_logical_bytes": 5240195,
  "strata_dataset_dir_bytes": 786640,
  "strata_final_compressed_bytes": 783224,
  "strata_semantic_encoded_bytes": 1650876,
  "wall_elapsed_ms": 245
}
```

## compare / market / 100k

- git: unknown
- rustc: rustc 1.98.1 (48a229cea 2026-09-01)
- profile: release (debug_assertions=false)
- compiler: opt-level>=1 (release/no debug_assertions)
- os/arch: macos/aarch64
- seed: 42
- workers: 18
- block_size: 65536
- segment_size_bytes: 268435456
- codec: lz4
- records: 100000

```json
{
  "csv_size_bytes": 5918439,
  "disclaimer": "Do not claim Strata is universally better than Parquet, SQLite, DuckDB, or another system based on this suite alone.",
  "fixed_binary_size_bytes": 5500114,
  "formats": [
    {
      "decode_elapsed_ms": 44,
      "decode_records": 100000,
      "decode_records_per_sec": 2259814.6405119635,
      "encode_elapsed_ms": 53,
      "encode_records_per_sec": 1865304.810111877,
      "format": "jsonl",
      "notes": "NDJSON; text; field names per row",
      "path": "/var/folders/pz/ncdtccx12f7cblfxr9gtwvz80000gn/T/strata-bench-83438/compare-market-100k-42/baseline.jsonl",
      "size_bytes": 14918373
    },
    {
      "decode_elapsed_ms": 10,
      "decode_records": 100000,
      "decode_records_per_sec": 9806004.863974532,
      "encode_elapsed_ms": 34,
      "encode_records_per_sec": 2926561.5664268606,
      "format": "csv",
      "notes": "flat tabular CSV with header",
      "path": "/var/folders/pz/ncdtccx12f7cblfxr9gtwvz80000gn/T/strata-bench-83438/compare-market-100k-42/baseline.csv",
      "size_bytes": 5918439
    },
    {
      "decode_elapsed_ms": 0,
      "decode_records": 100000,
      "decode_records_per_sec": 171845813.0626876,
      "encode_elapsed_ms": 13,
      "encode_records_per_sec": 7408916.631165608,
      "format": "fixed_binary",
      "notes": "naive little-endian fixed slots; not a production format",
      "path": "/var/folders/pz/ncdtccx12f7cblfxr9gtwvz80000gn/T/strata-bench-83438/compare-market-100k-42/baseline.fixed",
      "size_bytes": 5500114
    },
    {
      "decode_elapsed_ms": 5,
      "decode_records": 100000,
      "decode_records_per_sec": 19156475.262189887,
      "encode_elapsed_ms": 51,
      "encode_records_per_sec": 1949181.562052273,
      "format": "parquet_zstd",
      "notes": "Apache Parquet + ZSTD level 3 via arrow/parquet; single file",
      "path": "/var/folders/pz/ncdtccx12f7cblfxr9gtwvz80000gn/T/strata-bench-83438/compare-market-100k-42/baseline.parquet",
      "size_bytes": 1338721
    },
    {
      "decode_elapsed_ms": 93,
      "decode_records": 100000,
      "decode_records_per_sec": 1075102.638705039,
      "encode_elapsed_ms": 26,
      "encode_records_per_sec": 3832096.536949688,
      "format": "strata",
      "notes": "Strata dataset dir; codec=lz4; semantic_encoded and compressed reported separately",
      "path": "/var/folders/pz/ncdtccx12f7cblfxr9gtwvz80000gn/T/strata-bench-83438/compare-market-100k-42/strata",
      "size_bytes": 786640
    }
  ],
  "json_size_bytes": 14918373,
  "methodology": "Fair comparison methodology (strata-bench compare):\n1. One deterministic generator (workload + seed + size) produces the logical records.\n2. The same chunks are written, in lockstep, to each baseline and to Strata.\n3. Sizes are on-disk bytes after a full encode/close (Strata = dataset directory bytes\n   and separately semantic_encoded / compressed block totals from segment metadata).\n4. Raw logical size is the sum of decoded field payloads (schema widths / UTF-8 lengths)\n   with no keys, separators, dictionaries, headers, or compression.\n5. Encode/decode throughput uses wall-clock for a full write or full scan of that format.\n6. This does NOT compare Strata to SQLite, DuckDB, or a tuned Parquet lakehouse stack.\n7. Parquet+Zstd (when enabled) is a single-file columnar baseline, not a claim that\n   Strata is universally better or worse than Parquet.\n8. Do not treat CSV/JSONL/fixed as optimized storage engines — they are controlled baselines.",
  "parquet_zstd_size_bytes": 1338721,
  "raw_logical_bytes": 4840195,
  "records": 100000,
  "sanity": {
    "csv_records": 100000,
    "expected_records": 100000,
    "fixed_records": 100000,
    "jsonl_records": 100000,
    "strata_records": 100000
  },
  "strata_block_logical_bytes": 5240195,
  "strata_dataset_dir_bytes": 786640,
  "strata_final_compressed_bytes": 783224,
  "strata_semantic_encoded_bytes": 1651216,
  "wall_elapsed_ms": 236
}
```

