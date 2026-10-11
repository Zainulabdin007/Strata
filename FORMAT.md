# Strata On-Disk Format (v0.2)

Status: implemented for composable semantic pipelines and optional LZ4/Zstd compression. Little-endian throughout. The legacy PLAIN layout below remains supported; current writers use the framed pipeline layout described at the end of this document.

## Goals

- Versioned, append-friendly segments
- Columnar blocks (row groups)
- Exact lossless round-trip of logical values
- Structured errors on corruption (no panics)

## Dataset layout

```text
<dataset>/
  manifest.json
  segments/
    000000.strata
    000001.strata
    ...
  indexes/                    # optional; rebuilt via `strata index rebuild`
    manifest.json
    block_minmax.json
    sequence/<field>.json
    time/<field>.json
    categorical/<field>.json
```

Indexes map values to **blocks** (segment ordinal + block id), not individual rows.
They are derived data and always rebuildable from segments.

## Sealed segments and mmap

A segment with a valid `SFTR` footer is **sealed**: immutable historical data.
Readers may memory-map sealed segments (`SegmentReadMode::Mmap` / `Auto`) and
borrow block frames as `&[u8]` views into an immutable anonymous snapshot. Opening
a segment copies the file into private pages and makes them read-only; this is
not file-backed zero-copy mmap. External truncation cannot invalidate slices. Active writers and recovery paths continue to use buffered I/O.

## Segment file (`.strata`)

```text
┌──────────────────────────┐
│ Segment Header           │
├──────────────────────────┤
│ Block 0 .. Block N-1     │  (columnar PLAIN payloads)
├──────────────────────────┤
│ Block Directory          │
├──────────────────────────┤
│ Segment Footer           │
└──────────────────────────┘
```

### Header (LE)

| Field | Type | Notes |
| --- | --- | --- |
| magic | `[u8; 8]` | `STRATA01` |
| version_major | `u16` | |
| version_minor | `u16` | |
| flags | `u32` | reserved `0` |
| schema_len | `u32` | |
| schema_json | `[u8; schema_len]` | full `Schema` as JSON |
| segment_id_len | `u32` | |
| segment_id | UTF-8 | |
| record_count | `u64` | patched on finish |
| block_count | `u32` | patched on finish |
| reserved | `u32` | `0` |

### Block

| Field | Type | Notes |
| --- | --- | --- |
| row_count | `u32` | |
| column_count | `u32` | |
| column directory | … | name, encoding=`1` (PLAIN), `data_rel_offset`, `data_len` |
| column payloads | bytes | in schema order |
| crc32 | `u32` | IEEE CRC32 of all preceding block-frame bytes |

Offsets in the column directory are relative to the **start of the block**, enabling safe skip/read of individual columns.

### Block directory (after all blocks)

Per block: `block_id`, `file_offset`, `block_length`, `row_count`, `column_count`, then per-column name/encoding/offsets/logical_bytes.

### Footer (32 bytes)

| Field | Type |
| --- | --- |
| block_directory_offset | `u64` |
| logical_bytes | `u64` |
| encoded_bytes | `u64` |
| blocks_crc32 | `u32` | v0.2: CRC32 of the entire preceding file prefix, including header, directory and the first 24 footer bytes |
| footer_magic | `[u8; 4]` | `SFTR` |

## PLAIN column encoding

```text
bitmap_len: u32
null_bitmap: [u8; bitmap_len]   // bit i set => row i non-null
values: dense typed payloads for non-null rows only (LE)
```

Typed payloads:

| Logical type | On disk |
| --- | --- |
| Boolean | `u8` 0/1 |
| UnsignedInteger | width bytes LE |
| SignedInteger | width bytes LE |
| Timestamp | `i64` LE |
| FixedDecimal | scaled integer, width bytes LE |
| CategoricalString / Enum | `u32` len + UTF-8 |
| Binary | `u32` len + bytes |

## Compatibility

- Major bump: incompatible
- Minor bump: additive; readers accept same major with `reader.minor >= file.minor`

## Current framed pipeline blocks

The block frame (version 1) contains `version: u8`, `codec: u8`,
`semantic_body_length: u32`, `compressed_length: u32`, the compressed body,
and an IEEE CRC32 of the preceding frame bytes. Codec tags are 0 (identity),
1 (LZ4), and 2 (Zstd). Lengths and CRC are checked before decoding.

The expanded semantic body starts with row and column counts, followed by a
column directory and contiguous payloads. Directory offsets are relative to
this expanded body, not the compressed frame. Pipeline columns use encoding
tag 6; legacy PLAIN uses tag 1. Gaps, overlaps, trailing bytes and schema/name
mismatches are rejected.

Each pipeline payload contains column-format version 1, a length-prefixed null
bitmap, the serialized encoding plan, ordered step metadata, and a
length-prefixed encoded body. Decoding reverses the plan, verifies types,
integer widths, enum domains, null counts and declared lengths, then restores
nulls. Compression and null handling never discard represented fields.

Format 0.2 segment CRC covers the entire file prefix through the first 24 footer
bytes, including patched header counts, schema and block directory. Format 0.1
used a block-only segment CRC and remains a legacy read path. CRC is corruption
detection, not cryptographic authentication. Benchmark identity and lossless
record checks use SHA-256 separately.

The authoritative encoder/decoder pairs are
`crates/strata-storage/src/segment/{writer,reader}.rs`,
`crates/strata-storage/src/block/{writer,reader}.rs`, and
`crates/strata-codec/src/pipeline.rs`.
