//! Column encode/decode using composable [`EncodingPlan`]s.

use strata_core::{FieldValue, IntegerWidth, LogicalType};

use crate::codecs::{decode_step, decode_step_bytes, encode_step};
use crate::error::{Error, Result};
use crate::nulls::{expand_nulls, read_null_bitmap, write_null_bitmap};
use crate::plan::{CodecMetadata, EncodingPlan, EncodingStep, StepMetadata};
use crate::stream::Stream;

/// Current on-disk column payload format version.
pub const COLUMN_FORMAT_VERSION: u8 = 1;

/// Result of encoding one column.
#[derive(Debug, Clone)]
pub struct EncodedColumn {
    /// Full column payload bytes (version + nulls + plan + metas + body).
    pub bytes: Vec<u8>,
    /// Codec metadata (also embedded in `bytes`).
    pub metadata: CodecMetadata,
    /// Logical size estimate of present values.
    pub logical_bytes: u64,
}

impl EncodedColumn {
    /// Encoded payload size in bytes.
    pub fn encoded_size(&self) -> usize {
        self.bytes.len()
    }
}

/// Encodes a column of [`FieldValue`]s with `plan`.
pub fn encode_column(
    logical_type: &LogicalType,
    values: &[FieldValue],
    plan: &EncodingPlan,
) -> Result<EncodedColumn> {
    encode_column_refs(logical_type, values.iter(), plan)
}

/// Encode borrowed values directly from schema-ordered rows, without cloning cells.
pub fn encode_column_refs<'a>(
    logical_type: &LogicalType,
    values: impl ExactSizeIterator<Item = &'a FieldValue>,
    plan: &EncodingPlan,
) -> Result<EncodedColumn> {
    crate::limits::check_count(values.len())?;
    plan.validate()?;
    let mut bitmap = vec![0u8; values.len().div_ceil(8)];
    let mut present = Vec::with_capacity(values.len());
    let mut logical_bytes = 0;
    for (i, value) in values.enumerate() {
        if !value.is_null() {
            bitmap[i / 8] |= 1 << (i % 8);
            logical_bytes += strata_core::logical_value_bytes(logical_type, value);
            present.push(value);
        }
    }
    let mut stream = values_to_stream(logical_type, &present, plan)?;

    let mut step_metas = Vec::new();
    for step in &plan.steps {
        let out = encode_step(*step, stream)?;
        if let Some(meta) = out.meta {
            step_metas.push(meta);
        }
        stream = out.stream;
    }
    let body = stream.into_encoded().map_err(|_| Error::InvalidPlan {
        message: "encoding plan did not produce a final encoded byte stream".into(),
    })?;

    let metadata = CodecMetadata {
        plan: plan.clone(),
        steps: step_metas,
    };

    let mut bytes = Vec::new();
    bytes.push(COLUMN_FORMAT_VERSION);
    write_null_bitmap(&mut bytes, &bitmap)?;
    let plan_bytes = plan.to_bytes();
    write_u32(&mut bytes, plan_bytes.len() as u32)?;
    bytes.extend_from_slice(&plan_bytes);
    write_u32(&mut bytes, metadata.steps.len() as u32)?;
    for step in &metadata.steps {
        bytes.push(step.step.tag());
        write_u32(&mut bytes, step.blob.len() as u32)?;
        bytes.extend_from_slice(&step.blob);
    }
    write_u32(&mut bytes, body.len() as u32)?;
    bytes.extend_from_slice(&body);

    Ok(EncodedColumn {
        bytes,
        metadata,
        logical_bytes,
    })
}

