//! Lossless streaming Databento DBN MBO input using the vendor's reference decoder.
//! The canonical column order reconstructs every MBO record byte, including its header.
use crate::{Error, ParseOptions, RecordParser, Result};
use ::dbn::{
    decode::{DbnMetadata, DecodeRecordRef},
    MboMsg, VersionUpgradePolicy,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::File,
    io::{self, BufReader, Cursor, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, RecordBatch, Schema};

/// Exact canonical field order: (name, signed, bit width). No fields are dropped.
pub const MBO_FIELDS: &[(&str, bool, u8)] = &[
    ("length", false, 8),
    ("rtype", false, 8),
    ("publisher_id", false, 16),
    ("instrument_id", false, 32),
    ("ts_event", false, 64),
    ("order_id", false, 64),
    ("price", true, 64),
    ("size", false, 32),
    ("flags", false, 8),
    ("channel_id", false, 8),
    ("action", false, 8),
    ("side", false, 8),
    ("ts_recv", false, 64),
    ("ts_in_delta", true, 32),
    ("sequence", false, 32),
];
/// Schema retains unsigned timestamp sentinels and signed integer prices in 1e-9 units.
pub fn mbo_schema(ts_out: bool) -> Schema {
    let mut fields = MBO_FIELDS
        .iter()
        .map(|(name, signed, bits)| {
            let width = match bits {
                8 => IntegerWidth::Bits8,
                16 => IntegerWidth::Bits16,
                32 => IntegerWidth::Bits32,
                _ => IntegerWidth::Bits64,
            };
            Field::required(
                *name,
                if *signed {
                    LogicalType::signed_integer(width)
                } else {
                    LogicalType::unsigned_integer(width)
                },
            )
        })
        .collect::<Vec<_>>();
    if ts_out {
        fields.push(Field::required(
            "ts_out",
            LogicalType::unsigned_integer(IntegerWidth::Bits64),
        ));
    }
    Schema::new(
        if ts_out {
            "databento_mbo_v1_ts_out"
        } else {
            "databento_mbo_v1"
        },
        fields,
    )
    .expect("static schema")
}
/// Untuned manual starting profile. Plain semantic encoding plus Zstd level 3.
/// Automatic candidates are subsequently measured from real records.
pub fn mbo_profile(ts_out: bool) -> String {
    let mut out = format!(
        "name = \"databento-mbo-manual\"\ninput_format = \"dbn\"\n[schema]\nid = \"{}\"\n",
        mbo_schema(ts_out).id.as_str()
    );
    for (name, signed, bits) in MBO_FIELDS
        .iter()
        .copied()
        .chain(ts_out.then_some(("ts_out", false, 64)))
    {
        out.push_str(&format!("\n[[columns]]\nname = \"{name}\"\nlogical_type = \"{}_integer\"\nwidth = \"bits{bits}\"\nnullable = false\nencoding = [\"plain\"]\n",if signed{"signed"}else{"unsigned"}));
    }
    out.push_str("\n[block]\nrow_count = 16384\n[segment]\ntarget_size_bytes = 67108864\n[compression]\ncodec = \"zstd\"\nlevel = 3\n");
    out
}
fn invalid(path: &Path, message: impl Into<String>) -> Error {
    Error::MalformedDbn {
        path: path.to_owned(),
        message: message.into(),
    }
}
#[derive(Default)]
struct Observed {
    bytes: u64,
    hash: Sha256,
}
struct CountRead<R> {
    inner: R,
    state: Arc<Mutex<Observed>>,
    hash: bool,
}
impl<R: Read> Read for CountRead<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        // The DBN decoder treats UnexpectedEof as exhaustion. Preserve zstd truncation errors.
        let n = self.inner.read(out).map_err(|e| {
            if e.kind() == io::ErrorKind::UnexpectedEof {
                io::Error::new(io::ErrorKind::InvalidData, e)
            } else {
                e
            }
        })?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.bytes += n as u64;
        if self.hash {
            state.hash.update(&out[..n]);
        }
        Ok(n)
    }
}
/// Observed source and logical statistics. Partial samples never claim full-file counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbnStats {
    /// Canonical source path.
    pub source: PathBuf,
    /// Complete source file length (vendor transport bytes for .zst).
    pub source_file_bytes: u64,
    /// Whether outer Zstandard was detected by magic bytes.
    pub zstd: bool,
    /// Exact uncompressed metadata prefix length.
    pub metadata_bytes: u64,
    /// Bytes delivered by decompression, including decoder readahead on partial samples.
    pub decompressed_bytes_observed: u64,
    /// Header plus fully accepted records; deterministic sample boundary.
    pub dbn_bytes_accepted: u64,
    /// Complete accepted records.
    pub records: u64,
    /// Canonical payload bytes, with original integer widths.
    pub logical_bytes: u64,
    /// Distinct instrument IDs actually observed.
    pub instrument_ids: Vec<u32>,
    /// Minimum defined ts_event (nanoseconds); u64::MAX is counted separately.
    pub min_ts_event: Option<u64>,
    /// Maximum defined ts_event.
    pub max_ts_event: Option<u64>,
    /// Number of undefined ts_event sentinels.
    pub undefined_ts_event: u64,
    /// Ordered canonical-record SHA-256, equal to the original MBO record bytes hash.
    pub canonical_sha256: String,
    /// Full original source SHA-256, only available after verified EOF.
    pub source_sha256: Option<String>,
    /// True only after clean DBN and outer decompressor EOF.
    pub complete: bool,
    /// DBN metadata version, dataset, schema and symbol conventions.
    pub metadata: serde_json::Value,
}
/// Streaming decoder with bounded metadata and explicit truncation checks.
pub struct MboReader {
    decoder: ::dbn::decode::dbn::Decoder<Box<dyn Read + Send>>,
    raw_metadata: Vec<u8>,
    stats: DbnStats,
    compressed: Arc<Mutex<Observed>>,
    decompressed: Arc<Mutex<Observed>>,
    instruments: HashSet<u32>,
    canonical: Sha256,
    ts_out: bool,
    names: Arc<Vec<String>>,
}
impl MboReader {
    /// Open read-only. No expanded DBN temporary file is created.
    pub fn open(path: &Path) -> Result<Self> {
        let path = path.canonicalize().map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
        let run = || -> std::result::Result<Self, Box<dyn std::error::Error + Send + Sync>> {
            let source_file_bytes = std::fs::metadata(&path)?.len();
            let mut sniff = File::open(&path)?;
            let mut magic = [0; 4];
            sniff.read_exact(&mut magic)?;
            let zstd = magic == [0x28, 0xb5, 0x2f, 0xfd];
            let compressed = Arc::new(Mutex::new(Observed::default()));
            let decompressed = Arc::new(Mutex::new(Observed::default()));
            let source = CountRead {
                inner: File::open(&path)?,
                state: compressed.clone(),
                hash: true,
            };
            let stream: Box<dyn Read + Send> = if zstd {
                let mut d = zstd::stream::read::Decoder::new(source)?;
                d.window_log_max(27)?;
                Box::new(d)
            } else {
                Box::new(source)
            };
            let mut reader = CountRead {
                inner: stream,
                state: decompressed.clone(),
                hash: false,
            };
            let mut prefix = [0; 8];
            reader.read_exact(&mut prefix)?;
            if &prefix[..3] != b"DBN" {
                return Err("not a DBN stream".into());
            }
            let len = u32::from_le_bytes(prefix[4..8].try_into()?) as usize;
            if len > 16 * 1024 * 1024 {
                return Err("DBN metadata exceeds 16 MiB safety limit".into());
            }
            let mut raw_metadata = prefix.to_vec();
            raw_metadata.resize(8 + len, 0);
            reader.read_exact(&mut raw_metadata[8..])?;
            let input: Box<dyn Read + Send> = Box::new(
                Cursor::new(raw_metadata.clone())
                    .chain(BufReader::with_capacity(1024 * 1024, reader)),
            );
            let decoder = ::dbn::decode::dbn::Decoder::with_upgrade_policy(
                input,
                VersionUpgradePolicy::AsIs,
            )?;
            let m = decoder.metadata();
            if m.schema != Some(::dbn::Schema::Mbo) {
                return Err(
                    format!("DBN schema {:?} is not MBO; no records discarded", m.schema).into(),
                );
            }
            let ts_out = m.ts_out;
            let metadata = serde_json::json!({"dbn_version":m.version,"dataset":m.dataset,"schema":"mbo","ts_out":ts_out,"start":m.start.to_string(),"end":m.end.map(|n|n.to_string()),"stype_in":format!("{:?}",m.stype_in),"stype_out":format!("{:?}",m.stype_out),"symbol_cstr_len":m.symbol_cstr_len,"symbols":m.symbols,"partial":m.partial,"not_found":m.not_found,"mapping_count":m.mappings.len(),"decoder":"dbn 0.69.0","upgrade_policy":"AsIs"});
            let stats = DbnStats {
                source: path.clone(),
                source_file_bytes,
                zstd,
                metadata_bytes: raw_metadata.len() as u64,
                decompressed_bytes_observed: 0,
                dbn_bytes_accepted: raw_metadata.len() as u64,
                records: 0,
                logical_bytes: 0,
                instrument_ids: Vec::new(),
                min_ts_event: None,
                max_ts_event: None,
                undefined_ts_event: 0,
                canonical_sha256: String::new(),
                source_sha256: None,
                complete: false,
                metadata,
            };
            Ok(Self {
                decoder,
                raw_metadata,
                stats,
                compressed,
                decompressed,
                names: Arc::new(
                    mbo_schema(ts_out)
                        .fields
                        .iter()
                        .map(|f| f.name.clone())
                        .collect(),
                ),
                instruments: HashSet::new(),
                canonical: Sha256::new(),
                ts_out,
            })
        };
        run().map_err(|e| invalid(&path, e.to_string()))
    }
    /// Canonical schema discovered from the metadata's ts_out flag.
    pub fn schema(&self) -> Schema {
        mbo_schema(self.ts_out)
    }
    /// Exact original metadata bytes, including complete symbol mappings.
    pub fn raw_metadata(&self) -> &[u8] {
        &self.raw_metadata
    }
    /// Snapshot; complete=false until a clean EOF has been consumed.
    pub fn stats(&self) -> DbnStats {
        let mut s = self.stats.clone();
        s.decompressed_bytes_observed = self
            .decompressed
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .bytes;
        s.instrument_ids = self.instruments.iter().copied().collect();
        // The hot ingest path only needs membership. Sort once when publishing
        // statistics so census/sample artifacts remain deterministic.
        s.instrument_ids.sort_unstable();
        s.canonical_sha256 = format!("{:x}", self.canonical.clone().finalize());
        if s.complete {
            s.source_sha256 = Some(format!(
                "{:x}",
                self.compressed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .hash
                    .clone()
                    .finalize()
            ));
        }
        s
    }
    /// Decode at most n rows. Unknown record kinds or lengths fail explicitly.
    pub fn next_batch(&mut self, n: usize) -> Result<Vec<Record>> {
        if !(1..=65536).contains(&n) {
            return Err(invalid(
                &self.stats.source,
                "DBN batch must contain 1..65536 rows",
            ));
        }
        let mut rows = Vec::with_capacity(n);
        while rows.len() < n && !self.stats.complete {
            let next = self
                .decoder
                .decode_record_ref()
                .map_err(|e| invalid(&self.stats.source, e.to_string()))?;
            let Some(record) = next else {
                let observed = self
                    .decompressed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .bytes;
                if observed != self.stats.dbn_bytes_accepted {
                    return Err(invalid(
                        &self.stats.source,
                        format!(
                            "truncated DBN record: {observed} bytes observed, {} accepted",
                            self.stats.dbn_bytes_accepted
                        ),
                    ));
                }
                let source = self
                    .compressed
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .bytes;
                if source != self.stats.source_file_bytes {
                    return Err(invalid(&self.stats.source, "source not fully consumed"));
                }
                self.stats.complete = true;
                break;
            };
            let raw: &[u8] = record.as_ref();
            let expected = if self.ts_out { 64 } else { 56 };
            if raw.len() != expected {
                return Err(invalid(&self.stats.source,format!("unexpected MBO record length {} (expected {expected}); no extension bytes discarded",raw.len())));
            }
            let m = record
                .try_get::<MboMsg>()
                .map_err(|e| invalid(&self.stats.source, e.to_string()))?;
            let mut slots = Vec::with_capacity(self.names.len());
            let unsigned = [
                u64::from(raw[0]),
                u64::from(m.hd.rtype),
                u64::from(m.hd.publisher_id),
                u64::from(m.hd.instrument_id),
                m.hd.ts_event,
                m.order_id,
                m.price as u64,
                u64::from(m.size),
                u64::from(m.flags.raw()),
                u64::from(m.channel_id),
                u64::from(m.action as u8),
                u64::from(m.side as u8),
                m.ts_recv,
                m.ts_in_delta as u64,
                u64::from(m.sequence),
            ];
            for ((_, signed, _), v) in MBO_FIELDS.iter().zip(unsigned) {
                slots.push(if *signed {
                    FieldValue::SignedInteger(v as i64)
                } else {
                    FieldValue::UnsignedInteger(v)
                });
            }
            if self.ts_out {
                slots.push(FieldValue::UnsignedInteger(u64::from_le_bytes(
                    raw[56..64].try_into().expect("checked length"),
                )));
            }
            self.canonical.update(raw);
            self.stats.records += 1;
            self.stats.logical_bytes += expected as u64;
            self.stats.dbn_bytes_accepted += raw.len() as u64;
            self.instruments.insert(m.hd.instrument_id);
            if self.instruments.len() > 1_000_000 {
                return Err(invalid(
                    &self.stats.source,
                    "instrument count exceeds safety limit",
                ));
            }
            let ts = m.hd.ts_event;
            if ts == u64::MAX {
                self.stats.undefined_ts_event += 1;
            } else {
                self.stats.min_ts_event = Some(self.stats.min_ts_event.map_or(ts, |n| n.min(ts)));
                self.stats.max_ts_event = Some(self.stats.max_ts_event.map_or(ts, |n| n.max(ts)));
            }
            rows.push(Record::from_ordered(self.names.clone(), slots));
        }
        Ok(rows)
    }
}
/// Reconstruct exact MBO bytes from logical values for independent lossless verification.
pub fn canonical_bytes(record: &Record, ts_out: bool) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(if ts_out { 64 } else { 56 });
    for (name, signed, bits) in MBO_FIELDS
        .iter()
        .copied()
        .chain(ts_out.then_some(("ts_out", false, 64)))
    {
        let bytes = match (record.get(name), signed) {
            (Some(FieldValue::SignedInteger(n)), true)
                if bits == 64 || (*n >= -(1i64 << (bits - 1)) && *n < (1i64 << (bits - 1))) =>
            {
                n.to_le_bytes()
            }
            (Some(FieldValue::UnsignedInteger(n)), false) if bits == 64 || *n < (1u64 << bits) => {
                n.to_le_bytes()
            }
            _ => {
                return Err(invalid(
                    Path::new("<canonical MBO>"),
                    format!("missing, out-of-range, or wrong type: {name}"),
                ))
            }
        };
        out.extend_from_slice(&bytes[..usize::from(bits / 8)]);
    }
    Ok(out)
}
/// RecordParser adapter used by the normal CLI/API ingest/analyze/optimizer path.
#[derive(Default)]
pub struct DbnParser;
impl RecordParser for DbnParser {
    fn format_name(&self) -> &'static str {
        "dbn"
    }
    fn parse_file(
        &mut self,
        path: &Path,
        schema: &Schema,
        options: &ParseOptions,
    ) -> Result<RecordBatch> {
        let mut reader = MboReader::open(path)?;
        if &reader.schema() != schema {
            return Err(invalid(
                path,
                "profile must retain the complete discovered DBN MBO schema",
            ));
        }
        let limit = options.max_batch_rows.unwrap_or(65536).min(65536);
        let mut batch = RecordBatch::empty(schema.clone());
        if limit > 0 {
            for r in reader.next_batch(limit)? {
                batch.push(r)?;
            }
        }
        Ok(batch)
    }
    fn for_each_batch(
        &mut self,
        path: &Path,
        schema: &Schema,
        batch_rows: usize,
        on_batch: &mut dyn FnMut(RecordBatch) -> Result<()>,
    ) -> Result<()> {
        let mut reader = MboReader::open(path)?;
        if &reader.schema() != schema {
            return Err(invalid(
                path,
                "profile must retain the complete discovered DBN MBO schema",
            ));
        }
        loop {
            let rows = reader.next_batch(batch_rows.min(65536))?;
            if rows.is_empty() {
                break;
            }
            let mut batch = RecordBatch::empty(schema.clone());
            for r in rows {
                batch.push(r)?;
            }
            on_batch(batch)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::dbn::{
        encode::{dbn::Encoder, EncodeRecord},
        Metadata, RecordHeader, SType, WithTsOut,
    };
    fn fixture(ts_out: bool) -> (Vec<u8>, Vec<Vec<u8>>) {
        let metadata = Metadata::builder()
            .dataset("XNAS.ITCH")
            .schema(Some(::dbn::Schema::Mbo))
            .start(0)
            .stype_in(Some(SType::RawSymbol))
            .stype_out(SType::InstrumentId)
            .ts_out(ts_out)
            .build();
        let mut out = Vec::new();
        let mut expected = Vec::new();
        {
            let mut encoder = Encoder::new(&mut out, &metadata).unwrap();
            for i in 0..17u64 {
                let m = MboMsg {
                    hd: RecordHeader::new::<MboMsg>(
                        ::dbn::rtype::MBO,
                        u16::MAX,
                        100 + i as u32,
                        if i == 0 {
                            u64::MAX
                        } else {
                            1_789_000_000_000_000_000 + i
                        },
                    ),
                    order_id: u64::MAX - i,
                    price: if i == 0 {
                        i64::MAX
                    } else {
                        i64::MIN + i as i64
                    },
                    size: u32::MAX - i as u32,
                    flags: 255.into(),
                    channel_id: 255,
                    action: b'A' as _,
                    side: b'B' as _,
                    ts_recv: u64::MAX - i,
                    ts_in_delta: i32::MIN + i as i32,
                    sequence: u32::MAX - i as u32,
                };
                if ts_out {
                    let m = WithTsOut::new(m, u64::MAX - i);
                    expected.push(m.as_ref().to_vec());
                    encoder.encode_record(&m).unwrap();
                } else {
                    expected.push(m.as_ref().to_vec());
                    encoder.encode_record(&m).unwrap();
                }
            }
        }
        (out, expected)
    }
    fn path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("strata-dbn-{}-{label}", std::process::id()))
    }
    #[test]
    fn reference_dbn_and_zstd_preserve_every_byte_and_sentinel() {
        for ts_out in [false, true] {
            let (raw, expected) = fixture(ts_out);
            for compressed in [false, true] {
                let path = path(&format!("{ts_out}-{compressed}"));
                let bytes = if compressed {
                    zstd::encode_all(&raw[..], 3).unwrap()
                } else {
                    raw.clone()
                };
                std::fs::write(&path, &bytes).unwrap();
                let mut r = MboReader::open(&path).unwrap();
                let mut actual = Vec::new();
                loop {
                    let rows = r.next_batch(3).unwrap();
                    if rows.is_empty() {
                        break;
                    }
                    for row in rows {
                        actual.push(canonical_bytes(&row, ts_out).unwrap());
                    }
                }
                assert_eq!(actual, expected);
                let s = r.stats();
                assert!(s.complete);
                assert_eq!(s.records, 17);
                assert_eq!(s.decompressed_bytes_observed, raw.len() as u64);
                assert_eq!(s.source_file_bytes, bytes.len() as u64);
                assert_eq!(
                    s.source_sha256,
                    Some(format!("{:x}", Sha256::digest(bytes)))
                );
                assert_eq!(s.undefined_ts_event, 1);
                assert_eq!(s.instrument_ids.len(), 17);
                assert_eq!(s.instrument_ids, (100..117).collect::<Vec<_>>());
                strata_core::parse_profile_str(&mbo_profile(ts_out), "fixture").unwrap();
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    #[test]
    fn canonical_hash_cannot_hide_integer_truncation() {
        let (raw, _) = fixture(false);
        let path = path("canonical-width");
        std::fs::write(&path, raw).unwrap();
        let mut reader = MboReader::open(&path).unwrap();
        let mut row = reader.next_batch(1).unwrap().remove(0);
        row.insert("action", FieldValue::UnsignedInteger(256));
        assert!(canonical_bytes(&row, false).is_err());
        row.insert("action", FieldValue::UnsignedInteger(65));
        row.insert("ts_in_delta", FieldValue::SignedInteger(i64::MAX));
        assert!(canonical_bytes(&row, false).is_err());
        std::fs::remove_file(path).unwrap();
    }
    #[test]
    fn partial_record_and_zstd_truncation_are_errors() {
        let (raw, _) = fixture(false);
        let encoded = zstd::encode_all(&raw[..], 3).unwrap();
        for (label, bytes) in [
            ("raw-tail", &raw[..raw.len() - 1]),
            ("zstd-tail", &encoded[..encoded.len() - 1]),
        ] {
            let path = path(label);
            std::fs::write(&path, bytes).unwrap();
            let result = (|| -> Result<()> {
                let mut r = MboReader::open(&path)?;
                while !r.next_batch(7)?.is_empty() {}
                Ok(())
            })();
            assert!(result.is_err(), "accepted {label}");
            std::fs::remove_file(path).unwrap();
        }
    }
    #[test]
    fn bounded_metadata_and_partial_statistics() {
        let path = path("metadata");
        std::fs::write(&path, [b'D', b'B', b'N', 3, 255, 255, 255, 255]).unwrap();
        assert!(MboReader::open(&path).is_err());
        let (raw, _) = fixture(false);
        std::fs::write(&path, raw).unwrap();
        let mut r = MboReader::open(&path).unwrap();
        assert_eq!(r.next_batch(1).unwrap().len(), 1);
        let s = r.stats();
        assert!(!s.complete);
        assert_eq!(s.records, 1);
        assert!(s.source_sha256.is_none());
        std::fs::remove_file(path).unwrap();
    }
}
