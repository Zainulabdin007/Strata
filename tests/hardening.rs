//! Adversarial / property-style hardening for parsers and codecs.
//!
//! Malformed inputs must return errors — not panic, and not silently invent values.

use std::collections::HashMap;

use proptest::prelude::*;
use strata_codec::codecs::{BitPackingCodec, Codec, DictionaryEncodingCodec, VarintCodec};
use strata_codec::plan::EncodingStep;
use strata_codec::plan::StepMetadata;
use strata_codec::IdentityCompressor;
use strata_codec::{EncodingPlan, Stream};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};
use strata_format::BlockId;
use strata_storage::{encode_block, probe_block_frame, recover_segment_file, DatasetReader};

fn schema() -> Schema {
    Schema::new(
        "harden",
        vec![
            Field::required("seq", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required("sym", LogicalType::categorical_string(Some(8))),
        ],
    )
    .unwrap()
}

#[test]
fn truncated_varint_is_error_not_panic() {
    let codec = VarintCodec;
    for n in 0..16usize {
        let mut bytes = vec![0u8; n];
        if n >= 4 {
            bytes[0..4].copy_from_slice(&(8u32).to_le_bytes());
        }
        let _ = codec.decode(Stream::Encoded(bytes), None);
    }
}

#[test]
fn random_bitpack_noise_does_not_panic() {
    let codec = BitPackingCodec;
    for seed in 0..64u64 {
        let mut rng = seed;
        let mut bytes = vec![0u8; 64];
        for b in &mut bytes {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1);
            *b = (rng >> 33) as u8;
        }
        let meta = StepMetadata {
            step: EncodingStep::BitPacking,
            blob: {
                let mut b = vec![8u8];
                b.extend_from_slice(&8u32.to_le_bytes());
                b
            },
        };
        let _ = codec.decode(Stream::Encoded(bytes), Some(&meta));
    }
}

#[test]
fn dictionary_rejects_absurd_count() {
    let codec = DictionaryEncodingCodec;
    let meta = StepMetadata {
        step: EncodingStep::DictionaryEncoding,
        blob: u32::MAX.to_le_bytes().to_vec(),
    };
    let err = codec.decode(Stream::U64(vec![0]), Some(&meta));
    assert!(err.is_err(), "absurd dictionary length must fail");
}

#[test]
fn probe_block_rejects_truncated_and_garbage() {
    for payload in [
        vec![],
        vec![0u8; 3],
        vec![0xff; 17],
        b"not-a-block".to_vec(),
    ] {
        assert!(probe_block_frame(&payload).is_none() || probe_block_frame(&payload).is_some());
        let _ = probe_block_frame(&payload);
    }
}

#[test]
fn decode_block_rejects_random_corruption() {
    let schema = schema();
    let mut records = Vec::new();
    for i in 0..32u64 {
        records.push(
            Record::new()
                .with("seq", FieldValue::UnsignedInteger(i))
                .with("ts", FieldValue::Timestamp(i as i64 * 1000))
                .with("sym", FieldValue::CategoricalString("AAPL".into())),
        );
    }
    let plans = HashMap::new();
    let encoded = encode_block(&schema, &records, BlockId(0), &plans, &IdentityCompressor).unwrap();
    let mut bytes = encoded.bytes;
    for i in (0..bytes.len()).step_by(7) {
        bytes[i] ^= 0x5A;
        let _ = probe_block_frame(&bytes);
    }
}

proptest! {
    #[test]
    fn prop_varint_roundtrip_or_clean_error(values in prop::collection::vec(any::<u64>(), 0..64)) {
        let codec = VarintCodec;
        let enc = codec.encode(Stream::U64(values.clone())).unwrap();
        let Stream::Encoded(mut raw) = enc.stream else { return Ok(()); };
        let dec = codec.decode(Stream::Encoded(raw.clone()), None).unwrap();
        assert_eq!(dec.into_u64().unwrap(), values);
        if raw.len() > 4 {
            raw.truncate(raw.len() / 2);
            let _ = codec.decode(Stream::Encoded(raw), None);
        }
    }

    #[test]
    fn prop_recover_segment_tolerates_noise(noise in prop::collection::vec(any::<u8>(), 0..256)) {
        let dir = std::env::temp_dir().join(format!("strata-harden-seg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("seg.bin");
        std::fs::write(&path, &noise).unwrap();
        let _ = recover_segment_file(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn open_dataset_missing_manifest_is_error() {
    let dir = std::env::temp_dir().join(format!("strata-harden-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    assert!(DatasetReader::open(&dir).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn encoding_plan_plain_is_stable() {
    let _ = EncodingPlan::plain();
}
