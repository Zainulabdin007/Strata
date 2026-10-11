# Docs index

- [../ARCHITECTURE.md](../ARCHITECTURE.md) — crate boundaries and pipeline
- [../FORMAT.md](../FORMAT.md) — on-disk format v0.2
- [../PRODUCT.md](../PRODUCT.md) — product definition
- [../README.md](../README.md) — quick start

## Profiles

TOML compression profiles live in `examples/profiles/`:

- `stock-market.toml` — CSV equity-style events
- `telemetry.toml` — JSONL telemetry events

Sample inputs: `examples/data/`.

- [DBN.md](DBN.md) — lossless Databento DBN input and reproducible Nasdaq tiers
- [../benchmarks/nasdaq/DATABENTO_SOURCES.md](../benchmarks/nasdaq/DATABENTO_SOURCES.md) — private split of the vendor `.dbn.zst` files and how to rebuild them
- [../BENCHMARKS.md](../BENCHMARKS.md) — primary real-data results
- [PHASES.md](PHASES.md) — completion gates
- [../tests/README.md](../tests/README.md) — correctness and product checks
