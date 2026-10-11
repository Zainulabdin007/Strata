//! BlockReader: decode a columnar block into a [`RecordBatch`].

use std::{borrow::Cow, path::Path, sync::Arc};

use strata_codec::{compressor_for, decode_column, ColumnDecoder, CompressionCodec, PlainEncoder};
use strata_core::{FieldValue, LogicalType, Record, RecordBatch, Schema};
use strata_format::{crc32, BlockEncoding, BlockMeta, ColumnBlockMeta, BLOCK_FRAME_VERSION};

use crate::error::{Error, Result};
use crate::io::BufCursor;

/// Reads columnar blocks encoded by [`super::BlockWriter`].
#[derive(Debug, Default)]
pub struct BlockReader;

impl BlockReader {
    /// Creates a block reader.
    pub fn new() -> Self {
        Self
    }

    /// Decodes block bytes into a [`RecordBatch`] using `schema`.
    pub fn read_batch(&self, schema: &Schema, bytes: &[u8]) -> Result<RecordBatch> {
        let (meta, columns) = self.read_columns(schema, bytes, Path::new("<memory>"))?;
        columns_to_batch(schema, meta.row_count, columns)
    }

    /// Decodes block bytes and returns metadata plus per-column values.
    pub fn read_columns(
        &self,
        schema: &Schema,
        bytes: &[u8],
        path: &Path,
    ) -> Result<(BlockMeta, Vec<Vec<FieldValue>>)> {
        let (body, frame_meta) = open_block_frame(bytes, path)?;
        decode_semantic_body(schema, &body, path, frame_meta, bytes.len() as u64, None)
    }

    /// Decodes only the named columns (plus validates the full column directory).
    ///
    /// Columns not in `project` are not decoded. The returned column vectors
    /// align with `project` order (not full schema order).
    pub fn read_columns_projected(
        &self,
        schema: &Schema,
        bytes: &[u8],
        path: &Path,
        project: &[String],
    ) -> Result<(BlockMeta, Vec<Vec<FieldValue>>)> {
        let (body, frame_meta) = open_block_frame(bytes, path)?;
        decode_semantic_body(
            schema,
            &body,
            path,
            frame_meta,
            bytes.len() as u64,
            Some(project),
        )
    }

    /// Decodes a projected [`RecordBatch`] (schema contains only projected fields).
    pub fn read_batch_projected(
        &self,
        schema: &Schema,
        bytes: &[u8],
        project: &[String],
    ) -> Result<RecordBatch> {
        let (meta, columns) =
            self.read_columns_projected(schema, bytes, Path::new("<memory>"), project)?;
        let projected_schema = project_schema(schema, project)?;
        columns_to_batch(&projected_schema, meta.row_count, columns)
    }
}

struct FrameMeta {
    compression_codec: u8,
    semantic_encoded_bytes: u64,
    compressed_bytes: u64,
}

fn open_block_frame<'a>(bytes: &'a [u8], path: &Path) -> Result<(Cow<'a, [u8]>, FrameMeta)> {
    if bytes.len() < 1 + 1 + 4 + 4 + 4 {
        return Err(Error::corrupt(path, "block too short for frame header"));
    }
    let (framed, crc_bytes) = bytes.split_at(bytes.len() - 4);
    let expected = u32::from_le_bytes(crc_bytes.try_into().unwrap());
    let actual = crc32(framed);
    if expected != actual {
        return Err(Error::corrupt(
            path,
            format!("block CRC32 mismatch: expected {expected:#x}, got {actual:#x}"),
        ));
    }

    let version = framed[0];
    if version != BLOCK_FRAME_VERSION {
        return Err(Error::corrupt(
            path,
            format!("unsupported block frame version {version}"),
        ));
    }
    let codec_tag = framed[1];
    let codec = CompressionCodec::from_tag(codec_tag).ok_or_else(|| {
        Error::corrupt(
            path,
            format!("unknown block compression codec tag {codec_tag}"),
        )
    })?;
    let uncompressed_len = u32::from_le_bytes(framed[2..6].try_into().unwrap()) as usize;
    let compressed_len = u32::from_le_bytes(framed[6..10].try_into().unwrap()) as usize;
    if framed.len() != 10 + compressed_len {
        return Err(Error::corrupt(
            path,
            format!(
                "block frame length mismatch: header+payload={}, declared compressed_len={compressed_len}",
                framed.len().saturating_sub(10)
            ),
        ));
    }
    let payload = &framed[10..];
    let body = if codec == CompressionCodec::None {
        if payload.len() != uncompressed_len {
            return Err(Error::corrupt(path, "identity frame length mismatch"));
        }
        Cow::Borrowed(payload)
    } else {
        Cow::Owned(
            compressor_for(codec, None)
                .decompress(payload, uncompressed_len)
                .map_err(Error::from)?,
        )
    };

    Ok((
        body,
        FrameMeta {
            compression_codec: codec_tag,
            semantic_encoded_bytes: uncompressed_len as u64,
            compressed_bytes: compressed_len as u64,
        },
    ))
}