/// Decodes a column payload produced by [`encode_column`].
pub fn decode_column(
    logical_type: &LogicalType,
    row_count: u32,
    bytes: &[u8],
) -> Result<Vec<FieldValue>> {
    if bytes.is_empty() {
        return Err(Error::Decode {
            message: "empty column payload".into(),
        });
    }
    let version = bytes[0];
    if version != COLUMN_FORMAT_VERSION {
        return Err(Error::Decode {
            message: format!("unsupported column format version {version}"),
        });
    }
    let (bitmap, consumed) = read_null_bitmap(&bytes[1..], row_count as usize)?;
    let mut pos = 1 + consumed;

    let plan_len = read_u32_at(bytes, &mut pos)? as usize;
    if pos + plan_len > bytes.len() {
        return Err(Error::Decode {
            message: "truncated encoding plan".into(),
        });
    }
    let plan = EncodingPlan::from_bytes(&bytes[pos..pos + plan_len])?;
    pos += plan_len;

    let n_meta = read_u32_at(bytes, &mut pos)? as usize;
    if n_meta > plan.steps.len() || n_meta > bytes.len().saturating_sub(pos) / 5 {
        return Err(Error::Decode {
            message: "invalid metadata count".into(),
        });
    }
    let mut metas = Vec::with_capacity(n_meta);
    for _ in 0..n_meta {
        if pos >= bytes.len() {
            return Err(Error::Decode {
                message: "truncated step metadata".into(),
            });
        }
        let tag = bytes[pos];
        pos += 1;
        let step = EncodingStep::from_tag(tag).ok_or_else(|| Error::Decode {
            message: format!("unknown step tag {tag} in metadata"),
        })?;
        let blob_len = read_u32_at(bytes, &mut pos)? as usize;
        if pos + blob_len > bytes.len() {
            return Err(Error::Decode {
                message: "truncated step metadata blob".into(),
            });
        }
        let blob = bytes[pos..pos + blob_len].to_vec();
        pos += blob_len;
        metas.push(StepMetadata { step, blob });
    }

    let body_len = read_u32_at(bytes, &mut pos)? as usize;
    if pos + body_len != bytes.len() {
        return Err(Error::Decode {
            message: format!(
                "column body length mismatch: pos={pos} body_len={body_len} total={}",
                bytes.len()
            ),
        });
    }
    let body = &bytes[pos..pos + body_len];

    let mut stream = None;
    let mut meta_idx = metas.len();
    for step in plan.steps.iter().rev() {
        let meta = if step_needs_meta(*step) {
            if meta_idx == 0 {
                return Err(Error::Decode {
                    message: format!("missing metadata for step {}", step.as_str()),
                });
            }
            meta_idx -= 1;
            let m = &metas[meta_idx];
            if m.step != *step {
                return Err(Error::Decode {
                    message: format!(
                        "step metadata mismatch: expected {}, found {}",
                        step.as_str(),
                        m.step.as_str()
                    ),
                });
            }
            Some(m)
        } else {
            None
        };
        stream = Some(match stream {
            None => decode_step_bytes(*step, body, meta)?,
            Some(current) => decode_step(*step, current, meta)?,
        });
    }

    if meta_idx != 0 {
        return Err(Error::Decode {
            message: "unused step metadata".into(),
        });
    }
    let present = stream_to_values(
        logical_type,
        stream.ok_or_else(|| Error::Decode {
            message: "empty decode plan".into(),
        })?,
    )?;
    for value in &present {
        match (logical_type, value) {
            (LogicalType::UnsignedInteger(t), FieldValue::UnsignedInteger(n)) => {
                check_u_width(*n, t.width)?
            }
            (LogicalType::SignedInteger(t), FieldValue::SignedInteger(n)) => {
                check_i_width(*n, t.width)?
            }
            (LogicalType::FixedDecimal(t), FieldValue::FixedDecimal(n)) => {
                check_i_width(*n, t.width)?
            }
            (LogicalType::Enum(t), FieldValue::Enum(s)) if !t.variants.contains(s) => {
                return Err(Error::Decode {
                    message: "enum outside schema domain".into(),
                })
            }
            (LogicalType::CategoricalString(t), FieldValue::CategoricalString(s))
                if t.max_bytes.is_some_and(|max| s.len() > max as usize) =>
            {
                return Err(Error::Decode {
                    message: "string exceeds schema bound".into(),
                })
            }
            (LogicalType::Binary(t), FieldValue::Binary(s))
                if t.max_bytes.is_some_and(|max| s.len() > max as usize) =>
            {
                return Err(Error::Decode {
                    message: "binary exceeds schema bound".into(),
                })
            }
            _ => {}
        }
    }
    let n = row_count as usize;
    let full = bitmap[..n / 8].iter().all(|&b| b == 255)
        && (n % 8 == 0 || bitmap[n / 8] & ((1 << (n % 8)) - 1) == (1 << (n % 8)) - 1);
    if full {
        if present.len() != n {
            return Err(Error::Decode {
                message: "all-present value count mismatch".into(),
            });
        }
        return Ok(present);
    }
    expand_nulls(n, &bitmap, &present, FieldValue::Null)
}

