//! Unsigned LEB128 varint encoding.

use crate::codecs::StepOutput;
use crate::error::{Error, Result};
use crate::plan::StepMetadata;
use crate::stream::Stream;

pub fn encode(input: Stream) -> Result<StepOutput> {
    let values = input.into_u64()?;
    let mut out = Vec::new();
    write_u32(&mut out, values.len() as u32);
    for v in values {
        write_varint(&mut out, v);
    }
    Ok(StepOutput {
        stream: Stream::Encoded(out),
        meta: None,
    })
}

pub fn decode(input: Stream, _meta: Option<&StepMetadata>) -> Result<Stream> {
    let bytes = input.into_encoded()?;
    decode_bytes(&bytes, _meta)
}

pub(super) fn decode_bytes(bytes: &[u8], _meta: Option<&StepMetadata>) -> Result<Stream> {
    if bytes.len() < 4 {
        return Err(Error::Decode {
            message: "truncated varint header".into(),
        });
    }
    let len = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    crate::limits::check_count(len)?;
    if len > bytes.len() - 4 {
        return Err(Error::Decode {
            message: "varint count exceeds payload".into(),
        });
    }
    let mut pos = 4usize;
    let mut values = Vec::with_capacity(len);
    for _ in 0..len {
        let (v, n) = read_varint(&bytes[pos..])?;
        values.push(v);
        pos += n;
    }
    if pos != bytes.len() {
        return Err(Error::Decode {
            message: "trailing bytes in varint payload".into(),
        });
    }
    Ok(Stream::U64(values))
}

pub fn write_varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let mut b = (v & 0x7f) as u8;
        v >>= 7;
        if v != 0 {
            b |= 0x80;
            out.push(b);
        } else {
            out.push(b);
            break;
        }
    }
}

pub fn read_varint(bytes: &[u8]) -> Result<(u64, usize)> {
    // Most deltas and small codes are a single byte. Keep that off the shift loop.
    if let Some(&b) = bytes.first() {
        if b < 0x80 {
            return Ok((u64::from(b), 1));
        }
    }
    let mut result = 0u64;
    let mut shift = 0u32;
    for (i, b) in bytes.iter().enumerate() {
        if shift >= 64 || (shift == 63 && b & 0x7f > 1) {
            return Err(Error::Decode {
                message: "varint overflow".into(),
            });
        }
        result |= u64::from(b & 0x7f) << shift;
        if b & 0x80 == 0 {
            return Ok((result, i + 1));
        }
        shift += 7;
    }
    Err(Error::Decode {
        message: "truncated varint".into(),
    })
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = vec![0u64, 1, 127, 128, 255, 300, u64::MAX];
        let enc = encode(Stream::U64(v.clone())).unwrap();
        let dec = decode(enc.stream, None).unwrap().into_u64().unwrap();
        assert_eq!(dec, v);
    }
}