fn decode_semantic_body(
    schema: &Schema,
    body: &[u8],
    path: &Path,
    frame: FrameMeta,
    on_disk_len: u64,
    project: Option<&[String]>,
) -> Result<(BlockMeta, Vec<Vec<FieldValue>>)> {
    let mut cur = BufCursor::new(body, path);
    let row_count = cur.read_u32()?;
    strata_codec::limits::check_count(row_count as usize)?;
    let column_count = cur.read_u32()? as usize;
    if (row_count as usize)
        .checked_mul(column_count)
        .is_none_or(|n| n > 4_194_304)
    {
        return Err(Error::corrupt(path, "block has too many cells"));
    }
    if column_count != schema.len() {
        return Err(Error::corrupt(
            path,
            format!(
                "block column_count {column_count} does not match schema length {}",
                schema.len()
            ),
        ));
    }

    let mut col_metas = Vec::with_capacity(column_count);
    for expected_field in &schema.fields {
        let name = cur.read_string()?;
        if name != expected_field.name {
            return Err(Error::corrupt(
                path,
                format!(
                    "block column name `{name}` does not match schema field `{}`",
                    expected_field.name
                ),
            ));
        }
        let encoding_tag = cur.read_u8()?;
        let encoding = BlockEncoding::from_u8(encoding_tag)
            .ok_or_else(|| Error::corrupt(path, format!("unknown encoding tag {encoding_tag}")))?;
        if !matches!(encoding, BlockEncoding::Pipeline | BlockEncoding::Plain) {
            return Err(Error::corrupt(
                path,
                format!(
                    "unsupported column encoding {:?}; expected Pipeline or Plain",
                    encoding
                ),
            ));
        }
        let data_rel_offset = cur.read_u32()?;
        let data_len = cur.read_u32()?;
        col_metas.push(ColumnBlockMeta {
            field_name: name,
            encoding,
            data_rel_offset,
            data_len,
            logical_bytes: 0,
            encoded_bytes: u64::from(data_len),
        });
    }

    let mut end_offset = cur.position();
    for column in &col_metas {
        if column.data_rel_offset as usize != end_offset {
            return Err(Error::corrupt(
                path,
                "column ranges overlap or contain gaps",
            ));
        }
        end_offset = end_offset
            .checked_add(column.data_len as usize)
            .ok_or_else(|| Error::corrupt(path, "column length overflow"))?;
        if end_offset > body.len() {
            return Err(Error::corrupt(path, "column exceeds body"));
        }
    }
    if end_offset != body.len() {
        return Err(Error::corrupt(path, "trailing block body bytes"));
    }
    let decode_names: Vec<String> = match project {
        None => schema.fields.iter().map(|f| f.name.clone()).collect(),
        Some(names) => {
            for name in names {
                if schema.field(name).is_none() {
                    return Err(Error::InvalidState {
                        message: format!("projected column `{name}` is not in schema"),
                    });
                }
            }
            names.to_vec()
        }
    };

    let plain = PlainEncoder;
    let mut columns = Vec::with_capacity(decode_names.len());
    let mut decoded_bytes = 0u64;
    for name in &decode_names {
        let field = schema.field(name).expect("validated");
        let col_meta = col_metas
            .iter()
            .find(|c| c.field_name == *name)
            .ok_or_else(|| Error::corrupt(path, format!("missing column meta for `{name}`")))?;
        let start = col_meta.data_rel_offset as usize;
        let end = start
            .checked_add(col_meta.data_len as usize)
            .ok_or_else(|| Error::corrupt(path, "column offset overflow"))?;
        if end > body.len() {
            return Err(Error::corrupt(
                path,
                format!(
                    "column `{name}` extends past block body (end={end}, body={})",
                    body.len()
                ),
            ));
        }
        let payload = &body[start..end];
        let values = match col_meta.encoding {
            BlockEncoding::Pipeline => decode_column(&field.logical_type, row_count, payload)?,
            BlockEncoding::Plain => plain.decode(&field.logical_type, row_count, payload)?,
            other => {
                return Err(Error::corrupt(
                    path,
                    format!("unsupported decode for encoding {other:?}"),
                ));
            }
        };
        if values.len() != row_count as usize {
            return Err(Error::corrupt(
                path,
                format!(
                    "column `{name}` decoded {} values, expected {row_count}",
                    values.len()
                ),
            ));
        }
        if !field.nullable && values.iter().any(FieldValue::is_null) {
            return Err(Error::corrupt(
                path,
                format!("null in required column {name}"),
            ));
        }
        // Legacy plain columns also need domain/bound validation. Pipeline codecs
        // already enforce these checks before returning their typed values.
        if col_meta.encoding == BlockEncoding::Plain {
            for value in &values {
                strata_core::validate_field_value(field, value)?;
            }
        }
        // Fixed-width, required columns have already passed type and null
        // validation. Their logical size follows from the schema and count;
        // another traversal of the decoded values would do redundant work.
        let fixed_width = match &field.logical_type {
            LogicalType::Boolean(_) => Some(1),
            LogicalType::UnsignedInteger(t) => Some(u64::from(t.width.bytes())),
            LogicalType::SignedInteger(t) => Some(u64::from(t.width.bytes())),
            LogicalType::FixedDecimal(t) => Some(u64::from(t.width.bytes())),
            LogicalType::Timestamp(_) => Some(8),
            _ => None,
        };
        let logical_bytes: u64 = match fixed_width.filter(|_| !field.nullable) {
            Some(width) => width * u64::from(row_count),
            None => values
                .iter()
                .map(|v| strata_core::logical_value_bytes(&field.logical_type, v))
                .sum(),
        };
        if let Some(meta) = col_metas.iter_mut().find(|c| c.field_name == *name) {
            meta.logical_bytes = logical_bytes;
        }
        decoded_bytes += logical_bytes;
        if decoded_bytes > 256 * 1024 * 1024 {
            return Err(Error::corrupt(path, "decoded block exceeds 256 MiB"));
        }
        columns.push(values);
    }

    let mut meta = BlockMeta::new(strata_format::BlockId(0), row_count);
    meta.block_length = on_disk_len;
    meta.semantic_encoded_bytes = frame.semantic_encoded_bytes;
    meta.compressed_bytes = frame.compressed_bytes;
    meta.compression_codec = frame.compression_codec;
    for col in col_metas {
        meta.push_column(col);
    }

    Ok((meta, columns))
}

