# Codex / Cursor handoff — Prompts 18–24

This document is the pickup point after Cursor continued from Codex’s `docs/PHASES.md` gate.

## Gate status (updated)

| Prompt | Status | Notes |
| --- | --- | --- |
| 18 Local API | **passed** (Codex) | `strata-api` + `strata-engine`, SSE, loopback guards |
| 19 Strata Lab | **passed** (Codex) | `strata-lab/` React UI |
| 20 Experimentation | **passed** | Codex UI + engine; Cursor marked complete (real optimizer experiments + save TOML) |
| 21 Folder ingest | **passed (Cursor)** | Globs, resume, continue-on-error, total_files, deterministic walk |
| 22 Hardening | **partial (Cursor)** | New `tests/hardening.rs` + existing codec `proptest`; not a full fuzz corpus |
| 23 Baseline | **partial (Cursor)** | `BENCHMARKS.md` with measured 100k/1m compare + 10m ingest/decode; no 100M / full profile matrix |
| 24 Final audit | **this handoff** | Architecture + bottlenecks below; not a speculative rewrite |

## What Cursor changed (for Codex)

### Prompt 21 — folder-scale ingest
- `InputFilterConfig`: `include_globs`, `exclude_globs`
- `strata-input::glob` + `discover_inputs_with` / `DiscoverOptions`
- Deterministic sorted recursive directory walk
- `MetricsSnapshot.total_files` + `MetricsHandle::set_total_files`
- `strata-engine::IngestOptions`: `resume`, `continue_on_error`, glob overrides
- Progress sidecar `{output}/.ingest-progress.json` (safe resume = clear partial dataset, skip completed files)
- Per-file errors → `ingest-errors.json` when continuing
- CLI: `--resume`, `--continue-on-error`, `--include`, `--exclude`

### Prompt 22 — hardening
- Integration test `tests/hardening.rs` (varint truncate, bitpack noise, absurd dictionary, block probe, segment recover noise, property varint)
- Codec crate already had `limits::check_count/check_bytes` and some `proptest`

### Prompt 23 — baseline
- Release runs written under `benchmarks/baseline/`
- Human summary: root `BENCHMARKS.md` (**measured only**)

## Remaining work (recommended for Codex / next agent)

1. **Prompt 22 complete:** dedicated fuzz targets (cargo-fuzz) for segment/block framing; allocation-bound tests with adversarial lengths; format version / checksum mutation suites; `cargo fmt` + `clippy -D warnings` + full `cargo test --workspace` green (watch for known hang in `parallel_ingest::pipeline_reports_queue_and_worker_metrics`).
2. **Prompt 23 complete:** 100M if machine allows; run plain vs stock vs optimizer goals through Lab/engine experiment on same seed; add query numbers; capture CPU brand + RAM + peak RSS; tag JSON with `"release_build": true` if Lab charts require that key.
3. **Lab wiring:** Live ingest form should expose resume / include / exclude / continue_on_error (API already accepts `IngestOptions` fields via serde default).
4. **Encode timing fairness:** compare-suite Strata encode rec/s can look unrealistically high on large runs — split “submit” vs “finish/drain” or report pipeline wall separately.
5. **CLI cleanup:** unused imports / dead `map_codec` warnings in `strata-cli`.
6. **Git metadata:** bench meta `git_commit` was null in this environment — ensure `.git` available when publishing results.

## Architecture snapshot (audit)

```
strata-cli / strata-api / strata-lab
        │
        ▼
 strata-engine  (orchestration only — ingest/analyze/optimize/query/inspect)
        │
        ├── strata-input   (discover + CSV/JSONL stream parse)
        ├── strata-storage (pipeline, segments, indexes, recover, compact)
        ├── strata-codec   (semantic pipelines + lz4/zstd)
        ├── strata-analyze / strata-optimize
        ├── strata-query
        └── strata-metrics (canonical counters; Lab/CLI/API must not invent rates)
```

### Storage format
Versioned segments with block directory, column pipelines, optional general compression, footers/CRC; dataset `manifest.json`; optional indexes; keyed updates/tombstones + compaction.

### Encoding / planner
Composable `EncodingPlan` steps; profile hints + optimizer candidates; Lab experiment writes real candidate datasets and equality-checks decode.

### Ingest
Bounded parallel parse→encode→write queues; multi-file discovery; progress/metrics per file; schema enforced by parsers (CSV header must include all columns).

### Query
Typed non-SQL language; index skip when present; projected decode; API caps responses at 1000 rows.

### Durability
FAST / DURABLE / STRICT modes on writer; recover/verify tools exist. Lab/API are local-only (Host/Origin + `X-Strata-Local`).

## Largest measured bottlenecks (from this baseline)

1. **Full decode / materialize records** (~1.1–1.3M rec/s) lags Parquet Arrow decode and naive fixed scans — likely allocation + row reconstruction.
2. **Semantic encode size** still much larger than final LZ4 payload — planner/encoding choice dominates before general compression.
3. **Compare encode timing** for Strata is hard to interpret vs sequential baselines (pipeline overlap).
4. **JSONL** remains a large on-disk baseline (~3× logical); expected for text.

## Code areas most deserving profiling (not rewritten here)

1. Block decode → `Record` materialization path in `strata-storage` / codec pipeline
2. Dictionary / bitpack hot loops under realistic cardinality
3. Ingest pipeline queue wait vs worker utilization (`active_workers`, queue depths)
4. Index build + query skip effectiveness on market symbol/time predicates
5. Mmap vs buffered sealed-segment reads at 10M+

## Validation Cursor ran

- `cargo test -p strata-input --lib` (globs/discovery)
- `cargo test -p strata-integration --test hardening`
- `cargo check -p strata-engine -p strata-cli -p strata-api`
- Release `strata-bench` 100k/1m compare + 10m ingest/compression/decode

Full workspace `clippy -D warnings` / full test suite / 100M **not** claimed green in this handoff.
