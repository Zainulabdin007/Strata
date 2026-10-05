//! Dictionary encoding: byte values → dictionary + indices.

use std::collections::HashMap;

use crate::codecs::StepOutput;
use crate::error::{Error, Result};
use crate::plan::{EncodingStep, StepMetadata};
use crate::stream::Stream;

pub fn encode(input: Stream) -> Result<StepOutput> {
    let values = input.into_bytes_list()?;
    let mut dict: Vec<Vec<u8>> = Vec::new();
    let mut index: HashMap<Vec<u8>, u32> = HashMap::new();
    let mut ids = Vec::with_capacity(values.len());

    for v in values {
        if let Some(&id) = index.get(&v) {
            ids.push(u64::from(id));
        } else {
            let id = dict.len() as u32;
            index.insert(v.clone(), id);
            dict.push(v);
            ids.push(u64::from(id));
        }
    }

    let mut blob = Vec::new();
    write_u32(&mut blob, dict.len() as u32);
    for entry in &dict {
        write_u32(&mut blob, entry.len() as u32);
        blob.extend_from_slice(entry);
    }

    Ok(StepOutput {
        stream: Stream::U64(ids),
        meta: Some(StepMetadata {
            step: EncodingStep::DictionaryEncoding,
            blob,
        }),
    })
}

pub fn decode(input: Stream, meta: Option<&StepMetadata>) -> Result<Stream> {
    let meta = meta.ok_or_else(|| Error::Decode {
        message: "dictionary encoding requires metadata".into(),
    })?;
    let dict = parse_dict(&meta.blob)?;
    let ids = input.into_u64()?;
    let mut values = Vec::with_capacity(ids.len());
    let mut expanded = 0usize;
    for id in ids {
        let idx = usize::try_from(id).map_err(|_| Error::Decode {
            message: "dictionary index out of range".into(),
        })?;
        let entry = dict.get(idx).ok_or_else(|| Error::Decode {
            message: format!("dictionary index {idx} out of range"),
        })?;
        expanded = expanded
            .checked_add(entry.len())
            .ok_or_else(|| Error::Decode {
                message: "dictionary expansion overflow".into(),
            })?;
        crate::limits::check_bytes(expanded)?;
        values.push(entry.clone());
    }
    Ok(Stream::Bytes(values))
}

fn parse_dict(blob: &[u8]) -> Result<Vec<Vec<u8>>> {
    if blob.len() < 4 {
        return Err(Error::Decode {
            message: "truncated dictionary metadata".into(),
        });
    }
    let n = u32::from_le_bytes(blob[0..4].try_into().unwrap()) as usize;
    crate::limits::check_count(n)?;
    if n > (blob.len() - 4) / 4 {
        return Err(Error::Decode {
            message: "dictionary count exceeds metadata".into(),
        });
    }
    let mut pos = 4usize;
    let mut dict = Vec::with_capacity(n);
    for _ in 0..n {
        if pos + 4 > blob.len() {
            return Err(Error::Decode {
                message: "truncated dictionary entry length".into(),
            });
        }
        let len = u32::from_le_bytes(blob[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if pos + len > blob.len() {
            return Err(Error::Decode {
                message: "truncated dictionary entry".into(),
            });
        }
        dict.push(blob[pos..pos + len].to_vec());
        pos += len;
    }
    if pos != blob.len() {
        return Err(Error::Decode {
            message: "trailing bytes in dictionary metadata".into(),
        });
    }
    Ok(dict)
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
            b"AAPL".to_vec(),
            b"MSFT".to_vec(),
            b"AAPL".to_vec(),
            b"AAPL".to_vec(),
        ];
        let enc = encode(Stream::Bytes(v.clone())).unwrap();
        let dec = decode(enc.stream, enc.meta.as_ref())
            .unwrap()
            .into_bytes_list()
            .unwrap();
        assert_eq!(dec, v);
    }
}
