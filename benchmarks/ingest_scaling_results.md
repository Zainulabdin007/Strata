# Ingest scaling benchmark (real results)

- Records: 100000
- Block rows: 4096
- Compression: lz4
- Host workers available: 18

| workers | elapsed_ms | records/s | blocks/s | encode MB/s | compress MB/s | write MB/s | disk MB/s | util % |
|--------:|-----------:|----------:|---------:|------------:|--------------:|-----------:|----------:|-------:|
| 1 | 197 | 506735 | 126.7 | 19.0 | 139.2 | 61.7 | 1.0 | 95.3 |
| 2 | 103 | 962172 | 240.5 | 18.6 | 139.9 | 69.5 | 1.9 | 92.3 |
| 4 | 56 | 1783804 | 446.0 | 18.3 | 139.9 | 72.2 | 3.6 | 86.7 |
| 8 | 31 | 3223982 | 806.0 | 20.4 | 150.2 | 71.2 | 6.5 | 70.4 |
| 12 | 32 | 3081810 | 770.5 | 20.0 | 149.9 | 73.8 | 6.2 | 45.8 |

Notes:
- Results are measured wall-clock / phase timers — not invented.
- More threads are **not** assumed faster; I/O and encode mix dominate.
- `util %` is encode-worker busy time / (wall × workers).
