//! Enum bit-packing: variant names → dense indices → bitpack.

use crate::codecs::bit_packing::{self, bit_width_for};
use crate::codecs::StepOutput;
use crate::error::{Error, Result};
use crate::plan::{EncodingStep, StepMetadata};
use crate::stream::Stream;

pub fn encode(input: Stream) -> Result<StepOutput> {
    let values = input.into_bytes_list()?;
    // Preserve first-seen order as the enum domain for this block.
    let mut variants: Vec<Vec<u8>> = Vec::new();
    let mut index = std::collections::HashMap::<Vec<u8>, u32>::new();
    let mut ids = Vec::with_capacity(values.len());
    for v in values {
        if let Some(&id) = index.get(&v) {
            ids.push(u64::from(id));
        } else {
            let id = variants.len() as u32;
            index.insert(v.clone(), id);
            variants.push(v);
            ids.push(u64::from(id));
        }
    }

    let bit_width = bit_width_for(&ids);
    let packed = bit_packing::pack(&ids, bit_width)?;

    let mut blob = Vec::new();
    blob.push(bit_width);
    write_u32(&mut blob, ids.len() as u32);
    write_u32(&mut blob, variants.len() as u32);
    for variant in &variants {
        write_u32(&mut blob, variant.len() as u32);
        blob.extend_from_slice(variant);
    }

    Ok(StepOutput {
        stream: Stream::Encoded(packed),
        meta: Some(StepMetadata {
            step: EncodingStep::EnumBitpack,
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
        message: "enum bitpack requires metadata".into(),
    })?;
    if meta.blob.len() < 9 {
        return Err(Error::Decode {
            message: "invalid enum bitpack metadata".into(),
        });
    }
    let bit_width = meta.blob[0];
    let count = u32::from_le_bytes(meta.blob[1..5].try_into().unwrap()) as usize;
    let n_variants = u32::from_le_bytes(meta.blob[5..9].try_into().unwrap()) as usize;
    crate::limits::check_count(n_variants)?;
    if n_variants > (meta.blob.len() - 9) / 4 {
        return Err(Error::Decode {
            message: "enum count exceeds metadata".into(),
        });
    }
    let mut pos = 9usize;
    let mut variants = Vec::with_capacity(n_variants);
    for _ in 0..n_variants {
        if pos + 4 > meta.blob.len() {
            return Err(Error::Decode {
                message: "truncated enum variant length".into(),
            });
        }
        let len = u32::from_le_bytes(meta.blob[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + len > meta.blob.len() {
            return Err(Error::Decode {
                message: "truncated enum variant".into(),
            });
        }
        variants.push(meta.blob[pos..pos + len].to_vec());
        pos += len;
    }
    if pos != meta.blob.len() {
        return Err(Error::Decode {
            message: "trailing bytes in enum bitpack metadata".into(),
        });
    }

    let ids = bit_packing::unpack(bytes, count, bit_width)?;
    let mut values = Vec::with_capacity(ids.len());
    let mut expanded = 0usize;
    for id in ids {
        let idx = id as usize;
        let v = variants.get(idx).ok_or_else(|| Error::Decode {
            message: format!("enum index {idx} out of range"),
        })?;
        expanded = expanded.checked_add(v.len()).ok_or_else(|| Error::Decode {
            message: "enum expansion overflow".into(),
        })?;
        crate::limits::check_bytes(expanded)?;
        values.push(v.clone());
    }
    Ok(Stream::Bytes(values))
}

fn write_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let v = vec![
            b"buy".to_vec(),
            b"sell".to_vec(),
            b"buy".to_vec(),
            b"buy".to_vec(),
            b"sell".to_vec(),
        ];
        let enc = encode(Stream::Bytes(v.clone())).unwrap();
        let dec = decode(enc.stream, enc.meta.as_ref())
            .unwrap()
            .into_bytes_list()
            .unwrap();
        assert_eq!(dec, v);
    }
}
