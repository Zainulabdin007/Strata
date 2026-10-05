//! Delta encoding for i64 streams.

use crate::codecs::StepOutput;
use crate::error::Result;
use crate::plan::StepMetadata;
use crate::stream::Stream;

pub fn encode(input: Stream) -> Result<StepOutput> {
    let values = input.into_i64()?;
    if values.is_empty() {
        return Ok(StepOutput {
            stream: Stream::I64(Vec::new()),
            meta: None,
        });
    }
    let mut out = Vec::with_capacity(values.len());
    out.push(values[0]);
    for i in 1..values.len() {
        out.push(values[i].wrapping_sub(values[i - 1]));
    }
    Ok(StepOutput {
        stream: Stream::I64(out),
        meta: None,
    })
}

pub fn decode(input: Stream, _meta: Option<&StepMetadata>) -> Result<Stream> {
    let mut values = input.into_i64()?;
    for i in 1..values.len() {
        values[i] = values[i - 1].wrapping_add(values[i]);
    }
    Ok(Stream::I64(values))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = vec![10i64, 12, 15, 15, 20];
        let enc = encode(Stream::I64(v.clone())).unwrap();
        let dec = decode(enc.stream, None).unwrap().into_i64().unwrap();
        assert_eq!(dec, v);
    }
}
