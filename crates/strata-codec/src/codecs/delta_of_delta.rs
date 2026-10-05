//! Second-order delta encoding for i64 streams.

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
    if values.len() == 1 {
        return Ok(StepOutput {
            stream: Stream::I64(values),
            meta: None,
        });
    }
    let mut out = Vec::with_capacity(values.len());
    out.push(values[0]);
    let mut prev_delta = values[1].wrapping_sub(values[0]);
    out.push(prev_delta);
    for i in 2..values.len() {
        let delta = values[i].wrapping_sub(values[i - 1]);
        out.push(delta.wrapping_sub(prev_delta));
        prev_delta = delta;
    }
    Ok(StepOutput {
        stream: Stream::I64(out),
        meta: None,
    })
}

pub fn decode(input: Stream, _meta: Option<&StepMetadata>) -> Result<Stream> {
    let mut values = input.into_i64()?;
    if values.len() > 1 {
        let mut delta = values[1];
        values[1] = values[0].wrapping_add(delta);
        for i in 2..values.len() {
            delta = delta.wrapping_add(values[i]);
            values[i] = values[i - 1].wrapping_add(delta);
        }
    }
    Ok(Stream::I64(values))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_linear() {
        let v: Vec<i64> = (0..20).map(|i| 1_000 + i * 5).collect();
        let enc = encode(Stream::I64(v.clone())).unwrap();
        let dec = decode(enc.stream, None).unwrap().into_i64().unwrap();
        assert_eq!(dec, v);
    }
}
