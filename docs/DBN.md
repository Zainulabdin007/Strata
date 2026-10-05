# Databento DBN MBO input

Status: the real September 11/14, 2026 corpus is available under `benchmarks/nasdaq/sources/`. Both original vendor transport hashes were verified against the manifest. Native streaming input and a 200,000-record, all-field round trip through Strata, Parquet/Zstd and fixed binary have passed. Larger observations are recorded only after their runs finish. See `benchmarks/nasdaq/results/` and `source-census.json` when present.

The implementation uses Databento's pinned Rust reference decoder (`dbn = 0.69.0`, version upgrade policy `AsIs`) and streaming Zstandard decompression. Sources are opened read-only. Normal ingestion requires no expanded DBN temporary file.

## Inspect, then ingest

```sh
cargo build --workspace --release
target/release/strata dbn /path/to/corpus --out benchmarks/nasdaq-inspection
# Outputs manual.toml, schema.json, sources.json, exact source-N.metadata.dbn headers.
# Default inspection reads at most 100,000 records per file.
# --records 0 explicitly scans every source record to EOF.
target/release/strata analyze /path/to/corpus --profile benchmarks/nasdaq-inspection/manual.toml --format json
target/release/strata ingest /path/to/corpus --profile benchmarks/nasdaq-inspection/manual.toml --output datasets/nasdaq --workers 2
```

The generated manual profile starts with Plain semantic encoding and Zstd level 3. It is deliberately untuned; no synthetic-market assumptions are reused. Analysis and optimizer CLI commands use the normal `RecordParser` adapter and label their prefix sample.

## Canonical mapping

Field order is the DBN MBO binary order. Numeric widths are preserved:

- `length`: u8 (original record length in 32-bit words).
- `rtype`: u8; `publisher_id`: u16; `instrument_id`: u32.
- `ts_event`: u64 nanoseconds; `order_id`: u64.
- `price`: i64, exact original integer in units of 1e-9; no floating-point conversion.
- `size`: u32; `flags`: u8; `channel_id`: u8.
- `action`, `side`: original u8 character bytes, including unknown codes.
- `ts_recv`: u64 nanoseconds; `ts_in_delta`: i32 nanoseconds; `sequence`: u32.
- `ts_out`: u64 only when the actual DBN header declares it present.

The payload is 56 bytes per standard MBO record, or 64 with `ts_out`. Unsigned timestamps preserve `u64::MAX`; signed prices preserve undefined-price sentinels. They are represented as integer columns rather than coercing them to Strata's signed timestamp or floating decimal representations. No field is silently removed. Original metadata (including symbol mappings) is retained separately in exact binary form by the inspection command.

## Observations and validation

`strata dbn` records exact source file lengths, decompressed bytes delivered by the stream, accepted DBN bytes, accepted record count, canonical payload bytes, observed instrument IDs and defined event-time range, undefined timestamp count, DBN version/schema/dataset, ordered record SHA-256, and full original file SHA-256 after EOF.

Partial inspections set `complete=false`. Decoder readahead is included in `decompressed_bytes_observed`; `dbn_bytes_accepted` is the deterministic accepted-record boundary. A partial source hash is never presented as a full-file checksum. The advertised purchase size is never substituted for observed bytes.

Tests cover vendor-encoded raw DBN and DBN.zst, optional `ts_out`, unsigned/signed extremes, sentinels, all fields, exact byte reconstruction, DBN/zstd truncation, oversized metadata, mismatched schema, and DBN → compressed Strata → canonical byte equality. These are **test fixtures**, not the purchased corpus.

Unknown schemas, record types, extension lengths, and unsupported versions fail explicitly. Safety limits are 16 MiB metadata, a 128 MiB Zstd window, 65,536 rows per batch, and one million distinct instrument IDs in the statistics collector. Exceeding a limit fails visibly; it never skips records.

## Reproducible real-data benchmark

`nasdaq-baseline` refuses debug builds. `scripts/run_nasdaq_baseline.py` runs a declared matrix sequentially, retains release binaries/source snapshots/configurations, records macOS peak RSS and CPU, and rejects failed verification or interrupted-clock timings. It refuses to overwrite a run. No full-corpus work is silently skipped.

```sh
cargo build --release --workspace
python3 scripts/run_nasdaq_baseline.py --prepare --tiers --tag census
python3 scripts/run_nasdaq_baseline.py --tiers validation 1gb --tag measured-small
python3 scripts/run_nasdaq_baseline.py --tiers 10gb full --profiles manual --tag measured-large
```

The census scans both complete files and compares their observed SHA-256 hashes to the frozen vendor inventory. It collects four 4,096-record consecutive windows per file, starting at the first defined `ts_event` at or after each metadata time quartile. Window offsets, actual time ranges, and exact DBN sample files are saved. Empty trading periods can move a window past its requested boundary; the observed range is authoritative. These bounded windows do not capture every instrument or market regime. All four optimizer goals test real candidate datasets on this frozen sample; the small sample is never replicated to inflate benchmark work.

The validation tier takes the first 100,000 records of each source. The approximately 1 GB and 10 GB tiers take `floor(target_logical_bytes / number_of_files / canonical_record_bytes)` records **from each file**, in sorted filename order. GB means decimal bytes of the canonical payload. The full tier reads both files to verified EOF. Each target independently receives that identical ordered stream and verifies its output against per-source canonical SHA-256 hashes. Original metadata is retained as exact, shared sidecars; target storage and target-plus-metadata storage are reported separately.

The untuned manual baseline uses Plain + Zstd level 3. Balanced, maximum-compression, maximum-ingest and maximum-decode profiles come from real optimizer experiments and are frozen before larger runs. Full-tier profile selection is explicit (`--full-profiles`, default manual). The Parquet baseline uses Zstd level 3, Arrow/Parquet 53.4.1, and the same logical values. Arrow widens unsigned sub-32-bit fields to UInt32 and signed fields to Int64; no values are dropped or rounded.

Ingest rates include streaming transport decompression, DBN parsing, source hashing, canonical record construction, and target writing. Decode rates include materializing every field and full-stream SHA-256 validation for every format. Strata also verifies a limited all-column query and an unlimited filtered projection against independently accumulated source results. These are buffered, warm-cache, fast-durability runs; physical write throughput measures target bytes handed to the filesystem, not a device-level I/O counter. Whole-process CPU/RSS include verification and query execution.

The original `.dbn.zst` files total exactly **34,317,850,334 bytes**. This is the full vendor transport/storage representation, never a logical-byte denominator and never compared as if it represented only a partial tier. Advertised purchase sizes and synthetic results are not used as real-corpus measurements.

References: [DBN format](https://databento.com/docs/standards-and-conventions/databento-binary-encoding), [reference MBO layout](https://docs.rs/dbn/0.69.0/dbn/record/struct.MboMsg.html).
