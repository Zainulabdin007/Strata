//! Fixture validation only: these are not observations from the purchased corpus.
use dbn::{
    encode::{dbn::Encoder, EncodeRecord},
    MboMsg, Metadata, RecordHeader, SType,
};
use strata_input::{
    dbn::{canonical_bytes, MboReader},
    open_parser,
};
use strata_storage::{DatasetReader, DatasetWriter, DatasetWriterOptions};
#[test]
fn dbn_zstd_to_compressed_strata_matches_all_source_fields() {
    let root = std::env::temp_dir().join(format!("strata-dbn-roundtrip-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let metadata = Metadata::builder()
        .dataset("XNAS.ITCH")
        .schema(Some(dbn::Schema::Mbo))
        .start(0)
        .stype_in(Some(SType::RawSymbol))
        .stype_out(SType::InstrumentId)
        .build();
    let mut bytes = Vec::new();
    let mut expected = Vec::new();
    {
        let mut encoder = Encoder::new(&mut bytes, &metadata).unwrap();
        for i in 0..129u64 {
            let m = MboMsg {
                hd: RecordHeader::new::<MboMsg>(dbn::rtype::MBO, 2, 17, u64::MAX - i),
                order_id: u64::MAX - i,
                price: if i == 0 { i64::MAX } else { -(i as i64) },
                size: i as u32,
                flags: 255.into(),
                channel_id: 1,
                action: b'A' as _,
                side: b'B' as _,
                ts_recv: u64::MAX - i,
                ts_in_delta: i32::MIN + i as i32,
                sequence: i as u32,
            };
            expected.push(m.as_ref().to_vec());
            encoder.encode_record(&m).unwrap();
        }
    }
    let source = root.join("fixture.dbn.zst");
    std::fs::write(&source, zstd::encode_all(&bytes[..], 3).unwrap()).unwrap();
    let reader = MboReader::open(&source).unwrap();
    let schema = reader.schema();
    let profile =
        strata_core::parse_profile_str(&strata_input::dbn::mbo_profile(false), "fixture").unwrap();
    let dataset = root.join("dataset");
    let _ = std::fs::remove_dir_all(&dataset);
    let mut writer = DatasetWriter::create(
        &dataset,
        schema.clone(),
        DatasetWriterOptions {
            block_row_count: 17,
            compressor: std::sync::Arc::from(strata_codec::compressor_for(
                strata_codec::CompressionCodec::Zstd,
                Some(3),
            )),
            ..Default::default()
        },
    )
    .unwrap();
    let mut parser = open_parser(&profile).unwrap();
    parser
        .for_each_batch(&source, &schema, 17, &mut |batch| {
            writer.write_records(&batch.records).unwrap();
            Ok(())
        })
        .unwrap();
    writer.finish().unwrap();
    let rows = DatasetReader::open(&dataset)
        .unwrap()
        .read_all_records()
        .unwrap();
    let decoded = rows
        .iter()
        .map(|r| canonical_bytes(r, false).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(decoded, expected);
    let dataset_reader = DatasetReader::open(&dataset).unwrap();
    for workers in [1, 2, 4] {
        let mut actual = Vec::new();
        let names = std::sync::Arc::new(
            schema
                .fields
                .iter()
                .map(|f| f.name.clone())
                .collect::<Vec<_>>(),
        );
        let count = dataset_reader
            .scan_columns_parallel(workers, |meta, columns| {
                for row in 0..meta.row_count as usize {
                    let record = strata_core::Record::from_ordered(
                        names.clone(),
                        columns.iter().map(|c| c[row].clone()).collect(),
                    );
                    actual.push(canonical_bytes(&record, false).unwrap());
                }
                Ok(())
            })
            .unwrap();
        assert_eq!(count as usize, expected.len());
        assert_eq!(actual, expected);
    }
    assert!(dataset_reader
        .scan_columns_parallel(4, |_, _| Err(strata_storage::Error::InvalidState {
            message: "cancel test".into()
        }))
        .is_err());

    let mut malformed = schema.clone();
    malformed.fields.pop();
    assert!(parser
        .parse_file(&source, &malformed, &Default::default())
        .is_err());
    std::fs::remove_dir_all(root).unwrap();
}
