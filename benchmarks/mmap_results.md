# Strata mmap vs buffered benchmarks

Records: 80000

Keep SegmentReadMode::Auto. Measured winners this run: sequential=mmap, selective=mmap.

| case | mode | ms | rows | rows/s |
| --- | --- | ---: | ---: | ---: |
| buffered_sequential | buffered | 31 | 80000 | 2578718 |
| mmap_sequential | mmap | 30 | 80000 | 2591359 |
| buffered_selective | buffered | 9 | 876 | 96886 |
| mmap_selective | mmap | 8 | 876 | 100163 |
