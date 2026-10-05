//! Round-trip and property tests for composable semantic encodings.

use proptest::prelude::*;
use strata_codec::{
    decode_column, encode_column, plan_examples, plan_for_field, BitPackingCodec, Codec,
    DeltaCodec, DeltaOfDeltaCodec, DictionaryEncodingCodec, EncodingPlan, EncodingStep,
    FrameOfReferenceCodec, PlainCodec, RunLengthEncodingCodec, Stream, VarintCodec, ZigZagCodec,
};
use strata_core::{FieldEncodingHint, FieldValue, IntegerWidth, LogicalType, TimestampUnit};

fn assert_roundtrip(ty: &LogicalType, plan: &EncodingPlan, values: &[FieldValue]) {
    let enc = encode_column(ty, values, plan).expect("encode");
    assert_eq!(enc.encoded_size(), enc.bytes.len());
    assert!(!enc.metadata.plan.steps.is_empty());
    let dec = decode_column(ty, values.len() as u32, &enc.bytes).expect("decode");
    assert_eq!(dec, values);
}

#[test]
fn example_timestamp_pipeline() {
    let ty = LogicalType::timestamp(TimestampUnit::Nanoseconds);
    let plan = plan_examples::timestamp();
    let values: Vec<_> = (0..64)
        .map(|i| FieldValue::Timestamp(1_700_000_000_000_000_000 + i * 1_000_000))
        .collect();
    assert_roundtrip(&ty, &plan, &values);
}

#[test]
fn example_price_pipeline() {
    let ty = LogicalType::fixed_decimal(18, 4, IntegerWidth::Bits64);
    let plan = plan_examples::price();
    let values: Vec<_> = (0..40)
        .map(|i| FieldValue::FixedDecimal(19_02500 + i * 3))
        .collect();
    assert_roundtrip(&ty, &plan, &values);
}

#[test]
fn example_symbol_pipeline() {
    let ty = LogicalType::categorical_string(Some(16));
    let plan = plan_examples::symbol();
    let symbols = ["AAPL", "MSFT", "AAPL", "GOOG", "AAPL", "MSFT"];
    let values: Vec<_> = symbols
        .iter()
        .map(|s| FieldValue::CategoricalString((*s).into()))
        .collect();
    assert_roundtrip(&ty, &plan, &values);
}

#[test]
fn example_side_pipeline() {
    let ty = LogicalType::enum_type(vec!["buy".into(), "sell".into()]);
    let plan = plan_examples::side();
    let values = vec![
        FieldValue::Enum("buy".into()),
        FieldValue::Enum("sell".into()),
        FieldValue::Enum("buy".into()),
        FieldValue::Enum("buy".into()),
    ];
    assert_roundtrip(&ty, &plan, &values);
}

#[test]
fn boolean_bitpack_with_nulls() {
    let ty = LogicalType::boolean();
    let plan = plan_for_field(&ty, FieldEncodingHint::Auto).unwrap();
    let values = vec![
        FieldValue::Boolean(true),
        FieldValue::Null,
        FieldValue::Boolean(false),
        FieldValue::Boolean(true),
        FieldValue::Null,
    ];
    assert_roundtrip(&ty, &plan, &values);
}

#[test]
fn plain_plan_all_types() {
    let cases: Vec<(LogicalType, Vec<FieldValue>)> = vec![
        (
            LogicalType::boolean(),
            vec![FieldValue::Boolean(true), FieldValue::Boolean(false)],
        ),
        (
            LogicalType::unsigned_integer(IntegerWidth::Bits64),
            vec![
                FieldValue::UnsignedInteger(0),
                FieldValue::UnsignedInteger(u64::MAX),
            ],
        ),
        (
            LogicalType::signed_integer(IntegerWidth::Bits64),
            vec![
                FieldValue::SignedInteger(i64::MIN),
                FieldValue::SignedInteger(0),
                FieldValue::SignedInteger(i64::MAX),
            ],
        ),
        (
            LogicalType::timestamp(TimestampUnit::Milliseconds),
            vec![FieldValue::Timestamp(0), FieldValue::Timestamp(-1)],
        ),
        (
            LogicalType::categorical_string(None),
            vec![
                FieldValue::CategoricalString("".into()),
                FieldValue::CategoricalString("x".into()),
            ],
        ),
        (
            LogicalType::binary(None),
            vec![
                FieldValue::Binary(vec![]),
                FieldValue::Binary(vec![1, 2, 3]),
            ],
        ),
    ];
    for (ty, values) in cases {
        let plan = EncodingPlan::plain();
        assert_roundtrip(&ty, &plan, &values);
    }
}

