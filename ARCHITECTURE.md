# Strata Architecture

## Purpose

Strata turns structured input (file or directory) plus a `CompressionProfile` into versioned, compressed segments on disk, while emitting canonical metrics for the CLI and a future Lab dashboard.

## Pipeline

```text
┌─────────────┐   ┌──────────────────┐   ┌──────────────┐
│ Input path  │──▶│ strata-input     │──▶│ RecordBatch  │
│ + Profile   │   │ discover+parse   │   │ (strata-core)│
└─────────────┘   └──────────────────┘   └──────┬───────┘
                                                 │
                                                 ▼
                                          ┌──────────────┐
                                          │ Columnar     │
                                          │ blocks       │
                                          └──────┬───────┘
                                                 │
                         ┌───────────────────────┼───────────────────────┐
                         ▼                       ▼                       ▼
                ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
                │ Semantic encode │    │ Optional general│    │ Segment write   │
                │ (strata-codec)  │───▶│ compression     │───▶│ (strata-storage)│
                └─────────────────┘    └─────────────────┘    └────────┬────────┘
                                                                       │
                                                                       ▼
                                                              ┌─────────────────┐
                                                              │ Metrics         │
                                                              │ (strata-metrics)│
                                                              └────────┬────────┘
                                                                       │
                                                  ┌────────────────────┴────────────────────┐
                                                  ▼                                         ▼
                                           ┌──────────┐                            ┌────────────────┐
                                           │ strata   │                            │ Lab dashboard  │
                                           │ CLI      │                            │ API (later)    │
                                           └──────────┘                            └────────────────┘
```

## Crate responsibilities

### `strata-core`

Domain model + profile loading:

- Logical type system
- Schema / field definitions
- `Record` / `RecordBatch`
- TOML `CompressionProfile` (input format, schema/columns, semantic hints, block/segment/compression settings)
- Shared validation errors

No compression algorithms. Profile loading is the only disk I/O.

### `strata-input`

Ingest front-end:

- Recursive file discovery with extension filters
- `RecordParser` trait (CSV + JSONL today; Parquet/binary reserved)
- Text/JSON coercion into `FieldValue`s

### `strata-format`

Logical description of the on-disk format:

- Format version (`0.1` draft)
- `SegmentHeader` / `SegmentFooter` / `SegmentMeta`
- `BlockMeta` / `ColumnBlockMeta` / `BlockEncoding`

Binary layout details live in `FORMAT.md`. This crate owns metadata types, not file bytes.

### `strata-storage`

Filesystem layout and segment I/O:

- `DatasetWriter` / `DatasetReader`
- `SegmentWriter` / `SegmentReader`
- `BlockWriter` / `BlockReader`
- Columnar PLAIN blocks, little-endian, CRC32-checked

### `strata-codec`

- `ColumnEncoder` / `ColumnDecoder`
- `PlainEncoder` (null bitmap + typed dense values)
- `Compressor` trait (`IdentityCompressor` only for now)

### `strata-metrics`

**Single source of truth** for numbers shown to users:

- `MetricsHandle` / `MetricsCollector` — runtime accumulation (shared with storage)
- `MetricsSnapshot` — live or final view (serde-friendly for dashboard)
- `MetricsReport` — versioned envelope
- Derived rates: records/sec, logical MB/s, disk MB/s, compression ratio
- Phase timers: parse, encode, compression (0 when disabled), write

CLI and dashboard must consume this crate rather than inventing parallel counters.
The storage engine emits through `MetricsHandle` and never formats output itself.

### `strata-query`

Typed query engine (not SQL):

- Strata Query Language parser (`QUERY.md`)
- Predicate pushdown + index block skipping
- Projected column decode + streaming `RecordBatch` results

### `strata-cli`

Thin command layer:

- `strata version`
- `strata ingest` / `analyze` / `optimize` / `inspect` / `verify` / `recover`
- `strata index rebuild`
- `strata query`

### `strata-bench`

Serious benchmark binary (not a library; **not** run by `cargo test`):

- Deterministic generators: market / telemetry / logs
- Suites: ingest, compression, decode, query (p50/p95/p99), recovery, compaction, compare
- Fair baselines in `compare`: JSONL, CSV, fixed-width binary, optional Parquet+Zstd
- Persists measured JSON + `RESULTS.md` under `benchmarks/` with git/rustc/OS metadata
- See [benchmarks/README.md](benchmarks/README.md) and [benchmarks/COMPARISON.md](benchmarks/COMPARISON.md)

## Dependency direction

```text
strata-cli
  ├── strata-core
  ├── strata-input
  ├── strata-format
  ├── strata-codec
  ├── strata-storage
  ├── strata-metrics
  ├── strata-analyze
  ├── strata-optimize
  └── strata-query

strata-bench   → strata-core, strata-codec, strata-storage, strata-metrics, strata-query
strata-query   → strata-core, strata-storage
strata-input   → strata-core
strata-storage → strata-core, strata-format, strata-codec
strata-format  → strata-core
strata-codec   → strata-core
strata-metrics → (standalone; serde only)
strata-core    → (standalone + toml)
```

Lower crates must not depend on `strata-cli` or `strata-metrics` unless a later design explicitly requires engine-level metric hooks via traits.

## What this phase does *not* include

- SQL / joins / aggregations
- Parquet / binary input parsers
- Dashboard server

Those belong to subsequent implementation phases.
