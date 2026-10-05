//! Null bitmap helpers shared by column encode/decode.

use strata_core::FieldValue;

use crate::error::{Error, Result};

/// Builds a null bitmap (bit i set ⇒ present) and returns present value indices.
pub fn split_nulls(values: &[FieldValue]) -> (Vec<u8>, Vec<usize>) {
    let bitmap_len = values.len().div_ceil(8);
    let mut bitmap = vec![0u8; bitmap_len];
    let mut present = Vec::new();
    for (i, value) in values.iter().enumerate() {
        if !value.is_null() {
            bitmap[i / 8] |= 1 << (i % 8);
            present.push(i);
        }
    }
    (bitmap, present)
}

/// Writes `bitmap_len` + bitmap.
pub fn write_null_bitmap(out: &mut Vec<u8>, bitmap: &[u8]) -> Result<()> {
    let len = u32::try_from(bitmap.len()).map_err(|_| Error::InvalidInput {
        message: "null bitmap too large".into(),
    })?;
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(bitmap);
    Ok(())
}

/// Reads null bitmap; returns (bitmap, bytes_consumed).
pub fn read_null_bitmap(bytes: &[u8], row_count: usize) -> Result<(Vec<u8>, usize)> {
    if bytes.len() < 4 {
        return Err(Error::Decode {
            message: "truncated null bitmap header".into(),
        });
    }
    let len = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let expected = row_count.div_ceil(8);
    if len != expected {
        return Err(Error::Decode {
            message: format!(
                "null bitmap length {len} != expected {expected} for {row_count} rows"
            ),
        });
    }
    if bytes.len() < 4 + len {
        return Err(Error::Decode {
            message: "truncated null bitmap".into(),
        });
    }
    Ok((bytes[4..4 + len].to_vec(), 4 + len))
}

/// Expands present values back into a full column with nulls.
pub fn expand_nulls<T: Clone>(
    row_count: usize,
    bitmap: &[u8],
    present_values: &[T],
    null_value: T,
) -> Result<Vec<T>> {
    crate::limits::check_count(row_count)?;
    if bitmap.len() != row_count.div_ceil(8) {
        return Err(Error::Decode {
            message: "invalid null bitmap length".into(),
        });
    }
    let mut out = Vec::with_capacity(row_count);
    let mut pi = 0usize;
    for i in 0..row_count {
        let present = bitmap[i / 8] & (1 << (i % 8)) != 0;
        if present {
            if pi >= present_values.len() {
                return Err(Error::Decode {
                    message: "null bitmap has more present bits than decoded values".into(),
                });
            }
            out.push(present_values[pi].clone());
            pi += 1;
        } else {
            out.push(null_value.clone());
        }
    }
    if pi != present_values.len() {
        return Err(Error::Decode {
            message: format!(
                "decoded {} present values but bitmap marks {pi} present",
                present_values.len()
            ),
        });
    }
    Ok(out)
}