#[test]
fn auto_plans_roundtrip_stock_like() {
    let schema_cols: Vec<(LogicalType, EncodingPlan, Vec<FieldValue>)> = vec![
        (
            LogicalType::timestamp(TimestampUnit::Nanoseconds),
            plan_for_field(
                &LogicalType::timestamp(TimestampUnit::Nanoseconds),
                FieldEncodingHint::Auto,
            )
            .unwrap(),
            (0..100)
                .map(|i| FieldValue::Timestamp(1_000_000 + i * 50))
                .collect(),
        ),
        (
            LogicalType::fixed_decimal(18, 4, IntegerWidth::Bits64),
            plan_examples::price(),
            (0..100)
                .map(|i| FieldValue::FixedDecimal(10000 + (i % 17)))
                .collect(),
        ),
        (
            LogicalType::categorical_string(Some(8)),
            plan_examples::symbol(),
            (0..100)
                .map(|i| FieldValue::CategoricalString(["AAPL", "MSFT", "GOOG"][i % 3].into()))
                .collect(),
        ),
        (
            LogicalType::enum_type(vec!["buy".into(), "sell".into()]),
            plan_examples::side(),
            (0..100)
                .map(|i| {
                    FieldValue::Enum(if i % 2 == 0 {
                        "buy".into()
                    } else {
                        "sell".into()
                    })
                })
                .collect(),
        ),
    ];
    for (ty, plan, values) in schema_cols {
        assert_roundtrip(&ty, &plan, &values);
    }
}

#[test]
fn empty_and_all_null_columns() {
    let ty = LogicalType::unsigned_integer(IntegerWidth::Bits32);
    let plan = plan_for_field(&ty, FieldEncodingHint::Auto).unwrap();
    assert_roundtrip(&ty, &plan, &[]);
    assert_roundtrip(
        &ty,
        &plan,
        &[FieldValue::Null, FieldValue::Null, FieldValue::Null],
    );
}

#[test]
fn codec_trait_surface() {
    let codecs: &[&dyn Codec] = &[
        &PlainCodec,
        &DeltaCodec,
        &DeltaOfDeltaCodec,
        &ZigZagCodec,
        &VarintCodec,
        &FrameOfReferenceCodec,
        &BitPackingCodec,
        &RunLengthEncodingCodec,
        &DictionaryEncodingCodec,
    ];
    for c in codecs {
        let _ = c.id();
    }

    let v = Stream::I64(vec![1, 2, 4, 7]);
    let out = DeltaCodec.encode(v).unwrap();
    assert!(DeltaCodec.metadata(&out).is_none());
    let _ = DeltaCodec.encoded_size(&out);
    let back = DeltaCodec.decode(out.stream, None).unwrap();
    assert_eq!(back.into_i64().unwrap(), vec![1, 2, 4, 7]);
}

#[test]
fn plan_rejects_non_terminal_end() {
    let err = EncodingPlan::new(vec![EncodingStep::Delta, EncodingStep::ZigZag]).unwrap_err();
    assert!(err.to_string().contains("terminal"));
}

#[test]
fn plan_rejects_terminal_mid() {
    let err = EncodingPlan::new(vec![
        EncodingStep::BitPacking,
        EncodingStep::ZigZag,
        EncodingStep::Varint,
    ])
    .unwrap_err();
    assert!(err.to_string().contains("must be last"));
}

#[test]
fn delta_zigzag_varint_pipeline() {
    let ty = LogicalType::signed_integer(IntegerWidth::Bits64);
    let plan = EncodingPlan::new(vec![
        EncodingStep::Delta,
        EncodingStep::ZigZag,
        EncodingStep::Varint,
    ])
    .unwrap();
    let values: Vec<_> = (0..50)
        .map(|i| FieldValue::SignedInteger(1000 + i * 3))
        .collect();
    assert_roundtrip(&ty, &plan, &values);
}

