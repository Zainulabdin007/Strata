//! Run-length encoding for unsigned integer streams.

use crate::codecs::varint::{read_varint, write_varint};
use crate::codecs::StepOutput;
use crate::error::{Error, Result};
use crate::plan::StepMetadata;
use crate::stream::Stream;

pub fn encode(input: Stream) -> Result<StepOutput> {
    let values = input.into_u64()?;
    let mut out = Vec::new();
    if values.is_empty() {
        write_u32(&mut out, 0);
        return Ok(StepOutput {
            stream: Stream::Encoded(out),
            meta: None,
        });
    }

    let mut runs: Vec<(u64, u32)> = Vec::new();
    let mut cur = values[0];
    let mut count = 1u32;
    for &v in &values[1..] {
        if v == cur {
            if count == u32::MAX {
                runs.push((cur, count));
                count = 1;
            } else {
                count += 1;
            }
        } else {
            runs.push((cur, count));
            cur = v;
            count = 1;
        }
    }
    runs.push((cur, count));

    write_u32(&mut out, runs.len() as u32);
    for (value, cnt) in runs {
        write_varint(&mut out, value);
        write_varint(&mut out, u64::from(cnt));
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
            message: "truncated RLE header".into(),
        });
    }
    let n_runs = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let mut pos = 4usize;
    let mut values = Vec::new();
    for _ in 0..n_runs {
        let (value, n1) = read_varint(&bytes[pos..])?;
        pos += n1;
        let (count, n2) = read_varint(&bytes[pos..])?;
        pos += n2;
        let count = u32::try_from(count).map_err(|_| Error::Decode {
            message: "RLE count exceeds u32".into(),
        })?;
        if count == 0 {
            return Err(Error::Decode {
                message: "zero RLE run".into(),
            });
        }
        crate::limits::check_count(values.len().checked_add(count as usize).ok_or_else(|| {
            Error::Decode {
                message: "RLE overflow".into(),
            }
        })?)?;
        for _ in 0..count {
            values.push(value);
        }
    }
    if pos != bytes.len() {
        return Err(Error::Decode {
            message: "trailing bytes in RLE payload".into(),
        });
    }
    Ok(Stream::U64(values))
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = vec![1u64, 1, 1, 2, 2, 3, 3, 3, 3];
        let enc = encode(Stream::U64(v.clone())).unwrap();
        let dec = decode(enc.stream, None).unwrap().into_u64().unwrap();
        assert_eq!(dec, v);
    }
}
