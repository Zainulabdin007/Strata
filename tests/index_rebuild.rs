//! Exhaustive correctness tests for sequence / time / categorical / min-max indexes.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use strata_codec::{compressor_for, CompressionCodec};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_storage::{
    rebuild_indexes, BlockRef, DatasetIndexes, DatasetReader, DatasetWriter, DatasetWriterOptions,
    IndexValue,
};

fn schema() -> Schema {
    Schema::new(
        "index_ticks",
        vec![
            Field::required("seq", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required("symbol", LogicalType::categorical_string(Some(8))),
            Field::required(
                "side",
                LogicalType::enum_type(vec!["buy".into(), "sell".into()]),
            ),
            Field::required(
                "price",
                LogicalType::fixed_decimal(18, 4, IntegerWidth::Bits64),
            ),
        ],
    )
    .unwrap()
}

fn make_records(n: u64) -> Vec<Record> {
    let symbols = [
        "AAPL", "MSFT", "GOOG", "AMZN", "META", "NVDA", "TSLA", "AMD",
    ];
    (0..n)
        .map(|i| {
            Record::new()
                .with("seq", FieldValue::UnsignedInteger(i))
                .with(
                    "ts",
                    FieldValue::Timestamp(1_700_000_000_000_000_000 + i as i64 * 1_000_000),
                )
                .with(
                    "symbol",
                    FieldValue::CategoricalString(symbols[(i as usize) % symbols.len()].into()),
                )
                .with(
                    "side",
                    FieldValue::Enum(if i % 2 == 0 {
                        "buy".into()
                    } else {
                        "sell".into()
                    }),
                )
                .with("price", FieldValue::FixedDecimal(10_000 + (i % 500) as i64))
        })
        .collect()
}

fn write_dataset(root: &std::path::Path, records: &[Record], block_rows: u32, segment_bytes: u64) {
    let _ = std::fs::remove_dir_all(root);
    let mut writer = DatasetWriter::create(
        root,
        schema(),
        DatasetWriterOptions {
            block_row_count: block_rows,
            segment_target_size_bytes: segment_bytes,
            compressor: Arc::from(compressor_for(CompressionCodec::None, None)),
            ..Default::default()
        },
    )
    .unwrap();
    writer.write_records(records).unwrap();
    writer.finish().unwrap();
}

/// Ground truth: map each (segment, block) to its records by scanning the dataset.
fn ground_truth_blocks(root: &std::path::Path) -> Vec<(BlockRef, Vec<Record>)> {
    let reader = DatasetReader::open(root).unwrap();
    let mut out = Vec::new();
    for seg_ord in 0..reader.segment_count() {
        let seg = reader.open_segment(seg_ord).unwrap();
        let file = reader.manifest().segments[seg_ord].file.clone();
        for (bi, bm) in seg.meta().blocks.iter().enumerate() {
            let batch = seg.read_block(bi).unwrap();
            out.push((
                BlockRef {
                    segment_ordinal: seg_ord as u32,
                    segment_file: file.clone(),
                    block_id: bm.id.0,
                    row_count: bm.row_count,
                },
                batch.records,
            ));
        }
    }
    out
}

fn block_key(b: &BlockRef) -> (u32, u32) {
    (b.segment_ordinal, b.block_id)
}

#[test]
fn rebuild_produces_all_index_kinds() {
    let root = std::env::temp_dir().join(format!("strata-idx-kinds-{}", std::process::id()));
    write_dataset(&root, &make_records(2_500), 128, 32 * 1024);
    let report = rebuild_indexes(&root).unwrap();
    assert!(report.block_count > 1);
    assert_eq!(report.record_count, 2_500);
    assert!(report.block_minmax_written);
    assert_eq!(report.sequence_fields, vec!["seq".to_string()]);
    assert_eq!(report.time_fields, vec!["ts".to_string()]);
    assert!(report.categorical_fields.contains(&"symbol".to_string()));
    assert!(report.categorical_fields.contains(&"side".to_string()));

    let idx = DatasetIndexes::open(&root).unwrap();
    assert!(idx.block_minmax().is_some());
    assert!(idx.sequence_fields().any(|f| f == "seq"));
    assert!(idx.time_fields().any(|f| f == "ts"));
    assert!(idx.categorical_fields().any(|f| f == "symbol"));
}

#[test]
fn sequence_index_no_false_negatives_and_exact_for_unique_seq() {
    let root = std::env::temp_dir().join(format!("strata-idx-seq-{}", std::process::id()));
    let records = make_records(3_000);
    write_dataset(&root, &records, 100, 24 * 1024);
    rebuild_indexes(&root).unwrap();
    let idx = DatasetIndexes::open(&root).unwrap();
    let blocks = ground_truth_blocks(&root);

    // Build seq → owning block(s).
    let mut seq_to_blocks: HashMap<u64, HashSet<(u32, u32)>> = HashMap::new();
    for (bref, recs) in &blocks {
        for r in recs {
            let seq = match r.get("seq") {
                Some(FieldValue::UnsignedInteger(s)) => *s,
                _ => panic!("missing seq"),
            };
            seq_to_blocks
                .entry(seq)
                .or_default()
                .insert(block_key(bref));
        }
    }

    for seq in 0..3_000u64 {
        let got: HashSet<_> = idx
            .lookup_sequence("seq", seq)
            .unwrap()
            .into_iter()
            .map(|b| block_key(&b))
            .collect();
        let expect = seq_to_blocks.get(&seq).cloned().unwrap_or_default();
        assert!(
            expect.is_subset(&got),
            "seq {seq}: false negative; expect={expect:?} got={got:?}"
        );
        // Unique monotonic seqs → exactly one block.
        assert_eq!(expect.len(), 1);
        assert_eq!(got, expect, "seq {seq} should map to exactly its block");
    }

    // Missing seq outside range → empty or only edge overlaps; 3_000 not present.
    let none = idx.lookup_sequence("seq", 9_999_999).unwrap();
    assert!(none.is_empty());
}

#[test]
fn time_index_range_covers_all_matching_blocks() {
    let root = std::env::temp_dir().join(format!("strata-idx-time-{}", std::process::id()));
    let records = make_records(2_000);
    write_dataset(&root, &records, 64, 16 * 1024);
    rebuild_indexes(&root).unwrap();
    let idx = DatasetIndexes::open(&root).unwrap();
    let blocks = ground_truth_blocks(&root);

    let start = 1_700_000_000_000_000_000 + 500 * 1_000_000;
    let end = 1_700_000_000_000_000_000 + 1_200 * 1_000_000;

    let mut expect: HashSet<(u32, u32)> = HashSet::new();
    for (bref, recs) in &blocks {
        let hit = recs.iter().any(|r| match r.get("ts") {
            Some(FieldValue::Timestamp(t)) => *t >= start && *t <= end,
            _ => false,
        });
        if hit {
            expect.insert(block_key(bref));
        }
    }

    let got: HashSet<_> = idx
        .lookup_time_range("ts", start, end)
        .unwrap()
        .into_iter()
        .map(|b| block_key(&b))
        .collect();
    assert!(
        expect.is_subset(&got),
        "time range false negatives: missing {:?}",
        expect.difference(&got).collect::<Vec<_>>()
    );

    // Point lookup via degenerate range for a known timestamp.
    let point = 1_700_000_000_000_000_000 + 777 * 1_000_000;
    let point_blocks = idx.lookup_time_range("ts", point, point).unwrap();
    assert!(!point_blocks.is_empty());
    let keys: HashSet<_> = point_blocks.iter().map(block_key).collect();
    let owner = blocks
        .iter()
        .find(|(_, recs)| {
            recs.iter()
                .any(|r| r.get("ts") == Some(&FieldValue::Timestamp(point)))
        })
        .map(|(b, _)| block_key(b))
        .unwrap();
    assert!(keys.contains(&owner));
}

#[test]
fn categorical_index_exact_postings_for_every_value() {
    let root = std::env::temp_dir().join(format!("strata-idx-cat-{}", std::process::id()));
    let records = make_records(4_000);
    // Small segments to force multiple segment files.
    write_dataset(&root, &records, 50, 8 * 1024);
    rebuild_indexes(&root).unwrap();
    let idx = DatasetIndexes::open(&root).unwrap();
    let blocks = ground_truth_blocks(&root);
    assert!(
        DatasetReader::open(&root).unwrap().segment_count() > 1,
        "expected multiple segments for categorical spread test"
    );

    let mut expect_symbol: BTreeMap<String, HashSet<(u32, u32)>> = BTreeMap::new();
    let mut expect_side: BTreeMap<String, HashSet<(u32, u32)>> = BTreeMap::new();
    for (bref, recs) in &blocks {
        let mut symbols = BTreeSet::new();
        let mut sides = BTreeSet::new();
        for r in recs {
            if let Some(FieldValue::CategoricalString(s)) = r.get("symbol") {
                symbols.insert(s.clone());
            }
            if let Some(FieldValue::Enum(s)) = r.get("side") {
                sides.insert(s.clone());
            }
        }
        for s in symbols {
            expect_symbol.entry(s).or_default().insert(block_key(bref));
        }
        for s in sides {
            expect_side.entry(s).or_default().insert(block_key(bref));
        }
    }

    for (value, expect) in &expect_symbol {
        let got: HashSet<_> = idx
            .lookup_categorical("symbol", value)
            .unwrap()
            .into_iter()
            .map(|b| block_key(&b))
            .collect();
        assert_eq!(&got, expect, "symbol={value}");
    }
    // Unknown symbol → empty.
    assert!(idx.lookup_categorical("symbol", "ZZZZ").unwrap().is_empty());

    for (value, expect) in &expect_side {
        let got: HashSet<_> = idx
            .lookup_categorical("side", value)
            .unwrap()
            .into_iter()
            .map(|b| block_key(&b))
            .collect();
        assert_eq!(&got, expect, "side={value}");
    }

    // AAPL appears across multiple segments.
    let aapl = idx.lookup_categorical("symbol", "AAPL").unwrap();
    let segs: HashSet<_> = aapl.iter().map(|b| b.segment_ordinal).collect();
    assert!(
        segs.len() > 1,
        "AAPL should span multiple segments, got {segs:?}"
    );
}

#[test]
fn block_minmax_matches_actual_block_scans() {
    let root = std::env::temp_dir().join(format!("strata-idx-minmax-{}", std::process::id()));
    let records = make_records(1_500);
    write_dataset(&root, &records, 75, 20 * 1024);
    rebuild_indexes(&root).unwrap();
    let idx = DatasetIndexes::open(&root).unwrap();
    let stats = idx.block_minmax().unwrap();
    let blocks = ground_truth_blocks(&root);
    assert_eq!(stats.blocks.len(), blocks.len());

    for (stored, (bref, recs)) in stats.blocks.iter().zip(blocks.iter()) {
        assert_eq!(&stored.block, bref);

        // seq
        let seqs: Vec<u64> = recs
            .iter()
            .filter_map(|r| match r.get("seq") {
                Some(FieldValue::UnsignedInteger(s)) => Some(*s),
                _ => None,
            })
            .collect();
        let fm = stored.fields.iter().find(|f| f.field == "seq").unwrap();
        assert_eq!(fm.min, Some(IndexValue::U64(*seqs.iter().min().unwrap())));
        assert_eq!(fm.max, Some(IndexValue::U64(*seqs.iter().max().unwrap())));
        assert_eq!(fm.null_count, 0);

        // ts
        let tss: Vec<i64> = recs
            .iter()
            .filter_map(|r| match r.get("ts") {
                Some(FieldValue::Timestamp(t)) => Some(*t),
                _ => None,
            })
            .collect();
        let fm = stored.fields.iter().find(|f| f.field == "ts").unwrap();
        assert_eq!(fm.min, Some(IndexValue::I64(*tss.iter().min().unwrap())));
        assert_eq!(fm.max, Some(IndexValue::I64(*tss.iter().max().unwrap())));

        // price
        let prices: Vec<i64> = recs
            .iter()
            .filter_map(|r| match r.get("price") {
                Some(FieldValue::FixedDecimal(p)) => Some(*p),
                _ => None,
            })
            .collect();
        let fm = stored.fields.iter().find(|f| f.field == "price").unwrap();
        assert_eq!(fm.min, Some(IndexValue::I64(*prices.iter().min().unwrap())));
        assert_eq!(fm.max, Some(IndexValue::I64(*prices.iter().max().unwrap())));

        // symbol lex min/max
        let syms: Vec<String> = recs
            .iter()
            .filter_map(|r| match r.get("symbol") {
                Some(FieldValue::CategoricalString(s)) => Some(s.clone()),
                _ => None,
            })
            .collect();
        let fm = stored.fields.iter().find(|f| f.field == "symbol").unwrap();
        assert_eq!(
            fm.min,
            Some(IndexValue::String(syms.iter().min().unwrap().clone()))
        );
        assert_eq!(
            fm.max,
            Some(IndexValue::String(syms.iter().max().unwrap().clone()))
        );
    }

    // Prune APIs agree with sequence / time indexes for ranges.
    let pruned = idx.prune_unsigned_range("seq", 100, 250).unwrap();
    let via_seq: HashSet<_> = (100..=250u64)
        .flat_map(|s| idx.lookup_sequence("seq", s).unwrap())
        .map(|b| block_key(&b))
        .collect();
    let via_prune: HashSet<_> = pruned.iter().map(block_key).collect();
    assert_eq!(via_prune, via_seq);
}

#[test]
fn rebuild_is_idempotent_and_replaces_stale_indexes() {
    let root = std::env::temp_dir().join(format!("strata-idx-idem-{}", std::process::id()));
    write_dataset(&root, &make_records(800), 100, 64 * 1024);
    let r1 = rebuild_indexes(&root).unwrap();
    let idx1 = DatasetIndexes::open(&root).unwrap();
    let catalog1 = idx1.catalog().clone();

    let r2 = rebuild_indexes(&root).unwrap();
    assert_eq!(r1.block_count, r2.block_count);
    assert_eq!(r1.record_count, r2.record_count);
    let idx2 = DatasetIndexes::open(&root).unwrap();
    assert_eq!(idx2.catalog().artifacts.len(), catalog1.artifacts.len());
    assert_eq!(
        idx2.lookup_sequence("seq", 42).unwrap(),
        idx1.lookup_sequence("seq", 42).unwrap()
    );
}

#[test]
fn example_lookups_match_prompt_shape() {
    let root = std::env::temp_dir().join(format!("strata-idx-example-{}", std::process::id()));
    write_dataset(&root, &make_records(5_000), 256, 12 * 1024);
    rebuild_indexes(&root).unwrap();
    let idx = DatasetIndexes::open(&root).unwrap();

    // timestamp → segment / block
    let ts = 1_700_000_000_000_000_000 + 2_048 * 1_000_000;
    let hits = idx.lookup_time_range("ts", ts, ts).unwrap();
    assert_eq!(hits.len(), 1);
    let loc = hits[0].display_location();
    assert!(loc.contains("segment"));
    assert!(loc.contains("block"));

    // AAPL → blocks across segments
    let aapl = idx.lookup_categorical("symbol", "AAPL").unwrap();
    assert!(!aapl.is_empty());
    let segs: HashSet<_> = aapl.iter().map(|b| b.segment_ordinal).collect();
    assert!(
        segs.len() > 1,
        "AAPL should appear in multiple segments, got {aapl:?}"
    );
}