#[test]
fn zigzag_rle_pipeline() {
    let ty = LogicalType::signed_integer(IntegerWidth::Bits32);
    let plan =
        EncodingPlan::new(vec![EncodingStep::ZigZag, EncodingStep::RunLengthEncoding]).unwrap();
    let values = vec![
        FieldValue::SignedInteger(5),
        FieldValue::SignedInteger(5),
        FieldValue::SignedInteger(5),
        FieldValue::SignedInteger(-1),
        FieldValue::SignedInteger(-1),
    ];
    assert_roundtrip(&ty, &plan, &values);
}

proptest! {
    #[test]
    fn prop_i64_dod_zigzag_bitpack(values in prop::collection::vec(any::<i64>(), 0..200)) {
        let ty = LogicalType::signed_integer(IntegerWidth::Bits64);
        let plan = EncodingPlan::new(vec![
            EncodingStep::DeltaOfDelta,
            EncodingStep::ZigZag,
            EncodingStep::BitPacking,
        ]).unwrap();
        let fv: Vec<_> = values.iter().copied().map(FieldValue::SignedInteger).collect();
        assert_roundtrip(&ty, &plan, &fv);
    }

    #[test]
    fn prop_i64_for_bitpack(values in prop::collection::vec(any::<i64>(), 0..200)) {
        let ty = LogicalType::signed_integer(IntegerWidth::Bits64);
        let plan = plan_examples::price();
        let fv: Vec<_> = values.iter().copied().map(FieldValue::SignedInteger).collect();
        assert_roundtrip(&ty, &plan, &fv);
    }

    #[test]
    fn prop_u64_bitpack(values in prop::collection::vec(any::<u64>(), 0..200)) {
        let ty = LogicalType::unsigned_integer(IntegerWidth::Bits64);
        let plan = EncodingPlan::new(vec![EncodingStep::BitPacking]).unwrap();
        let fv: Vec<_> = values.iter().copied().map(FieldValue::UnsignedInteger).collect();
        assert_roundtrip(&ty, &plan, &fv);
    }

    #[test]
    fn prop_u64_rle(values in prop::collection::vec(any::<u8>().prop_map(u64::from), 0..300)) {
        let ty = LogicalType::unsigned_integer(IntegerWidth::Bits64);
        let plan = EncodingPlan::new(vec![EncodingStep::RunLengthEncoding]).unwrap();
        let fv: Vec<_> = values.iter().copied().map(FieldValue::UnsignedInteger).collect();
        assert_roundtrip(&ty, &plan, &fv);
    }

    #[test]
    fn prop_u64_varint(values in prop::collection::vec(any::<u64>(), 0..150)) {
        let ty = LogicalType::unsigned_integer(IntegerWidth::Bits64);
        let plan = EncodingPlan::new(vec![EncodingStep::Varint]).unwrap();
        let fv: Vec<_> = values.iter().copied().map(FieldValue::UnsignedInteger).collect();
        assert_roundtrip(&ty, &plan, &fv);
    }

    #[test]
    fn prop_bool_bitpack(values in prop::collection::vec(any::<bool>(), 0..257)) {
        let ty = LogicalType::boolean();
        let plan = EncodingPlan::new(vec![EncodingStep::BooleanBitpack]).unwrap();
        let fv: Vec<_> = values.iter().copied().map(FieldValue::Boolean).collect();
        assert_roundtrip(&ty, &plan, &fv);
    }

    #[test]
    fn prop_dictionary_bitpack(
        values in prop::collection::vec(
            prop::sample::select(vec!["AAPL", "MSFT", "GOOG", "AMZN", ""]),
            0..200
        )
    ) {
        let ty = LogicalType::categorical_string(None);
        let plan = plan_examples::symbol();
        let fv: Vec<_> = values.iter().map(|s| FieldValue::CategoricalString((*s).into())).collect();
        assert_roundtrip(&ty, &plan, &fv);
    }

    #[test]
    fn prop_zigzag_lossless(n in any::<i64>()) {
        let enc = ZigZagCodec.encode(Stream::I64(vec![n])).unwrap();
        let dec = ZigZagCodec.decode(enc.stream, None).unwrap().into_i64().unwrap();
        assert_eq!(dec, vec![n]);
    }

    #[test]
    fn prop_plain_timestamps(values in prop::collection::vec(any::<i64>(), 0..100)) {
        let ty = LogicalType::timestamp(TimestampUnit::Nanoseconds);
        let plan = EncodingPlan::plain();
        let fv: Vec<_> = values.iter().copied().map(FieldValue::Timestamp).collect();
        assert_roundtrip(&ty, &plan, &fv);
    }
}
