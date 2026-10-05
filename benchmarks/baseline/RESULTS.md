# Strata bench results

Measured runs only. Re-run `strata-bench` to refresh.

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
      "decode_elapsed_ms": 41,
      "decode_records": 100000,
      "decode_records_per_sec": 2386423.5979683804,
      "encode_elapsed_ms": 54,
      "encode_records_per_sec": 1844685.1475710303,
      "format": "jsonl",
      "notes": "NDJSON; text; field names per row",
      "path": "/tmp/strata-baseline-100k/compare-market-100k-42/baseline.jsonl",
      "size_bytes": 14918373
    },
    {
      "decode_elapsed_ms": 10,
      "decode_records": 100000,
      "decode_records_per_sec": 9823102.621069003,
      "encode_elapsed_ms": 31,
      "encode_records_per_sec": 3146562.9039301802,
      "format": "csv",
      "notes": "flat tabular CSV with header",
      "path": "/tmp/strata-baseline-100k/compare-market-100k-42/baseline.csv",
      "size_bytes": 5918439
    },
    {
      "decode_elapsed_ms": 0,
      "decode_records": 100000,
      "decode_records_per_sec": 182107898.9301161,
      "encode_elapsed_ms": 12,
      "encode_records_per_sec": 8285061.115167569,
      "format": "fixed_binary",
      "notes": "naive little-endian fixed slots; not a production format",
      "path": "/tmp/strata-baseline-100k/compare-market-100k-42/baseline.fixed",
      "size_bytes": 5500114
    },
    {
      "decode_elapsed_ms": 4,
      "decode_records": 100000,
      "decode_records_per_sec": 20677891.59119064,
      "encode_elapsed_ms": 48,
      "encode_records_per_sec": 2048678.2757479574,
      "format": "parquet_zstd",
      "notes": "Apache Parquet + ZSTD level 3 via arrow/parquet; single file",
      "path": "/tmp/strata-baseline-100k/compare-market-100k-42/baseline.parquet",
      "size_bytes": 1338721
    },
    {
      "decode_elapsed_ms": 86,
      "decode_records": 100000,
      "decode_records_per_sec": 1155388.1526498827,
      "encode_elapsed_ms": 20,
      "encode_records_per_sec": 4919242.517327908,
      "format": "strata",
      "notes": "Strata dataset dir; codec=lz4; semantic_encoded and compressed reported separately",
      "path": "/tmp/strata-baseline-100k/compare-market-100k-42/strata",
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
  "wall_elapsed_ms": 242
}
```

## compare / market / 1m

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
- records: 1000000

```json
{
  "csv_size_bytes": 60182174,
  "disclaimer": "Do not claim Strata is universally better than Parquet, SQLite, DuckDB, or another system based on this suite alone.",
  "fixed_binary_size_bytes": 55000114,
  "formats": [
    {
      "decode_elapsed_ms": 409,
      "decode_records": 1000000,
      "decode_records_per_sec": 2440880.8468222125,
      "encode_elapsed_ms": 511,
      "encode_records_per_sec": 1954089.9655008349,
      "format": "jsonl",
      "notes": "NDJSON; text; field names per row",
      "path": "/tmp/strata-baseline-1m/compare-market-1m-42/baseline.jsonl",
      "size_bytes": 150182108
    },
    {
      "decode_elapsed_ms": 96,
      "decode_records": 1000000,
      "decode_records_per_sec": 10376247.077037139,
      "encode_elapsed_ms": 318,
      "encode_records_per_sec": 3135207.3730245405,
      "format": "csv",
      "notes": "flat tabular CSV with header",
      "path": "/tmp/strata-baseline-1m/compare-market-1m-42/baseline.csv",
      "size_bytes": 60182174
    },
    {
      "decode_elapsed_ms": 4,
      "decode_records": 1000000,
      "decode_records_per_sec": 227233994.20553315,
      "encode_elapsed_ms": 123,
      "encode_records_per_sec": 8126552.064783378,
      "format": "fixed_binary",
      "notes": "naive little-endian fixed slots; not a production format",
      "path": "/tmp/strata-baseline-1m/compare-market-1m-42/baseline.fixed",
      "size_bytes": 55000114
    },
    {
      "decode_elapsed_ms": 39,
      "decode_records": 1000000,
      "decode_records_per_sec": 25599781.766980395,
      "encode_elapsed_ms": 415,
      "encode_records_per_sec": 2408347.08244634,
      "format": "parquet_zstd",
      "notes": "Apache Parquet + ZSTD level 3 via arrow/parquet; single file",
      "path": "/tmp/strata-baseline-1m/compare-market-1m-42/baseline.parquet",
      "size_bytes": 9124987
    },
    {
      "decode_elapsed_ms": 851,
      "decode_records": 1000000,
      "decode_records_per_sec": 1174616.7086487599,
      "encode_elapsed_ms": 19,
      "encode_records_per_sec": 51619118.83376718,
      "format": "strata",
      "notes": "Strata dataset dir; codec=lz4; semantic_encoded and compressed reported separately",
      "path": "/tmp/strata-baseline-1m/compare-market-1m-42/strata",
      "size_bytes": 7760483
    }
  ],
  "json_size_bytes": 150182108,
  "methodology": "Fair comparison methodology (strata-bench compare):\n1. One deterministic generator (workload + seed + size) produces the logical records.\n2. The same chunks are written, in lockstep, to each baseline and to Strata.\n3. Sizes are on-disk bytes after a full encode/close (Strata = dataset directory bytes\n   and separately semantic_encoded / compressed block totals from segment metadata).\n4. Raw logical size is the sum of decoded field payloads (schema widths / UTF-8 lengths)\n   with no keys, separators, dictionaries, headers, or compression.\n5. Encode/decode throughput uses wall-clock for a full write or full scan of that format.\n6. This does NOT compare Strata to SQLite, DuckDB, or a tuned Parquet lakehouse stack.\n7. Parquet+Zstd (when enabled) is a single-file columnar baseline, not a claim that\n   Strata is universally better or worse than Parquet.\n8. Do not treat CSV/JSONL/fixed as optimized storage engines — they are controlled baselines.",
  "parquet_zstd_size_bytes": 9124987,
  "raw_logical_bytes": 48400297,
  "records": 1000000,
  "sanity": {
    "csv_records": 1000000,
    "expected_records": 1000000,
    "fixed_records": 1000000,
    "jsonl_records": 1000000,
    "strata_records": 1000000
  },
  "strata_block_logical_bytes": 52400297,
  "strata_dataset_dir_bytes": 7760483,
  "strata_final_compressed_bytes": 7753508,
  "strata_semantic_encoded_bytes": 16505488,
  "wall_elapsed_ms": 1915
}
```

## ingest / market / 10m

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
- records: 10000000

```json
{
  "elapsed_ms": 5330,
  "logical_bytes": 524006517,
  "logical_gb_per_sec": 0.09155747471351765,
  "metrics_records_per_sec": 1876172.6078799248,
  "physical_bytes": 77818429,
  "physical_mb_per_sec": 13.923215114714088,
  "records": 10000000,
  "records_per_sec": 1876104.3366895055
}
```

## compression / market / 10m

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
- records: 10000000

```json
{
  "bits_per_value": 7.7776653,
  "column_blocks_observed": 1224,
  "compressed_bytes": 77776653,
  "compression_ratio_logical_over_compressed": 6.7373240784737805,
  "encoded_semantic_bytes": 165093024,
  "raw_logical_bytes": 524006517,
  "records": 10000000,
  "schema_columns": 8,
  "segments": 1
}
```

## decode / market / 10m

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
- records: 10000000

```json
{
  "elapsed_ms": 7628,
  "logical_bytes": 524006517,
  "logical_gb_per_sec": 0.0639742169371103,
  "records": 10000000,
  "records_per_sec": 1310895.7647376838,
  "segment_read_mode": "auto"
}
```

## compare / market / 1m

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
- records: 1000000

```json
{
  "csv_size_bytes": 60182174,
  "disclaimer": "Do not claim Strata is universally better than Parquet, SQLite, DuckDB, or another system based on this suite alone.",
  "fixed_binary_size_bytes": 55000114,
  "formats": [
    {
      "decode_elapsed_ms": 410,
      "decode_records": 1000000,
      "decode_records_per_sec": 2434474.096896169,
      "encode_elapsed_ms": 501,
      "encode_records_per_sec": 1993512.1148223109,
      "format": "jsonl",
      "notes": "NDJSON; text; field names per row",
      "path": "/var/folders/pz/ncdtccx12f7cblfxr9gtwvz80000gn/T/strata-bench-97057/compare-market-1m-42/baseline.jsonl",
      "size_bytes": 150182108
    },
    {
      "decode_elapsed_ms": 96,
      "decode_records": 1000000,
      "decode_records_per_sec": 10388531.061707875,
      "encode_elapsed_ms": 314,
      "encode_records_per_sec": 3182047.3167063026,
      "format": "csv",
      "notes": "flat tabular CSV with header",
      "path": "/var/folders/pz/ncdtccx12f7cblfxr9gtwvz80000gn/T/strata-bench-97057/compare-market-1m-42/baseline.csv",
      "size_bytes": 60182174
    },
    {
      "decode_elapsed_ms": 5,
      "decode_records": 1000000,
      "decode_records_per_sec": 185990282.00776508,
      "encode_elapsed_ms": 118,
      "encode_records_per_sec": 8405339.000728415,
      "format": "fixed_binary",
      "notes": "naive little-endian fixed slots; not a production format",
      "path": "/var/folders/pz/ncdtccx12f7cblfxr9gtwvz80000gn/T/strata-bench-97057/compare-market-1m-42/baseline.fixed",
      "size_bytes": 55000114
    },
    {
      "decode_elapsed_ms": 37,
      "decode_records": 1000000,
      "decode_records_per_sec": 26969625.459326435,
      "encode_elapsed_ms": 422,
      "encode_records_per_sec": 2365607.14363585,
      "format": "parquet_zstd",
      "notes": "Apache Parquet + ZSTD level 3 via arrow/parquet; single file",
      "path": "/var/folders/pz/ncdtccx12f7cblfxr9gtwvz80000gn/T/strata-bench-97057/compare-market-1m-42/baseline.parquet",
      "size_bytes": 9124987
    },
    {
      "decode_elapsed_ms": 850,
      "decode_records": 1000000,
      "decode_records_per_sec": 1176458.9966540034,
      "encode_elapsed_ms": 15,
      "encode_records_per_sec": 64178158.568185285,
      "format": "strata",
      "notes": "Strata dataset dir; codec=lz4; semantic_encoded and compressed reported separately",
      "path": "/var/folders/pz/ncdtccx12f7cblfxr9gtwvz80000gn/T/strata-bench-97057/compare-market-1m-42/strata",
      "size_bytes": 7760483
    }
  ],
  "json_size_bytes": 150182108,
  "methodology": "Fair comparison methodology (strata-bench compare):\n1. One deterministic generator (workload + seed + size) produces the logical records.\n2. The same chunks are written, in lockstep, to each baseline and to Strata.\n3. Sizes are on-disk bytes after a full encode/close (Strata = dataset directory bytes\n   and separately semantic_encoded / compressed block totals from segment metadata).\n4. Raw logical size is the sum of decoded field payloads (schema widths / UTF-8 lengths)\n   with no keys, separators, dictionaries, headers, or compression.\n5. Encode/decode throughput uses wall-clock for a full write or full scan of that format.\n6. This does NOT compare Strata to SQLite, DuckDB, or a tuned Parquet lakehouse stack.\n7. Parquet+Zstd (when enabled) is a single-file columnar baseline, not a claim that\n   Strata is universally better or worse than Parquet.\n8. Do not treat CSV/JSONL/fixed as optimized storage engines — they are controlled baselines.",
  "parquet_zstd_size_bytes": 9124987,
  "raw_logical_bytes": 48400297,
  "records": 1000000,
  "sanity": {
    "csv_records": 1000000,
    "expected_records": 1000000,
    "fixed_records": 1000000,
    "jsonl_records": 1000000,
    "strata_records": 1000000
  },
  "strata_block_logical_bytes": 52400297,
  "strata_dataset_dir_bytes": 7760483,
  "strata_final_compressed_bytes": 7753508,
  "strata_semantic_encoded_bytes": 16505488,
  "wall_elapsed_ms": 1879
}
```