fn step_needs_meta(step: EncodingStep) -> bool {
    matches!(
        step,
        EncodingStep::BitPacking
            | EncodingStep::FrameOfReference
            | EncodingStep::DictionaryEncoding
            | EncodingStep::BooleanBitpack
            | EncodingStep::EnumBitpack
    )
}

fn plan_needs_signed(plan: &EncodingPlan) -> bool {
    plan.steps.iter().any(|s| {
        matches!(
            s,
            EncodingStep::Delta
                | EncodingStep::DeltaOfDelta
                | EncodingStep::ZigZag
                | EncodingStep::FrameOfReference
        )
    })
}

fn values_to_stream(
    logical_type: &LogicalType,
    present: &[&FieldValue],
    plan: &EncodingPlan,
) -> Result<Stream> {
    match logical_type {
        LogicalType::Boolean(_) => {
            let mut v = Vec::with_capacity(present.len());
            for x in present {
                match x {
                    FieldValue::Boolean(b) => v.push(*b),
                    other => {
                        return Err(Error::InvalidInput {
                            message: format!("expected boolean, got {}", other.kind_name()),
                        })
                    }
                }
            }
            Ok(Stream::Bool(v))
        }
        LogicalType::UnsignedInteger(t) => {
            let mut u = Vec::with_capacity(present.len());
            for x in present {
                match x {
                    FieldValue::UnsignedInteger(n) => {
                        check_u_width(*n, t.width)?;
                        u.push(*n);
                    }
                    other => {
                        return Err(Error::InvalidInput {
                            message: format!("expected unsigned, got {}", other.kind_name()),
                        })
                    }
                }
            }
            if plan_needs_signed(plan) {
                Ok(Stream::I64(u.into_iter().map(|n| n as i64).collect()))
            } else {
                Ok(Stream::U64(u))
            }
        }
        LogicalType::SignedInteger(t) => {
            let mut v = Vec::with_capacity(present.len());
            for x in present {
                match x {
                    FieldValue::SignedInteger(n) => {
                        check_i_width(*n, t.width)?;
                        v.push(*n);
                    }
                    other => {
                        return Err(Error::InvalidInput {
                            message: format!("expected signed integer, got {}", other.kind_name()),
                        })
                    }
                }
            }
            Ok(Stream::I64(v))
        }
        LogicalType::Timestamp(_) => {
            let mut v = Vec::with_capacity(present.len());
            for x in present {
                match x {
                    FieldValue::Timestamp(n) => v.push(*n),
                    other => {
                        return Err(Error::InvalidInput {
                            message: format!("expected timestamp, got {}", other.kind_name()),
                        })
                    }
                }
            }
            Ok(Stream::I64(v))
        }
        LogicalType::FixedDecimal(t) => {
            let mut v = Vec::with_capacity(present.len());
            for x in present {
                match x {
                    FieldValue::FixedDecimal(n) => {
                        check_i_width(*n, t.width)?;
                        v.push(*n);
                    }
                    other => {
                        return Err(Error::InvalidInput {
                            message: format!("expected fixed decimal, got {}", other.kind_name()),
                        })
                    }
                }
            }
            Ok(Stream::I64(v))
        }
        LogicalType::CategoricalString(_) | LogicalType::Enum(_) => {
            let mut v = Vec::with_capacity(present.len());
            for x in present {
                match x {
                    FieldValue::CategoricalString(s) | FieldValue::Enum(s) => {
                        v.push(s.as_bytes().to_vec())
                    }
                    other => {
                        return Err(Error::InvalidInput {
                            message: format!("expected string/enum, got {}", other.kind_name()),
                        })
                    }
                }
            }
            Ok(Stream::Bytes(v))
        }
        LogicalType::Binary(_) => {
            let mut v = Vec::with_capacity(present.len());
            for x in present {
                match x {
                    FieldValue::Binary(b) => v.push(b.clone()),
                    other => {
                        return Err(Error::InvalidInput {
                            message: format!("expected binary, got {}", other.kind_name()),
                        })
                    }
                }
            }
            Ok(Stream::Bytes(v))
        }
    }
}