/// Builds a schema containing only `project` fields (in that order).
pub fn project_schema(schema: &Schema, project: &[String]) -> Result<Schema> {
    let mut fields = Vec::with_capacity(project.len());
    for name in project {
        let field = schema.field(name).ok_or_else(|| Error::InvalidState {
            message: format!("projected column `{name}` is not in schema"),
        })?;
        fields.push(field.clone());
    }
    Schema::new(format!("{}__proj", schema.id.as_str()), fields).map_err(|e| Error::InvalidState {
        message: format!("invalid projected schema: {e}"),
    })
}

fn columns_to_batch(
    schema: &Schema,
    row_count: u32,
    columns: Vec<Vec<FieldValue>>,
) -> Result<RecordBatch> {
    let names = Arc::new(schema.fields.iter().map(|f| f.name.clone()).collect());
    let mut columns: Vec<_> = columns.into_iter().map(Vec::into_iter).collect();
    let records = (0..row_count)
        .map(|_| {
            let slots = columns
                .iter_mut()
                .map(|c| c.next().expect("checked column length"))
                .collect();
            Record::from_ordered(Arc::clone(&names), slots)
        })
        .collect();
    let batch = RecordBatch {
        schema: schema.clone(),
        records,
    };
    Ok(batch)
}

/// Probes a potential block frame at `bytes[0..]`.
///
/// Returns the total frame length (including CRC) when the length fields and
/// CRC32 validate. Used by crash recovery to find the last intact block.
pub fn probe_block_frame(bytes: &[u8]) -> Option<usize> {
    if bytes.len() < 1 + 1 + 4 + 4 + 4 {
        return None;
    }
    if bytes[0] != BLOCK_FRAME_VERSION {
        return None;
    }
    let compressed_len = u32::from_le_bytes(bytes[6..10].try_into().ok()?) as usize;
    let total = 10 + compressed_len + 4;
    if bytes.len() < total {
        return None;
    }
    let frame = &bytes[..total];
    let (framed, crc_bytes) = frame.split_at(frame.len() - 4);
    let expected = u32::from_le_bytes(crc_bytes.try_into().ok()?);
    let actual = crc32(framed);
    if expected != actual {
        return None;
    }
    if framed.len() != 10 + compressed_len {
        return None;
    }
    Some(total)
}
