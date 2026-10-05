//! BlockWriter: encode records into columnar blocks via encoding plans + optional compression.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use strata_codec::{
    encode_column_refs, plan_for_field, Compressor, EncodingPlan, IdentityCompressor,
};
use strata_core::{FieldEncodingHint, FieldValue, Record, Schema};
use strata_format::{
    crc32, BlockEncoding, BlockId, BlockMeta, ColumnBlockMeta, BLOCK_FRAME_VERSION,
    ENCODING_PIPELINE,
};

use crate::error::{Error, Result};
use crate::io::{write_bytes, write_u32};

/// In-memory encoded block ready to append to a segment.
#[derive(Debug, Clone)]
pub struct EncodedBlock {
    /// Full block bytes including frame header and trailing CRC32.
    pub bytes: Vec<u8>,
    /// Block metadata (file_offset filled by the segment writer).
    pub meta: BlockMeta,
    /// Time spent building the semantic body (column encode + framing prep).
    pub semantic_encode_duration: Duration,
    /// Time spent in general compression (`0` when codec is `none`).
    pub compress_duration: Duration,
}

/// Builds a columnar block from records using composable semantic encodings.
pub struct BlockWriter {
    schema: Schema,
    records: Vec<Record>,
    /// Optional per-field plans; missing fields use [`plan_for_field`] with Auto.
    plans: HashMap<String, EncodingPlan>,
    /// General compressor applied to the semantic block body.
    compressor: Arc<dyn Compressor>,
}

impl BlockWriter {
    /// Creates an empty block writer for `schema` (Auto plans, no general compression).
    pub fn new(schema: Schema) -> Self {
        Self {
            schema,
            records: Vec::new(),
            plans: HashMap::new(),
            compressor: Arc::new(IdentityCompressor),
        }
    }

    /// Creates a writer with explicit per-field encoding plans.
    pub fn with_plans(schema: Schema, plans: HashMap<String, EncodingPlan>) -> Self {
        Self {
            schema,
            records: Vec::new(),
            plans,
            compressor: Arc::new(IdentityCompressor),
        }
    }

    /// Creates a writer with plans and a general block compressor.
    pub fn with_plans_and_compressor(
        schema: Schema,
        plans: HashMap<String, EncodingPlan>,
        compressor: Arc<dyn Compressor>,
    ) -> Self {
        Self {
            schema,
            records: Vec::new(),
            plans,
            compressor,
        }
    }

    /// Overrides the encoding plan for one field.
    pub fn set_plan(&mut self, field: impl Into<String>, plan: EncodingPlan) {
        self.plans.insert(field.into(), plan);
    }

    /// Sets the general block compressor.
    pub fn set_compressor(&mut self, compressor: Arc<dyn Compressor>) {
        self.compressor = compressor;
    }

    /// Number of buffered rows.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Returns true when no rows are buffered.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Appends a validated record.
    pub fn push(&mut self, record: Record) -> Result<()> {
        record.validate_against(&self.schema)?;
        self.records.push(record);
        Ok(())
    }

    /// Appends many records.
    pub fn extend<I>(&mut self, records: I) -> Result<()>
    where
        I: IntoIterator<Item = Record>,
    {
        for record in records {
            self.push(record)?;
        }
        Ok(())
    }

    /// Consumes the writer and produces an encoded (+ optionally compressed) block.
    pub fn finish(self, block_id: BlockId) -> Result<EncodedBlock> {
        encode_block(
            &self.schema,
            &self.records,
            block_id,
            &self.plans,
            self.compressor.as_ref(),
        )
    }
}

