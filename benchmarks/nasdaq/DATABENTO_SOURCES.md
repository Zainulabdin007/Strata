# Databento source files

The Nasdaq benchmark inputs are two vendor Databento files, XNAS ITCH MBO, already compressed as `.dbn.zst`:

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `xnas-itch-20260911.mbo.dbn.zst` | 11,391,551,884 | `2029814878f2b5d90d2230fbb7b8ff9741c6a5ea925ba5f7f323043c0ffb4a29` |
| `xnas-itch-20260914.mbo.dbn.zst` | 22,926,298,450 | `52c19cfa036c6106015a4dfa88f036a08ec528610fa9e7ae5d1e9c4d8ca534a3` |

Together that is 34,317,850,334 bytes. These files are the original vendor transport. They are not Strata datasets, and a single slice is not a valid `.dbn.zst` by itself.

`benchmarks/nasdaq/sources/` is gitignored. The bytes are stored as a private split under `Zainulabdin007`, because GitHub rejects files over 100 MB and a single repository is too small for 32 GB.

## Where the bytes are

37 private repositories:

- https://github.com/Zainulabdin007/strata-xnas-mbo-001
- through https://github.com/Zainulabdin007/strata-xnas-mbo-037

Each repository holds raw cuts of 94,371,840 bytes (90 MiB), plus a short `manifest.json`. Repositories 001–036 have 10 chunks. Repository 037 has the last 4 chunks. There are 364 chunks in total. GitHub warned that 90 MB is above its 50 MB recommendation and accepted the pushes. The files are ordinary git blobs, not Git LFS.

September 11 is 121 chunks, `xnas-itch-20260911.mbo.dbn.zst.0000` through `.0120`. The last chunk is 66,931,084 bytes. Those chunks fill repositories 001–012 completely. Repository 013 holds `.0120` and then the start of September 14.

September 14 is 243 chunks, `xnas-itch-20260914.mbo.dbn.zst.0000` through `.0242`. The last chunk is 88,313,170 bytes. It starts in repository 013 and ends in repository 037.

Each split repository's `manifest.json` lists the chunk names, source file, index, and byte size in that repository. The two full-file sizes and hashes above are the check after reassembly.

## Rebuild

Cloning needs access to those private repositories. Run this from the Strata repository root. It clones into a temporary directory, concatenates each source in index order, checks the SHA-256, and moves the two files into `benchmarks/nasdaq/sources/`.

```bash
#!/usr/bin/env bash
set -euo pipefail

DEST="benchmarks/nasdaq/sources"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$DEST" "$WORK/parts"

for i in $(seq 1 37); do
  n=$(printf '%03d' "$i")
  git clone --depth 1 "https://github.com/Zainulabdin007/strata-xnas-mbo-$n.git" "$WORK/repo-$n"
  find "$WORK/repo-$n" -name '*.dbn.zst.*' -exec mv {} "$WORK/parts/" \;
done

assemble() {
  local name="$1" expect="$2"
  find "$WORK/parts" -name "${name}.*" | sort | xargs cat > "$WORK/$name"
  local got
  got="$(shasum -a 256 "$WORK/$name" | awk '{print $1}')"
  if [[ "$got" != "$expect" ]]; then
    echo "hash mismatch for $name" >&2
    echo "got    $got" >&2
    echo "expect $expect" >&2
    exit 1
  fi
  mv "$WORK/$name" "$DEST/$name"
}

assemble xnas-itch-20260911.mbo.dbn.zst \
  2029814878f2b5d90d2230fbb7b8ff9741c6a5ea925ba5f7f323043c0ffb4a29
assemble xnas-itch-20260914.mbo.dbn.zst \
  52c19cfa036c6106015a4dfa88f036a08ec528610fa9e7ae5d1e9c4d8ca534a3
```

Chunk names sort in byte order: `0000` before `0001`, and the September 11 prefix sorts apart from September 14, so each `assemble` call joins one source only.

After both hashes match, the benchmark inputs are:

- `benchmarks/nasdaq/sources/xnas-itch-20260911.mbo.dbn.zst`
- `benchmarks/nasdaq/sources/xnas-itch-20260914.mbo.dbn.zst`
