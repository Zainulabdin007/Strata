# Strata mmap vs buffered benchmarks

Records: 80000

Keep SegmentReadMode::Auto. Measured winners this run: sequential=mmap, selective=mmap.

| case | mode | ms | rows | rows/s |
| --- | --- | ---: | ---: | ---: |
| buffered_sequential | buffered | 52 | 80000 | 1521929 |
| mmap_sequential | mmap | 52 | 80000 | 1526118 |
| buffered_selective | buffered | 13 | 876 | 63845 |
| mmap_selective | mmap | 13 | 876 | 65520 |