/// Encodes `records` into a columnar block, then optionally generally-compresses the body.
pub fn encode_block(
    schema: &Schema,
    records: &[Record],
    block_id: BlockId,
    plans: &HashMap<String, EncodingPlan>,
    compressor: &dyn Compressor,
) -> Result<EncodedBlock> {
    let semantic_started = Instant::now();
    let row_count = u32::try_from(records.len()).map_err(|_| Error::InvalidState {
        message: "block row count exceeds u32".into(),
    })?;

    let mut column_payloads: Vec<(String, Vec<u8>, u64)> = Vec::with_capacity(schema.len());

    let ordered = records.iter().all(|r| r.is_schema_ordered(schema));
    for (ordinal, field) in schema.fields.iter().enumerate() {
        let values = records.iter().map(|record| {
            if ordered {
                record.get_ordinal(ordinal)
            } else {
                record.get(&field.name)
            }
            .unwrap_or(&FieldValue::Null)
        });
        let fallback;
        let plan = match plans.get(&field.name) {
            Some(p) => p,
            None => {
                fallback = plan_for_field(&field.logical_type, FieldEncodingHint::Auto)?;
                &fallback
            }
        };
        let encoded = encode_column_refs(&field.logical_type, values, plan)?;
        column_payloads.push((field.name.clone(), encoded.bytes, encoded.logical_bytes));
    }

    let column_count = u32::try_from(column_payloads.len()).map_err(|_| Error::InvalidState {
        message: "column count exceeds u32".into(),
    })?;

    let mut dir_size: u32 = 8;
    for (name, _, _) in &column_payloads {
        let name_len = u32::try_from(name.len()).map_err(|_| Error::InvalidState {
            message: "column name too long".into(),
        })?;
        dir_size = dir_size
            .checked_add(4 + name_len + 1 + 4 + 4)
            .ok_or_else(|| Error::InvalidState {
                message: "block directory too large".into(),
            })?;
    }

    let mut meta = BlockMeta::new(block_id, row_count);
    let mut rel = dir_size;
    let mut columns_meta = Vec::with_capacity(column_payloads.len());
    for (name, payload, logical_bytes) in &column_payloads {
        let data_len = u32::try_from(payload.len()).map_err(|_| Error::InvalidState {
            message: "column payload exceeds u32".into(),
        })?;
        columns_meta.push(ColumnBlockMeta {
            field_name: name.clone(),
            encoding: BlockEncoding::Pipeline,
            data_rel_offset: rel,
            data_len,
            logical_bytes: *logical_bytes,
            encoded_bytes: u64::from(data_len),
        });
        rel = rel
            .checked_add(data_len)
            .ok_or_else(|| Error::InvalidState {
                message: "block payload offset overflow".into(),
            })?;
    }
    for col in columns_meta {
        meta.push_column(col);
    }

    // Semantic body: directory + column payloads (pre-general-compression).
    let mut body = Vec::new();
    let path = std::path::Path::new("<memory>");
    write_u32(&mut body, path, row_count)?;
    write_u32(&mut body, path, column_count)?;
    for col in &meta.columns {
        let name_len = u32::try_from(col.field_name.len()).unwrap();
        write_u32(&mut body, path, name_len)?;
        write_bytes(&mut body, path, col.field_name.as_bytes())?;
        body.push(ENCODING_PIPELINE);
        write_u32(&mut body, path, col.data_rel_offset)?;
        write_u32(&mut body, path, col.data_len)?;
    }
    debug_assert_eq!(body.len(), dir_size as usize);
    for (_, payload, _) in &column_payloads {
        write_bytes(&mut body, path, payload)?;
    }

    let semantic_encode_duration = semantic_started.elapsed();
    let semantic_encoded_bytes = body.len() as u64;

    let compress_started = Instant::now();
    let compressed = if compressor.codec() == strata_codec::CompressionCodec::None {
        body
    } else {
        compressor.compress(&body)?
    };
    let compress_duration = if matches!(compressor.codec(), strata_codec::CompressionCodec::None) {
        Duration::ZERO
    } else {
        compress_started.elapsed()
    };
    let compressed_bytes = compressed.len() as u64;
    let uncompressed_len =
        u32::try_from(semantic_encoded_bytes).map_err(|_| Error::InvalidState {
            message: "semantic block body exceeds u32".into(),
        })?;
    let compressed_len = u32::try_from(compressed.len()).map_err(|_| Error::InvalidState {
        message: "compressed block payload exceeds u32".into(),
    })?;

    // Frame: version | codec | uncompressed_len | compressed_len | payload | crc32
    let mut bytes = Vec::with_capacity(1 + 1 + 4 + 4 + compressed.len() + 4);
    bytes.push(BLOCK_FRAME_VERSION);
    bytes.push(compressor.codec().tag());
    bytes.extend_from_slice(&uncompressed_len.to_le_bytes());
    bytes.extend_from_slice(&compressed_len.to_le_bytes());
    bytes.extend_from_slice(&compressed);
    let checksum = crc32(&bytes);
    bytes.extend_from_slice(&checksum.to_le_bytes());

    meta.semantic_encoded_bytes = semantic_encoded_bytes;
    meta.compressed_bytes = compressed_bytes;
    meta.compression_codec = compressor.codec().tag();
    meta.block_length = bytes.len() as u64;

    Ok(EncodedBlock {
        bytes,
        meta,
        semantic_encode_duration,
        compress_duration,
    })
}
