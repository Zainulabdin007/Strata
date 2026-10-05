# Updates, Tombstones, and Compaction

Strata never rewrites sealed compressed blocks in place. Mutations are
**append-only**.

## Keyed vs append-only

Declare keys in the compression profile:

```toml
[schema]
id = "positions"
key = ["id"]          # empty / omitted ⇒ append-only (no compaction)
```

Keyed datasets:

- gain a reserved boolean column `__deleted`
- support `update` (append live version) and `delete` (append tombstone)
- may be compacted

Append-only datasets keep pure append semantics and **must not** be compacted.

## Logical model

| Operation | Physical effect |
|-----------|-----------------|
| insert / update | append record with `__deleted = false` |
| delete | append tombstone (`__deleted = true`, key fields set) |

Latest-wins resolution scans physical order; later rows override earlier ones.
Tombstones remove a key from the logical state.

APIs:

- `DatasetAppender::update` / `delete`
- `DatasetReader::read_latest_records`
- `resolve_latest(key_fields, records)`

## Latest-version index

`strata index rebuild` writes `indexes/latest_version.json` for keyed datasets:
each live key → `(segment_ordinal, block_id, row_in_block)`.

## Compaction

```bash
strata compact <dataset>
```

Compaction:

1. Resolves the latest live row per key
2. Writes new sealed segments (re-encoded / re-compressed)
3. Swaps directories crash-safely (`segments` → `segments.old` → new `segments`)
4. Publishes a new manifest
5. Rebuilds indexes (including latest-version)

Obsolete versions and tombstoned keys are dropped when safe. Logical state before
and after compaction must match.
