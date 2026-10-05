# Strata benchmarks

This directory stores **measured** benchmark artifacts.

Heavy suites live in `crates/strata-bench` and are **not** executed by
`cargo test --workspace`.

## Run

```bash
# Small smoke (100k market ingest)
cargo run -p strata-bench --release -- \
  --suite ingest --size 100k --workload market --seed 42

# Multiple suites
cargo run -p strata-bench --release -- \
  --suite ingest,compression,decode,query --size 1m --workload market

# Full suite list including recovery + compaction
cargo run -p strata-bench --release -- \
  --suite all --size 100k --workload market --out benchmarks/strata-bench

# Large runs (10m / 100m) — expect long wall time and disk use
cargo run -p strata-bench --release -- --suite ingest --size 10m --workload telemetry
cargo run -p strata-bench --release -- --suite all --size 100m --workload logs --workers 8

# Fair format comparison (JSONL / CSV / fixed / Parquet+Zstd / Strata)
cargo run -p strata-bench --release -- \
  --suite compare --size 100k --workload market --seed 42
```

## Outputs

Each successful run writes:

- `benchmarks/strata-bench/<suite>-<workload>-<size>-<unix>.json`
- appends a section to `benchmarks/strata-bench/RESULTS.md`

JSON includes environment metadata (git commit, rustc, OS/arch, seed, profile,
workers, block/segment sizes) plus **only metrics that were measured**.

Do not invent numbers in docs. Refresh results by re-running `strata-bench`.

Methodology for multi-format comparisons: [COMPARISON.md](COMPARISON.md).

## Suites

| Suite | Measures |
| --- | --- |
| `ingest` | records/sec, logical GB/sec, physical MB/sec |
| `compression` | raw / encoded / compressed bytes, bits/value, ratio |
| `decode` | records/sec, logical GB/sec |
| `query` | p50/p95/p99 latency, scan throughput, range queries |
| `recovery` | recover + verify after truncated segment |
| `compaction` | keyed update/delete fold; logical preservation |
| `compare` | fair sizes + encode/decode vs JSONL/CSV/fixed/Parquet+Zstd |

## Workloads

Deterministic generators (`--seed`):

- `market` — timestamp, sequence, order_id, symbol, price, quantity, side, event_type
- `telemetry` — device metrics
- `logs` — structured service logs

## Sizes

`100k` · `1m` · `10m` · `100m`
