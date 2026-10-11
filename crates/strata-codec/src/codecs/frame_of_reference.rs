//! Frame-of-reference: subtract min, emit unsigned offsets.

use crate::codecs::StepOutput;
use crate::error::{Error, Result};
use crate::plan::{EncodingStep, StepMetadata};
use crate::stream::Stream;

pub fn encode(input: Stream) -> Result<StepOutput> {
    let values = input.into_i64()?;
    if values.is_empty() {
        return Ok(StepOutput {
            stream: Stream::U64(Vec::new()),
            meta: Some(StepMetadata {
                step: EncodingStep::FrameOfReference,
                blob: 0i64.to_le_bytes().to_vec(),
            }),
        });
    }
    let frame = *values.iter().min().unwrap();
    let mut offsets = Vec::with_capacity(values.len());
    for v in values {
        let off = v.wrapping_sub(frame) as u64;
        offsets.push(off);
    }
    Ok(StepOutput {
        stream: Stream::U64(offsets),
        meta: Some(StepMetadata {
            step: EncodingStep::FrameOfReference,
            blob: frame.to_le_bytes().to_vec(),
        }),
    })
}

pub fn decode(input: Stream, meta: Option<&StepMetadata>) -> Result<Stream> {
    let meta = meta.ok_or_else(|| Error::Decode {
        message: "frame-of-reference requires metadata".into(),
    })?;
    if meta.blob.len() != 8 {
        return Err(Error::Decode {
            message: "invalid FOR frame metadata".into(),
        });
    }
    let frame = i64::from_le_bytes(meta.blob.as_slice().try_into().unwrap());
    let mut offsets = input.into_u64()?;
    for offset in &mut offsets {
        *offset = frame.wrapping_add(*offset as i64) as u64;
    }
    Ok(Stream::I64(crate::stream::reinterpret_u64_bits_as_i64(
        offsets,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = vec![1000i64, 1005, 1003, 1010];
        let enc = encode(Stream::I64(v.clone())).unwrap();
        let dec = decode(enc.stream, enc.meta.as_ref())
            .unwrap()
            .into_i64()
            .unwrap();
        assert_eq!(dec, v);
    }
}
