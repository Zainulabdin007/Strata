# Strata measured baseline (Prompt 23)

**Only numbers from actual release-mode runs on this machine are listed.**
Raw JSON lives under `benchmarks/baseline/`. Re-run to refresh; do not invent values.

## Hardware / software

| Item | Value |
| --- | --- |
| OS | macOS 26.6.2 (Build 25G83) |
| Arch | aarch64 (arm64) |
| CPU | Apple M5 Pro |
| RAM | 64 GiB |
| rustc | 1.98.1 (48a229cea 2026-09-01) |
| Cargo profile | `release` (`debug_assertions=false`) |
| Workers | 18 (host parallelism clamp) |
| Block rows | 65_536 |
| Segment target | 256 MiB |
| Dataset | deterministic `market` generator, seed `42` |
| Strata codec | LZ4 (default bench codec) |

## Methodology

- Generator: `strata-bench` market workload (timestamp, sequence, order_id, symbol, price, quantity, side, event_type).
- Compare suite writes the **same** logical chunks to JSONL, CSV, fixed binary, Parquet+Zstd, and Strata.
- Definitions match `benchmarks/COMPARISON.md`.
- 100M was **not** run in this pass (resource budget).
- Automatic optimizer profiles (balanced / max-compression / max-ingest) and plain-vs-manual stock-profile matrix were **not** fully re-run here; Lab experiment path exists but these tables are compare + ingest/decode only.
- **Limitation:** Strata encode wall time in the compare suite includes pipeline `finish()` drain; extremely high encode rec/s on large runs can reflect overlapping worker time vs sequential baselines — treat encode throughput as directional, sizes as authoritative.

## 100K market — format compare (`--suite compare`)

Artifact: `benchmarks/baseline/compare-market-100k-1789482255.json`

| Metric | Value |
| --- | --- |
| Raw logical bytes | 4_840_195 |
| JSONL size | 14_918_373 |
| CSV size | 5_918_439 |
| Fixed binary size | 5_500_114 |
| Parquet+Zstd size | 1_338_721 |
| Strata semantic encoded | 1_651_216 |
| Strata final compressed (blocks) | 783_224 |
| Strata dataset dir bytes | 786_640 |

| Format | Encode rec/s (measured) | Decode rec/s (measured) |
| --- | --- | --- |
| JSONL | ~1.84e6 | ~2.39e6 |
| CSV | ~3.15e6 | ~9.82e6 |
| Fixed binary | ~8.29e6 | (sub-ms wall; noisy) |
| Parquet+Zstd | ~2.05e6 | ~2.07e7 |
| Strata (lz4) | ~4.92e6 | ~1.16e6 |

## 1M market — format compare

Artifact: `benchmarks/baseline/compare-market-1m-1789482259.json`

| Metric | Value |
| --- | --- |
| Raw logical bytes | 48_400_297 |
| JSONL size | 150_182_108 |
| CSV size | 60_182_174 |
| Fixed binary size | 55_000_114 |
| Parquet+Zstd size | 9_124_987 |
| Strata semantic encoded | 16_505_488 |
| Strata final compressed (blocks) | 7_753_508 |
| Strata dataset dir bytes | 7_760_483 |

| Format | Encode rec/s | Decode rec/s |
| --- | --- | --- |
| JSONL | ~1.95e6 | ~2.44e6 |
| CSV | ~3.14e6 | ~1.04e7 |
| Fixed binary | ~8.13e6 | (noisy) |
| Parquet+Zstd | ~2.41e6 | ~2.56e7 |
| Strata (lz4) | (pipeline-skewed) | ~1.17e6 |

## 10M market — Strata ingest / compression / decode

Artifacts:

- `benchmarks/baseline/ingest-market-10m-1789482264.json`
- `benchmarks/baseline/compression-market-10m-1789482270.json`
- `benchmarks/baseline/decode-market-10m-1789482283.json`

| Metric | Value |
| --- | --- |
| Records | 10_000_000 |
| Ingest elapsed | 5330 ms |
| Ingest records/sec | ~1.876e6 |
| Logical GB/sec | ~0.092 |
| Physical disk MB/sec | ~13.9 |
| Logical bytes | 524_006_517 |
| Semantic encoded | 165_093_024 |
| Compressed bytes | 77_776_653 |
| Ratio (logical/compressed) | ~6.74× |
| Bits/value | ~7.78 |
| Decode elapsed | 7628 ms |
| Decode records/sec | ~1.311e6 |

## Not measured in this document

- 100M records
- Full automatic profile matrix (plain / stock / balanced / max-compression / max-ingest)
- Query latency matrix at baseline sizes
- Peak RSS / RAM
- Cross-machine reproducibility

## Disclaimer

These results do **not** claim Strata is universally better than Parquet, SQLite, DuckDB, or another system. Parquet here is a single-file ZSTD level-3 write via Rust `arrow`/`parquet`, not a lakehouse stack.
