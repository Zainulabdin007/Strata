# Strata

Strata is a Rust-based, compression-first storage engine and CLI for massive structured event datasets.

A user points Strata at a file or directory plus a configuration file describing the schema and compression strategy. Strata reads the source data, converts it into its own compressed on-disk format, writes the result to disk, and reports real metrics to the CLI and the local Strata Lab dashboard.

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
| `strata-input` | Input discovery + CSV/JSONL and native streaming Databento DBN MBO parsers |
| `strata-format` | Versioned on-disk format, segment metadata, block metadata |
| `strata-codec` | Encoding / compression interfaces |
| `strata-storage` | Files/directories, segments, readers/writers |
| `strata-metrics` | Canonical runtime and final metrics (CLI + dashboard) |
| `strata-cli` | Command-line application (`strata`) |

Supporting directories: `docs/`, `examples/`, `tests/`.

## Current status

Lossless columnar storage with composable semantic encodings, LZ4/Zstd compression, bounded parallel ingestion, columnar scans, typed queries, indexes, recovery, compaction, a loopback API, and Strata Lab. Format version 0.2 protects segment metadata and blocks with CRC32; checksums detect corruption, not malicious tampering.

The primary performance corpus is real Databento XNAS.ITCH MBO. Original vendor files remain unchanged. Streaming input preserves all MBO fields and metadata; measured comparisons use identical canonical records for Strata, Parquet/Zstd and fixed binary. See [BENCHMARKS.md](BENCHMARKS.md) for the current measured scope and [docs/DBN.md](docs/DBN.md) for mapping and reproduction.

Compression, ingest and decode have different optima. Choose a measured profile for your objective; sample winners are not universal defaults. This is a local storage engine, not a distributed database.

## Strata Lab

```sh
cargo build --release --workspace
cd strata-lab
npm install
npm run build
cd ..
target/release/strata-api --root . --port 9160
```

Open `http://127.0.0.1:9160`. For frontend development, run `npm run dev` inside `strata-lab`; Vite proxies API calls to port 9160. The API binds to loopback and restricts paths to its configured workspace root.

## Logical types

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
- [FORMAT.md](FORMAT.md) — on-disk segment format (v0.2)
- [DURABILITY.md](DURABILITY.md) — FAST / DURABLE / STRICT guarantees and recovery
- [QUERY.md](QUERY.md) — typed query language (not SQL)
- [COMPACTION.md](COMPACTION.md) — updates, tombstones, and compaction
- [benchmarks/README.md](benchmarks/README.md) — how to run `strata-bench` (measured results only)
- [benchmarks/COMPARISON.md](benchmarks/COMPARISON.md) — fair multi-format comparison methodology
- [BENCHMARKS.md](BENCHMARKS.md) — measured release baseline (no invented numbers)
- [docs/DBN.md](docs/DBN.md) — native Databento input and real-corpus methodology
- [docs/HANDOFF.md](docs/HANDOFF.md) — Prompts 18–24 status + next-agent pickup
- [PRODUCT.md](PRODUCT.md) — product definition and operating principles

## Design principles

1. Correctness first.
2. Optimization second.
3. Measurement always.
4. Do not fabricate benchmark results.
