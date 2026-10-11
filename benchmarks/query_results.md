# Strata query benchmarks

Records: 50000

| case | ms | rows | blocks_scanned | blocks_skipped | rows/s |
| --- | ---: | ---: | ---: | ---: | ---: |
| full_scan | 32 | 50000 | 98 | 0 | 1519988 |
| indexed_scan_all | 32 | 50000 | 98 | 0 | 1539158 |
| projection | 16 | 50000 | 98 | 0 | 2978968 |
| range_seq | 7 | 5001 | 11 | 87 | 691825 |
| symbol_time | 7 | 626 | 11 | 87 | 83066 |
| symbol_time_full_scan | 18 | 626 | 98 | 0 | 33391 |
