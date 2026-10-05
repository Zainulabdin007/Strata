# Strata

Strata is a Rust-based, compression-first storage engine and CLI for massive structured event datasets.

A user points Strata at a file or directory plus a configuration file describing the schema and compression strategy. Strata reads the source data, converts it into its own compressed on-disk format, writes the result to disk, and reports real metrics to both the CLI and a future Strata Lab dashboard.

## Core flow

```text
Input file/folder + CompressionProfile
  → Strata parser
  → Record batches
  → Columnar blocks
  → Semantic encodings
  → Optional general compression
  → Strata segments on disk
  → Canonical metrics subsystem
  → CLI + local dashboard API
```

## Workspace layout

| Crate | Responsibility |
| --- | --- |
| `strata-core` | Schemas, logical types, records/batches, TOML compression profiles, errors |
| `strata-input` | Input discovery + CSV/JSONL parsers (Parquet/binary reserved) |
| `strata-format` | Versioned on-disk format, segment metadata, block metadata |
| `strata-codec` | Encoding / compression interfaces |
| `strata-storage` | Files/directories, segments, readers/writers |
| `strata-metrics` | Canonical runtime and final metrics (CLI + dashboard) |
| `strata-cli` | Command-line application (`strata`) |

Supporting directories: `docs/`, `examples/`, `tests/`.

## Current status

**Uncompressed storage phase.** Profiles load from TOML, inputs are discovered and parsed (CSV/JSONL), and datasets are written/read with columnar PLAIN segments (lossless round-trip).

Semantic encodings and general compression are **not** implemented yet.

## Logical types (initial)

- `Boolean`
- `UnsignedInteger`
- `SignedInteger`
- `Timestamp`
- `FixedDecimal`
- `CategoricalString`
- `Enum`
- `Binary`

## Build and test

```bash
cargo fmt
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## CLI

```bash
cargo run -p strata-cli -- version
cargo run -p strata-cli -- ingest examples/data/market.csv --profile examples/profiles/stock-market.toml --output /tmp/strata-demo
cargo run -p strata-cli -- ingest examples/data/ --profile examples/profiles/telemetry.toml --output /tmp/strata-telemetry
cargo run -p strata-cli -- ingest examples/data/market.csv --profile examples/profiles/stock-market.toml --output /tmp/strata-strict --durability strict
cargo run -p strata-cli -- inspect --dataset /tmp/strata-demo
cargo run -p strata-cli -- verify /tmp/strata-demo
cargo run -p strata-cli -- recover /tmp/strata-demo
cargo run -p strata-cli -- index rebuild /tmp/strata-demo
cargo run -p strata-cli -- compact /tmp/strata-keyed
cargo run -p strata-cli -- query --dataset /tmp/strata-demo --query 'select ts, price where symbol == AAPL limit 10' --explain

# Serious benchmarks (not run by cargo test) — see benchmarks/README.md
cargo run -p strata-bench --release -- --suite ingest --size 100k --workload market
```

Example profiles live under `examples/profiles/`.

## Documentation

- [ARCHITECTURE.md](ARCHITECTURE.md) — crate boundaries and data flow
- [FORMAT.md](FORMAT.md) — on-disk segment format (v0.1 draft)
- [DURABILITY.md](DURABILITY.md) — FAST / DURABLE / STRICT guarantees and recovery
- [QUERY.md](QUERY.md) — typed query language (not SQL)
- [COMPACTION.md](COMPACTION.md) — updates, tombstones, and compaction
- [benchmarks/README.md](benchmarks/README.md) — how to run `strata-bench` (measured results only)
- [benchmarks/COMPARISON.md](benchmarks/COMPARISON.md) — fair multi-format comparison methodology
- [BENCHMARKS.md](BENCHMARKS.md) — measured release baseline (no invented numbers)
- [docs/HANDOFF.md](docs/HANDOFF.md) — Prompts 18–24 status + next-agent pickup
- [PRODUCT.md](PRODUCT.md) — product definition and operating principles

## Design principles

1. Correctness first.
2. Optimization second.
3. Measurement always.
4. Do not fabricate benchmark results.
