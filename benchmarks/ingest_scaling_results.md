# Ingest scaling benchmark (real results)

- Records: 100000
- Block rows: 4096
- Compression: lz4
- Host workers available: 18

| workers | elapsed_ms | records/s | blocks/s | encode MB/s | compress MB/s | write MB/s | disk MB/s | util % |
|--------:|-----------:|----------:|---------:|------------:|--------------:|-----------:|----------:|-------:|
| 1 | 108 | 923836 | 231.0 | 35.2 | 209.6 | 100.4 | 1.9 | 94.5 |
| 2 | 57 | 1737971 | 434.5 | 34.4 | 202.5 | 91.9 | 3.5 | 90.9 |
| 4 | 32 | 3114412 | 778.6 | 33.0 | 202.2 | 91.4 | 6.3 | 84.8 |
| 8 | 23 | 4251249 | 1062.8 | 31.6 | 191.0 | 100.6 | 8.6 | 60.6 |
| 12 | 22 | 4362240 | 1090.6 | 31.8 | 193.8 | 99.6 | 8.8 | 41.1 |

Notes:
- Results are measured wall-clock / phase timers — not invented.
- More threads are **not** assumed faster; I/O and encode mix dominate.
- `util %` is encode-worker busy time / (wall × workers).
