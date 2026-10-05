# Prompts 18–24 gate record

## 18 — Local API: passed
Implemented loopback-only `strata-api`, shared `strata-engine` orchestration, canonical metrics snapshots and physical-size ratios, dataset/segment/block/column/plan/history/job endpoints, actual queries, SSE. CLI uses the same ingest function. Inputs confined to configured root; mutations require local header; foreign Host/Origin rejected. Jobs bounded to one engine job at a time and 128 in-memory entries.

Validation: `python3 scripts/test_api.py` passed (real compressed ingest, persisted metrics equality, CLI/API counter parity, real query, analysis, optimizer, SSE, origin checks). Metrics, parallel ingest, storage roundtrip, optimization integration tests passed (17 tests).

Issues fixed: ratio used compressed payload instead of physical bytes; final elapsed time kept advancing; segment reader synthesized codec/size metadata; optimizer replicated small samples; pipeline outstanding results were unbounded. Parser hardening changes begun during inspection remain under review at gate 22.

## 19 — Strata Lab: passed
React/TypeScript/Vite local UI: overview, live ingest, column analysis, optimizer pipeline reasons, segments, benchmark charts, query console. Missing measurements are absent, samples labeled. Actual active workers instrumented. Production static build served by loopback API.
Validation: `npm run build`; `node tests/e2e.mjs` passed real browser ingest → inspect → profile → pipelines → segments → query, honest empty benchmark charts, mobile overflow check. No browser runtime errors.

## 20 — Experimentation: passed
Lab Experiment view + `strata-optimize::experiment` + API `experiment` options + save winner profile TOML. Candidates write/close/decode/equality-check real sample datasets. No duplicate compression logic in the frontend.

## 21 — Folder ingestion: passed
Deterministic recursive discovery, extension/glob filters, symlink-cycle exclusion, streaming parsing, exact schema checks, per-file errors/progress, canonical totals. Each file commits sealed segments through a durable checkpoint and manifest. Resume verifies source fingerprints and committed segments, retries only uncommitted files, refuses external dataset changes, and holds an OS writer lock. The earlier destructive resume implementation was replaced.
Validation: `python3 scripts/test_folder.py` passed; API and browser regression passed. This is file-boundary resume, not byte-offset resume.

## 22 — Hardening: passed
Property tests, deterministic mutation fuzzing, every-byte segment corruption/truncation coverage, bounds on varints/packing/dictionaries/decompression/records, authenticated-by-CRC format 0.2 metadata (CRC is not cryptographic), safe immutable mapped snapshots, index integrity/fallback, bounded pipeline and sampled analysis, exact JSON integer presentation. Recovery retains dataset metadata; durable publication propagates directory fsync failures. Release test scratch-directory collision fixed. Benchmark timers and decode work corrected.
Validation: full debug and release workspace suites; `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`; release workspace build; real API/folder/browser checks. Counts and final reruns recorded in FINAL_REPORT.md.

## 23 — Release baseline: in progress
Fresh market-v2 seed-42 workload, independent of chunk boundaries. Real measured optimizer selects the three automatic profiles. Full per-row Strata equality verification and actual compressed-dataset queries. Earlier Cursor numbers are historical and are not the final verified baseline.

## 24 — Final audit: pending gate 23
