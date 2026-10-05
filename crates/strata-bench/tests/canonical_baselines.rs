//! Format regression fixtures; not measurements from the purchased corpus.
use strata_bench::baselines::{
    fixed::{decode_fixed_records, FixedWriter},
    parquet_baseline::{decode_parquet_records, ParquetZstdWriter},
};
use strata_core::{FieldValue, Record};
use strata_input::dbn::{canonical_bytes, mbo_schema, MBO_FIELDS};
#[test]
fn all_mbo_fields_survive_each_baseline_and_invalid_widths_fail() {
    let root =
        std::env::temp_dir().join(format!("strata-baseline-canonical-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let schema = mbo_schema(true);
    let rows: Vec<_> = (0..19u64)
        .map(|i| {
            let mut row = Record::new();
            for &(name, signed, bits) in MBO_FIELDS {
                row.insert(
                    name,
                    if signed {
                        FieldValue::SignedInteger(if bits == 64 {
                            i64::MIN + i as i64
                        } else {
                            i32::MIN as i64 + i as i64
                        })
                    } else {
                        FieldValue::UnsignedInteger(if bits == 64 {
                            u64::MAX - i
                        } else {
                            (1u64 << bits) - 1 - i
                        })
                    },
                );
            }
            row.insert("ts_out", FieldValue::UnsignedInteger(u64::MAX - i));
            row
        })
        .collect();
    let expected: Vec<_> = rows
        .iter()
        .map(|r| canonical_bytes(r, true).unwrap())
        .collect();
    let fixed = root.join("data.fixed");
    let mut writer = FixedWriter::create(&fixed, &schema).unwrap();
    writer.write_chunk(&rows).unwrap();
    assert_eq!(writer.finish().unwrap(), 19);
    let mut actual = Vec::new();
    decode_fixed_records(&fixed, &schema, &mut |r| {
        actual.push(canonical_bytes(&r, true)?);
        Ok(())
    })
    .unwrap();
    assert_eq!(actual, expected);
    let parquet = root.join("data.parquet");
    let mut writer = ParquetZstdWriter::create(&parquet, &schema).unwrap();
    writer.write_chunk(&rows).unwrap();
    assert_eq!(writer.finish().unwrap(), 19);
    actual.clear();
    decode_parquet_records(&parquet, &schema, &mut |r| {
        actual.push(canonical_bytes(&r, true)?);
        Ok(())
    })
    .unwrap();
    assert_eq!(actual, expected);
    let mut packed = Vec::new();
    let n = strata_bench::baselines::parquet_baseline::decode_parquet_integer_bytes(
        &parquet,
        &schema,
        &mut |bytes| {
            packed.extend_from_slice(bytes);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(n, rows.len() as u64);
    assert_eq!(packed, expected.concat());
    assert_eq!(
        strata_bench::baselines::parquet_baseline::scan_parquet_columns(&parquet).unwrap(),
        n
    );
    assert_eq!(
        strata_bench::baselines::parquet_baseline::scan_parquet_columns_parallel(&parquet, 2)
            .unwrap(),
        rows.len() as u64
    );
    let mut invalid = rows[0].clone();
    invalid.insert("size", FieldValue::UnsignedInteger(u64::MAX));
    assert!(FixedWriter::create(&root.join("invalid.fixed"), &schema)
        .unwrap()
        .write_chunk(&[invalid.clone()])
        .is_err());
    assert!(
        ParquetZstdWriter::create(&root.join("invalid.parquet"), &schema)
            .unwrap()
            .write_chunk(&[invalid])
            .is_err()
    );
    let empty = root.join("empty.fixed");
    assert_eq!(
        FixedWriter::create(&empty, &schema)
            .unwrap()
            .finish()
            .unwrap(),
        0
    );
    assert_eq!(
        decode_fixed_records(&empty, &schema, &mut |_| panic!("empty file yielded a row"))
            .unwrap()
            .0,
        0
    );
    std::fs::remove_dir_all(root).unwrap();
}
#[test]
fn parquet_preserves_nullable_integer_and_boolean_values() {
    use strata_core::{Field, IntegerWidth, LogicalType, Schema};
    let schema = Schema::new(
        "nullable",
        vec![
            Field::nullable("n", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::nullable("b", LogicalType::boolean()),
        ],
    )
    .unwrap();
    let rows = vec![
        Record::new()
            .with("n", FieldValue::Null)
            .with("b", FieldValue::Null),
        Record::new()
            .with("n", FieldValue::UnsignedInteger(u64::MAX))
            .with("b", FieldValue::Boolean(false)),
    ];
    let path = std::env::temp_dir().join(format!(
        "strata-nullable-baseline-{}.parquet",
        std::process::id()
    ));
    let mut writer = ParquetZstdWriter::create(&path, &schema).unwrap();
    writer.write_chunk(&rows).unwrap();
    writer.finish().unwrap();
    let mut actual = Vec::new();
    decode_parquet_records(&path, &schema, &mut |r| {
        actual.push(r);
        Ok(())
    })
    .unwrap();
    assert_eq!(actual, rows);
    std::fs::remove_file(path).unwrap();
}
