# Strata query benchmarks

Records: 50000

| case | ms | rows | blocks_scanned | blocks_skipped | rows/s |
| --- | ---: | ---: | ---: | ---: | ---: |
| full_scan | 55 | 50000 | 98 | 0 | 908426 |
| indexed_scan_all | 54 | 50000 | 98 | 0 | 919389 |
| projection | 28 | 50000 | 98 | 0 | 1747020 |
| range_seq | 11 | 5001 | 11 | 87 | 442529 |
| symbol_time | 11 | 626 | 11 | 87 | 52305 |
| symbol_time_full_scan | 32 | 626 | 98 | 0 | 19539 |
