# Strata verification

Run `cargo test --workspace` and `cargo test --workspace --release` from the
repository root. Integration tests cover codec round trips, malformed input,
checksums, every-byte segment corruption/truncation, recovery, indexed queries,
compaction, bounded parallel ingest/decode, metrics, mmap snapshot lifetime,
DBN MBO fields/sentinels, and profile/input validation.

`fuzz_safety.rs` contains reproducible mutation and randomized pipeline tests;
`hardening.rs` includes proptest cases. These are bounded regression tests, not
an unbounded coverage-guided cargo-fuzz campaign.

Product checks after building the binaries and Lab:

```sh
cargo build --workspace
python3 scripts/test_api.py
python3 scripts/test_folder.py
cd strata-lab
npm run build
node tests/e2e.mjs
```

Large data benchmarks are explicitly separate from tests. Run the sequential
release Nasdaq harness described in `docs/DBN.md`; every completed report checks
the decoded canonical stream. Do not run builds or tests alongside timed cases.
