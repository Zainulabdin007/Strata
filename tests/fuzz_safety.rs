//! Deterministic property and mutation fuzz regressions; no nightly fuzz runtime required.
use std::{collections::HashMap, panic::catch_unwind};
use strata_codec::{
    codecs::{decode_step, encode_step},
    decode_column, encode_column,
    stream::Stream,
    EncodingPlan, EncodingStep, StepMetadata,
};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema};
use strata_storage::{
    BlockReader, DatasetReader, DatasetWriter, DatasetWriterOptions, SegmentPath, SegmentReader,
};
fn next(seed: &mut u64) -> u64 {
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    *seed
}
#[test]
fn randomized_lossless_integer_pipelines() {
    let ty = LogicalType::signed_integer(IntegerWidth::Bits64);
    let mut seed = 0x23a934dd91;
    for n in 0..160 {
        let mut vals: Vec<_> = (0..n)
            .map(|_| FieldValue::SignedInteger(next(&mut seed) as i64))
            .collect();
        if n > 3 {
            vals[0] = FieldValue::SignedInteger(i64::MIN);
            vals[1] = FieldValue::SignedInteger(i64::MAX);
            vals[2] = FieldValue::Null;
        }
        for steps in [
            vec![EncodingStep::Plain],
            vec![
                EncodingStep::Delta,
                EncodingStep::ZigZag,
                EncodingStep::Varint,
            ],
            vec![
                EncodingStep::DeltaOfDelta,
                EncodingStep::ZigZag,
                EncodingStep::BitPacking,
            ],
            vec![EncodingStep::FrameOfReference, EncodingStep::BitPacking],
        ] {
            let plan = EncodingPlan::new(steps).unwrap();
            let encoded = encode_column(&ty, &vals, &plan).unwrap();
            assert_eq!(decode_column(&ty, n as u32, &encoded.bytes).unwrap(), vals);
        }
    }
}
#[test]
fn malformed_counts_widths_dictionaries_and_varints_are_errors() {
    let bad = |step, bytes, meta| {
        assert!(
            catch_unwind(|| decode_step(step, Stream::Encoded(bytes), meta))
                .unwrap()
                .is_err()
        )
    };
    bad(EncodingStep::Varint, vec![255, 255, 255, 255], None);
    bad(EncodingStep::Plain, vec![1, 255, 255, 255, 255], None);
    bad(
        EncodingStep::Varint,
        [vec![1, 0, 0, 0], vec![255; 9], vec![2]].concat(),
        None,
    );
    bad(
        EncodingStep::RunLengthEncoding,
        vec![1, 0, 0, 0, 0, 255, 255, 255, 255, 15],
        None,
    );
    let meta = StepMetadata {
        step: EncodingStep::BitPacking,
        blob: vec![65, 1, 0, 0, 0],
    };
    bad(EncodingStep::BitPacking, vec![0; 9], Some(&meta));
    let meta = StepMetadata {
        step: EncodingStep::DictionaryEncoding,
        blob: vec![255; 4],
    };
    assert!(decode_step(
        EncodingStep::DictionaryEncoding,
        Stream::U64(vec![0]),
        Some(&meta)
    )
    .is_err());
    let encoded = encode_step(
        EncodingStep::DictionaryEncoding,
        Stream::Bytes(vec![b"one".to_vec()]),
    )
    .unwrap();
    assert!(decode_step(
        EncodingStep::DictionaryEncoding,
        Stream::U64(vec![1]),
        encoded.meta.as_ref()
    )
    .is_err());
    for codec in [
        strata_codec::CompressionCodec::Lz4,
        strata_codec::CompressionCodec::Zstd,
    ] {
        assert!(strata_codec::compressor_for(codec, None)
            .decompress(&[], usize::MAX)
            .is_err());
    }
}
#[test]
fn column_mutation_fuzz_never_panics() {
    let ty = LogicalType::unsigned_integer(IntegerWidth::Bits64);
    let vals = (0..64).map(FieldValue::UnsignedInteger).collect::<Vec<_>>();
    let plan = EncodingPlan::new(vec![
        EncodingStep::Delta,
        EncodingStep::ZigZag,
        EncodingStep::BitPacking,
    ])
    .unwrap();
    let good = encode_column(&ty, &vals, &plan).unwrap().bytes;
    for len in 0..good.len() {
        assert!(catch_unwind(|| decode_column(&ty, 64, &good[..len]))
            .unwrap()
            .is_err());
    }
    let mut seed = 71939;
    for _ in 0..4000 {
        let mut bytes = good.clone();
        let index = next(&mut seed) as usize % bytes.len();
        bytes[index] ^= (next(&mut seed) as u8) | 1;
        let _ = catch_unwind(|| decode_column(&ty, 64, &bytes)).expect("column parser panic");
    }
}
#[test]
fn segment_block_corruption_and_truncation() {
    let root = std::env::temp_dir().join(format!("strata-fuzz-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let schema = Schema::new(
        "fuzz",
        vec![Field::required(
            "id",
            LogicalType::unsigned_integer(IntegerWidth::Bits64),
        )],
    )
    .unwrap();
    let mut writer = DatasetWriter::create(
        &root,
        schema.clone(),
        DatasetWriterOptions {
            block_row_count: 8,
            encoding_plans: HashMap::new(),
            ..Default::default()
        },
    )
    .unwrap();
    let rows = (0..24)
        .map(|i| Record::new().with("id", FieldValue::UnsignedInteger(i)))
        .collect::<Vec<_>>();
    writer.write_records(&rows).unwrap();
    writer.finish().unwrap();
    let reader = DatasetReader::open(&root).unwrap();
    let seg = reader.open_segment(0).unwrap();
    let bytes = seg.as_bytes().to_vec();
    let block = seg.block_bytes(0).unwrap().to_vec();
    drop(seg);
    let path = root.join("mutation.strata");
    for len in 0..bytes.len() {
        std::fs::write(&path, &bytes[..len]).unwrap();
        assert!(
            catch_unwind(|| SegmentReader::open_buffered(SegmentPath::new(&path)))
                .unwrap()
                .is_err(),
            "accepted truncation {len}"
        );
    }
    for index in 0..bytes.len() {
        let mut damaged = bytes.clone();
        damaged[index] ^= 1;
        std::fs::write(&path, &damaged).unwrap();
        assert!(
            catch_unwind(|| SegmentReader::open_buffered(SegmentPath::new(&path)))
                .unwrap()
                .is_err(),
            "unprotected segment byte {index}"
        );
    }
    for len in 0..block.len() {
        assert!(
            catch_unwind(|| BlockReader::new().read_batch(&schema, &block[..len]))
                .unwrap()
                .is_err()
        );
    }
    let mut seed = 8829;
    for _ in 0..1000 {
        let mut damaged = block.clone();
        let n = damaged.len();
        let index = next(&mut seed) as usize % (n - 4);
        damaged[index] ^= next(&mut seed) as u8 | 1;
        let crc = strata_format::crc32(&damaged[..n - 4]);
        damaged[n - 4..].copy_from_slice(&crc.to_le_bytes());
        let result = catch_unwind(|| BlockReader::new().read_batch(&schema, &damaged));
        assert!(result.is_ok(), "block parser panic");
    }
    std::fs::remove_dir_all(root).unwrap();
}
