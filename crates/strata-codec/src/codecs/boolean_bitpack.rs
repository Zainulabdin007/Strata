//! 1-bit boolean packing.

use crate::codecs::StepOutput;
use crate::error::{Error, Result};
use crate::plan::{EncodingStep, StepMetadata};
use crate::stream::Stream;

pub fn encode(input: Stream) -> Result<StepOutput> {
    let values = input.into_bool()?;
    let mut bits = vec![0u8; values.len().div_ceil(8)];
    for (i, v) in values.iter().enumerate() {
        if *v {
            bits[i / 8] |= 1 << (i % 8);
        }
    }
    let mut blob = Vec::new();
    write_u32(&mut blob, values.len() as u32);
    Ok(StepOutput {
        stream: Stream::Encoded(bits),
        meta: Some(StepMetadata {
            step: EncodingStep::BooleanBitpack,
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
        message: "boolean bitpack requires metadata".into(),
    })?;
    if meta.blob.len() != 4 {
        return Err(Error::Decode {
            message: "invalid boolean bitpack metadata".into(),
        });
    }
    let count = u32::from_le_bytes(meta.blob.as_slice().try_into().unwrap()) as usize;
    crate::limits::check_count(count)?;
    let need = count.div_ceil(8);
    if bytes.len() != need {
        return Err(Error::Decode {
            message: "truncated boolean bitpack payload".into(),
        });
    }
    let mut values = Vec::with_capacity(count);
    for i in 0..count {
        values.push(bytes[i / 8] & (1 << (i % 8)) != 0);
    }
    Ok(Stream::Bool(values))
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = vec![true, false, true, true, false, false, false, true, true];
        let enc = encode(Stream::Bool(v.clone())).unwrap();
        let dec = decode(enc.stream, enc.meta.as_ref())
            .unwrap()
            .into_bool()
            .unwrap();
        assert_eq!(dec, v);
    }
}