fn stream_to_values(logical_type: &LogicalType, stream: Stream) -> Result<Vec<FieldValue>> {
    match logical_type {
        LogicalType::Boolean(_) => Ok(stream
            .into_bool()?
            .into_iter()
            .map(FieldValue::Boolean)
            .collect()),
        LogicalType::UnsignedInteger(_) => {
            let values = match stream {
                Stream::U64(v) => v,
                Stream::I64(v) => v.into_iter().map(|n| n as u64).collect(),
                other => {
                    return Err(Error::InvalidPlan {
                        message: format!("expected integer stream for unsigned, found {other:?}"),
                    })
                }
            };
            Ok(values
                .into_iter()
                .map(FieldValue::UnsignedInteger)
                .collect())
        }
        LogicalType::SignedInteger(_) => Ok(match stream {
            Stream::I64(v) => v.into_iter().map(FieldValue::SignedInteger).collect(),
            Stream::U64(v) => v
                .into_iter()
                .map(|n| FieldValue::SignedInteger(n as i64))
                .collect(),
            other => {
                return Err(Error::InvalidPlan {
                    message: format!("expected integer stream for signed, found {other:?}"),
                })
            }
        }),
        LogicalType::Timestamp(_) => Ok(match stream {
            Stream::I64(v) => v.into_iter().map(FieldValue::Timestamp).collect(),
            Stream::U64(v) => v
                .into_iter()
                .map(|n| FieldValue::Timestamp(n as i64))
                .collect(),
            other => {
                return Err(Error::InvalidPlan {
                    message: format!("expected integer stream for timestamp, found {other:?}"),
                })
            }
        }),
        LogicalType::FixedDecimal(_) => Ok(match stream {
            Stream::I64(v) => v.into_iter().map(FieldValue::FixedDecimal).collect(),
            Stream::U64(v) => v
                .into_iter()
                .map(|n| FieldValue::FixedDecimal(n as i64))
                .collect(),
            other => {
                return Err(Error::InvalidPlan {
                    message: format!("expected integer stream for decimal, found {other:?}"),
                })
            }
        }),
        LogicalType::CategoricalString(_) => {
            let mut out = Vec::new();
            for b in stream.into_bytes_list()? {
                let s = String::from_utf8(b).map_err(|e| Error::Decode {
                    message: format!("invalid UTF-8: {e}"),
                })?;
                out.push(FieldValue::CategoricalString(s));
            }
            Ok(out)
        }
        LogicalType::Enum(_) => {
            let mut out = Vec::new();
            for b in stream.into_bytes_list()? {
                let s = String::from_utf8(b).map_err(|e| Error::Decode {
                    message: format!("invalid UTF-8: {e}"),
                })?;
                out.push(FieldValue::Enum(s));
            }
            Ok(out)
        }
        LogicalType::Binary(_) => Ok(stream
            .into_bytes_list()?
            .into_iter()
            .map(FieldValue::Binary)
            .collect()),
    }
}

fn check_u_width(v: u64, width: IntegerWidth) -> Result<()> {
    let max = match width {
        IntegerWidth::Bits8 => u8::MAX as u64,
        IntegerWidth::Bits16 => u16::MAX as u64,
        IntegerWidth::Bits32 => u32::MAX as u64,
        IntegerWidth::Bits64 => u64::MAX,
    };
    if v > max {
        Err(Error::InvalidInput {
            message: format!("unsigned value {v} exceeds {width:?}"),
        })
    } else {
        Ok(())
    }
}

fn check_i_width(v: i64, width: IntegerWidth) -> Result<()> {
    let (min, max) = match width {
        IntegerWidth::Bits8 => (i8::MIN as i64, i8::MAX as i64),
        IntegerWidth::Bits16 => (i16::MIN as i64, i16::MAX as i64),
        IntegerWidth::Bits32 => (i32::MIN as i64, i32::MAX as i64),
        IntegerWidth::Bits64 => (i64::MIN, i64::MAX),
    };
    if v < min || v > max {
        Err(Error::InvalidInput {
            message: format!("signed value {v} exceeds {width:?}"),
        })
    } else {
        Ok(())
    }
}

fn write_u32(out: &mut Vec<u8>, v: u32) -> Result<()> {
    out.extend_from_slice(&v.to_le_bytes());
    Ok(())
}

fn read_u32_at(bytes: &[u8], pos: &mut usize) -> Result<u32> {
    if *pos + 4 > bytes.len() {
        return Err(Error::Decode {
            message: "truncated u32".into(),
        });
    }
    let v = u32::from_le_bytes(bytes[*pos..*pos + 4].try_into().unwrap());
    *pos += 4;
    Ok(v)
}
