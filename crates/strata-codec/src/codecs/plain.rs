//! PLAIN serializer: typed present values → bytes.

use crate::codecs::StepOutput;
use crate::error::{Error, Result};
use crate::plan::StepMetadata;
use crate::stream::Stream;

pub fn encode(input: Stream) -> Result<StepOutput> {
    let mut out = Vec::new();
    match input {
        Stream::I64(values) => {
            out.push(1); // type tag i64
            write_u32(&mut out, values.len() as u32)?;
            for v in values {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        Stream::U64(values) => {
            out.push(2);
            write_u32(&mut out, values.len() as u32)?;
            for v in values {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        Stream::Bool(values) => {
            out.push(3);
            write_u32(&mut out, values.len() as u32)?;
            for v in values {
                out.push(u8::from(v));
            }
        }
        Stream::Bytes(values) => {
            out.push(4);
            write_u32(&mut out, values.len() as u32)?;
            for v in values {
                write_u32(&mut out, v.len() as u32)?;
                out.extend_from_slice(&v);
            }
        }
        Stream::Encoded(_) => {
            return Err(Error::InvalidPlan {
                message: "PLAIN cannot encode an already-encoded stream".into(),
            });
        }
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
    if bytes.is_empty() {
        return Err(Error::Decode {
            message: "empty PLAIN payload".into(),
        });
    }
    let tag = bytes[0];
    let mut cur = Cursor::new(&bytes[1..]);
    let len = cur.read_u32()? as usize;
    crate::limits::check_count(len)?;
    let width = match tag {
        1 | 2 => 8,
        3 => 1,
        4 => 4,
        _ => 1,
    };
    if len > bytes.len().saturating_sub(5) / width {
        return Err(Error::Decode {
            message: "plain count exceeds payload".into(),
        });
    }
    match tag {
        1 => {
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                values.push(cur.read_i64()?);
            }
            cur.finish()?;
            Ok(Stream::I64(values))
        }
        2 => {
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                values.push(cur.read_u64()?);
            }
            cur.finish()?;
            Ok(Stream::U64(values))
        }
        3 => {
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                let value = cur.read_u8()?;
                if value > 1 {
                    return Err(Error::Decode {
                        message: "invalid boolean".into(),
                    });
                }
                values.push(value == 1);
            }
            cur.finish()?;
            Ok(Stream::Bool(values))
        }
        4 => {
            let mut values = Vec::with_capacity(len);
            for _ in 0..len {
                let n = cur.read_u32()? as usize;
                values.push(cur.read_exact(n)?.to_vec());
            }
            cur.finish()?;
            Ok(Stream::Bytes(values))
        }
        other => Err(Error::Decode {
            message: format!("unknown PLAIN type tag {other}"),
        }),
    }
}

fn write_u32(out: &mut Vec<u8>, v: u32) -> Result<()> {
    out.extend_from_slice(&v.to_le_bytes());
    Ok(())
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }
    fn read_exact(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.pos + n > self.data.len() {
            return Err(Error::Decode {
                message: "truncated PLAIN payload".into(),
            });
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }
    fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_exact(1)?[0])
    }
    fn read_u32(&mut self) -> Result<u32> {
        let mut b = [0u8; 4];
        b.copy_from_slice(self.read_exact(4)?);
        Ok(u32::from_le_bytes(b))
    }
    fn read_i64(&mut self) -> Result<i64> {
        let mut b = [0u8; 8];
        b.copy_from_slice(self.read_exact(8)?);
        Ok(i64::from_le_bytes(b))
    }
    fn read_u64(&mut self) -> Result<u64> {
        let mut b = [0u8; 8];
        b.copy_from_slice(self.read_exact(8)?);
        Ok(u64::from_le_bytes(b))
    }
    fn finish(self) -> Result<()> {
        if self.pos != self.data.len() {
            Err(Error::Decode {
                message: "trailing bytes in PLAIN payload".into(),
            })
        } else {
            Ok(())
        }
    }
}
