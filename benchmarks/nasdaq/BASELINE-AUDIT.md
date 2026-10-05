# Nasdaq baseline audit — 2026-09-17

Status at start: reference DBN reader, full source census, 32,768 time-window optimizer samples, and 200,000-record validation runs exist. No full-corpus target benchmark exists. Expanded DBN materialization is in progress.

## Exact source census

- xnas-itch-20260911.mbo.dbn.zst: 11,391,551,884 vendor Zstd bytes; 33,561,108,766 DBN bytes; 599,292,291 records; 33,560,368,296 canonical bytes.
- xnas-itch-20260914.mbo.dbn.zst: 22,926,298,450 vendor Zstd bytes; 66,688,499,808 DBN bytes; 1,190,852,830 records; 66,687,758,480 canonical bytes.

Total: 100,249,608,574 DBN bytes; 1,790,145,121 records; 100,248,126,776 logical bytes; 13,231 distinct instrument IDs.

## Schema

DBN metadata version 1, XNAS.ITCH, MBO, ts_out=false, decoder dbn 0.69.0 with AsIs upgrade policy. Every 56-byte MBO record is mapped in wire order:

- u8: length, rtype (first two header fields).
- u16: publisher_id.
- u32: instrument_id.
- u64: ts_event, order_id.
- i64: price (integer units of 1e-9; preserve sentinels).
- u32: size.
- u8: flags, channel_id, action, side (raw codes, not lossy enum remapping).
- u64: ts_recv.
- i32: ts_in_delta.
- u32: sequence.

All fields are required and widths are preserved in logical accounting. Full original DBN metadata, including symbol mappings, is retained byte-for-byte as shared sidecars. The canonical schema is profiles/schema.json. Reference: https://databento.com/docs/knowledge-base/new-users/dbn-encoding

## Prior validation only — not full-corpus results

- fixed: 200,000 records, 11,200,196 bytes, ingest 808,519 records/s, validation-inclusive decode 870,615 records/s. All-field SHA-256 passed.
- strata: 200,000 records, 3,192,905 bytes, ingest 760,214 records/s, validation-inclusive decode 695,760 records/s. All-field SHA-256 passed.
- parquet: 200,000 records, 4,243,922 bytes, ingest 452,504 records/s, validation-inclusive decode 832,171 records/s. All-field SHA-256 passed.

## Bottlenecks and methodology

- Existing decoded full scans construct an IndexMap and allocate field names for every row. Queries construct rows before filtering. These costs are present in source inspection; their individual contribution has not yet been isolated on this corpus.
- Existing input, ingest and validation timing includes canonical row construction and hashing. Standalone columnar scan measurements must be labeled separately and implemented for both Strata and Parquet.
- Both CRC layers and field range/domain validation remain enabled during baseline establishment.
- Existing mmap is an anonymous copied snapshot; changing it to file-backed mapping requires an explicit immutable-file lifetime contract to avoid unsafe concurrent truncation.
- Synthetic profile winners are not adopted. Existing real-data optimizer profiles are compared again at deterministic tiers.
- Profile selection must consider size, ingest and decode jointly and repeat candidates; no claim of a globally optimal profile.
- Full benchmarks must verify the complete ordered canonical hashes and observed sizes against the census, not extrapolate sample results.
- Repo has no .git directory; save source archives and binary hashes for reproducibility.
