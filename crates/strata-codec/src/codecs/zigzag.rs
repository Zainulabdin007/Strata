//! ZigZag mapping: i64 ↔ u64.

use crate::codecs::StepOutput;
use crate::error::Result;
use crate::plan::StepMetadata;
use crate::stream::Stream;

pub fn encode(input: Stream) -> Result<StepOutput> {
    let values = input.into_i64()?;
    let out = values.into_iter().map(zigzag_encode).collect();
    Ok(StepOutput {
        stream: Stream::U64(out),
        meta: None,
    })
}

pub fn decode(input: Stream, _meta: Option<&StepMetadata>) -> Result<Stream> {
    let mut values = input.into_u64()?;
    for value in &mut values {
        *value = zigzag_decode(*value) as u64;
    }
    Ok(Stream::I64(crate::stream::reinterpret_u64_bits_as_i64(values)))
}

pub fn zigzag_encode(n: i64) -> u64 {
    ((n << 1) ^ (n >> 63)) as u64
}

pub fn zigzag_decode(n: u64) -> i64 {
    ((n >> 1) as i64) ^ -((n & 1) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_signed() {
        for n in [0i64, 1, -1, 2, -2, i64::MAX, i64::MIN, 42, -42] {
            assert_eq!(zigzag_decode(zigzag_encode(n)), n);
        }
    }
}
