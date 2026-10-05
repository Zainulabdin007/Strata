//! Dense bit packing for unsigned integer streams.

use crate::codecs::StepOutput;
use crate::error::{Error, Result};
use crate::plan::{EncodingStep, StepMetadata};
use crate::stream::Stream;

pub fn encode(input: Stream) -> Result<StepOutput> {
    let values = input.into_u64()?;
    let bit_width = bit_width_for(&values);
    let body = pack(&values, bit_width)?;
    let mut blob = Vec::new();
    blob.push(bit_width);
    write_u32(&mut blob, values.len() as u32);

    Ok(StepOutput {
        stream: Stream::Encoded(body),
        meta: Some(StepMetadata {
            step: EncodingStep::BitPacking,
            blob,
        }),
    })
}

pub fn decode(input: Stream, meta: Option<&StepMetadata>) -> Result<Stream> {
    let bytes = input.into_encoded()?;
    decode_bytes(&bytes, meta)
}

pub(super) fn decode_bytes(bytes: &[u8], meta: Option<&StepMetadata>) -> Result<Stream> {
    let meta = meta.ok_or_else(|| Error::Decode {
        message: "bit packing requires metadata".into(),
    })?;
    if meta.blob.len() != 5 {
        return Err(Error::Decode {
            message: "invalid bit packing metadata".into(),
        });
    }
    let bit_width = meta.blob[0];
    let count = u32::from_le_bytes(meta.blob[1..5].try_into().unwrap()) as usize;
    let values = unpack(&bytes, count, bit_width)?;
    Ok(Stream::U64(values))
}

pub fn bit_width_for(values: &[u64]) -> u8 {
    if values.is_empty() {
        return 0;
    }
    let max = values.iter().copied().max().unwrap_or(0);
    if max == 0 {
        1
    } else {
        (64 - max.leading_zeros()) as u8
    }
}

pub fn pack(values: &[u64], bit_width: u8) -> Result<Vec<u8>> {
    if bit_width == 0 {
        if !values.is_empty() {
            return Err(Error::InvalidInput {
                message: "zero width for nonempty input".into(),
            });
        }
        return Ok(Vec::new());
    }
    if bit_width > 64 {
        return Err(Error::InvalidInput {
            message: format!("bit_width {bit_width} > 64"),
        });
    }
    let mask = if bit_width == 64 {
        u64::MAX
    } else {
        (1u64 << bit_width) - 1
    };
    let total_bits = values.len() * bit_width as usize;
    let mut out = vec![0u8; total_bits.div_ceil(8)];
    for (i, &v) in values.iter().enumerate() {
        if v > mask {
            return Err(Error::InvalidInput {
                message: format!("value {v} does not fit in {bit_width} bits"),
            });
        }
        let bit = i * bit_width as usize;
        let offset = bit / 8;
        let shift = bit % 8;
        let word = (v << shift).to_le_bytes();
        let n = (out.len() - offset).min(8);
        for j in 0..n {
            out[offset + j] |= word[j];
        }
        if shift + bit_width as usize > 64 {
            out[offset + 8] |= (v >> (64 - shift)) as u8;
        }
    }
    Ok(out)
}

pub fn unpack(bytes: &[u8], count: usize, bit_width: u8) -> Result<Vec<u64>> {
    crate::limits::check_count(count)?;
    if bit_width > 64 {
        return Err(Error::Decode {
            message: "bit width exceeds 64".into(),
        });
    }
    if bit_width == 0 {
        if count == 0 && bytes.is_empty() {
            return Ok(Vec::new());
        }
        return Err(Error::Decode {
            message: "bit_width 0 with non-empty count".into(),
        });
    }
    let need = (count * bit_width as usize).div_ceil(8);
    if bytes.len() != need {
        return Err(Error::Decode {
            message: format!("bitpack truncated: need {need}, have {}", bytes.len()),
        });
    }
    let mut out = Vec::with_capacity(count);
    if bit_width == 64 {
        out.extend(
            bytes
                .chunks_exact(8)
                .map(|v| u64::from_le_bytes(v.try_into().unwrap())),
        );
    } else {
        let mask = (1u64 << bit_width) - 1;
        for i in 0..count {
            let bit = i * bit_width as usize;
            let offset = bit / 8;
            let shift = bit % 8;
            let mut word = [0u8; 8];
            let n = (bytes.len() - offset).min(8);
            word[..n].copy_from_slice(&bytes[offset..offset + n]);
            let mut value = u64::from_le_bytes(word) >> shift;
            if shift + bit_width as usize > 64 {
                value |= u64::from(bytes[offset + 8]) << (64 - shift);
            }
            out.push(value & mask);
        }
    }
    Ok(out)
}

#[cfg(test)]
fn write_bits(out: &mut [u8], bit_pos: usize, width: u8, mut value: u64) {
    let mut remaining = width as usize;
    let mut pos = bit_pos;
    while remaining > 0 {
        let byte_index = pos / 8;
        let bit_offset = pos % 8;
        let space = 8 - bit_offset;
        let take = remaining.min(space);
        let bits = (value & ((1u64 << take) - 1)) as u8;
        out[byte_index] |= bits << bit_offset;
        value >>= take;
        remaining -= take;
        pos += take;
    }
}

#[cfg(test)]
fn read_bits(bytes: &[u8], bit_pos: usize, width: u8) -> u64 {
    let mut remaining = width as usize;
    let mut pos = bit_pos;
    let mut value = 0u64;
    let mut shift = 0u32;
    while remaining > 0 {
        let byte_index = pos / 8;
        let bit_offset = pos % 8;
        let space = 8 - bit_offset;
        let take = remaining.min(space);
        let mask = ((1u16 << take) - 1) as u8;
        let bits = (bytes[byte_index] >> bit_offset) & mask;
        value |= u64::from(bits) << shift;
        remaining -= take;
        pos += take;
        shift += take as u32;
    }
    value
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_unpack_matches_scalar_all_widths_and_tails() {
        for width in 1..=64 {
            for count in [0, 1, 2, 7, 8, 9, 31, 127] {
                let mask = u64::MAX >> (64 - width);
                let values: Vec<_> = (0..count)
                    .map(|i| (i as u64).wrapping_mul(0x9e3779b97f4a7c15) & mask)
                    .collect();
                let bytes = pack(&values, width).unwrap();
                let mut oracle = vec![0u8; (count * width as usize).div_ceil(8)];
                for (i, &v) in values.iter().enumerate() {
                    write_bits(&mut oracle, i * width as usize, width, v);
                }
                assert_eq!(bytes, oracle);
                let expected: Vec<_> = (0..count)
                    .map(|i| read_bits(&bytes, i * width as usize, width))
                    .collect();
                assert_eq!(unpack(&bytes, count, width).unwrap(), expected);
                assert_eq!(expected, values);
                if !bytes.is_empty() {
                    assert!(unpack(&bytes[..bytes.len() - 1], count, width).is_err());
                }
            }
        }
    }

    #[test]
    fn roundtrip() {
        let v = vec![0u64, 1, 2, 7, 15, 31];
        let enc = encode(Stream::U64(v.clone())).unwrap();
        let dec = decode(enc.stream, enc.meta.as_ref())
            .unwrap()
            .into_u64()
            .unwrap();
        assert_eq!(dec, v);
    }
}
