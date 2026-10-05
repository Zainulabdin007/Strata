# Strata Durability Guarantees

Strata supports three explicit durability modes. Choose one based on the
trade-off between ingest throughput and crash survival.

Modes may be set in a compression profile:

```toml
[durability]
mode = "durable"          # fast | durable | strict
sync_every_blocks = 16    # durable only
```

Or overridden on the CLI: `strata ingest … --durability strict`.

## Commit model

1. Each **block** is a self-describing frame (`version | codec | lengths | payload | CRC32`).
2. An incomplete block write never modifies prior block bytes.
3. Segment **commit** is the footer magic `SFTR` written **after** the block
   directory and after header counts are patched.
4. `strata recover` can salvage valid block prefixes from an active segment that
   crashed before footer commit by scanning frames and truncating the incomplete tail.

## FAST

**Goal:** maximize ingest speed.

**Acknowledgment:** records are acknowledged when buffered to the OS page cache
(after `write`), not when durable on disk.

**Crash loss conditions (expected):**

- Process crash or power loss may lose the entire active (unfinalized) segment,
  including blocks that were already `write()`n but not `fsync`d.
- Finalized segments from earlier rotations may still be present on disk, but
  without a flushed manifest they are recovered only via `strata recover`.

**Not a bug:** losing recent FAST acknowledgments after a crash.

## DURABLE

**Goal:** bound loss to a configured sync window.

**Policy:** `flush` after each block; `fsync` (`sync_all`) every
`sync_every_blocks` blocks (default 16) and on every segment finish / manifest publish.

**Acknowledgment:** advances only at sync points.

**Guarantee:** after acknowledgment of `N` records, a process crash may lose at
most the trailing unsynced window of the active segment (fewer than
`sync_every_blocks` blocks). Synced prefixes and finalized segments survive and
are recoverable.

## STRICT

**Goal:** acknowledged completed data survives process crash.

**Policy:** `flush` + `fsync` after every block, on segment finish, and when
publishing `manifest.json` (temp file + rename + directory fsync when available).

**Acknowledgment:** advances only after a successful `fsync` of the containing
block (or segment finalize).

**Guarantee:** if Strata has acknowledged `N` records under STRICT, then after a
process crash those `N` records must be present after `strata recover` (when
needed) and `strata verify`. Incomplete in-flight frames at the file tail are
discarded; they never corrupt earlier committed blocks.

## Tools

```bash
strata verify <dataset>
strata recover <dataset>
```

- **verify** — validate segment headers, block CRCs, footers, and manifest consistency.
- **recover** — truncate incomplete tails, rebuild segment commit metadata when
  possible, and rebuild `manifest.json` from recoverable segments.
