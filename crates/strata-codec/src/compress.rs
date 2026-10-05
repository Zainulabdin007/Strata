//! General-purpose block compression (after semantic encoding).
//!
//! Uses established crates: [`lz4_flex`] for LZ4 and [`zstd`] for Zstandard.
//! Strata does not reimplement these algorithms.

use crate::error::{Error, Result};

thread_local! {
    static ZSTD_ENCODER: std::cell::RefCell<Option<(i32, zstd::bulk::Compressor<'static>)>> = const { std::cell::RefCell::new(None) };
    static ZSTD_DECODER: std::cell::RefCell<Option<zstd::bulk::Decompressor<'static>>> = const { std::cell::RefCell::new(None) };
}

/// Stable identifier for a compressor implementation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompressorId(pub &'static str);

/// Wire / config tag for block-level general compression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum CompressionCodec {
    /// No general compression (identity).
    #[default]
    None = 0,
    /// LZ4 block compression.
    Lz4 = 1,
    /// Zstandard block compression.
    Zstd = 2,
}

impl CompressionCodec {
    /// Stable name for config / CLI.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Lz4 => "lz4",
            Self::Zstd => "zstd",
        }
    }

    /// Parse a config name (`none`, `lz4`, `zstd`, plus aliases).
    pub fn parse_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "none" | "identity" | "off" | "false" => Some(Self::None),
            "lz4" => Some(Self::Lz4),
            "zstd" | "zstandard" => Some(Self::Zstd),
            _ => None,
        }
    }

    /// Wire tag.
    pub const fn tag(self) -> u8 {
        self as u8
    }

    /// Parse wire tag.
    pub fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::None),
            1 => Some(Self::Lz4),
            2 => Some(Self::Zstd),
            _ => None,
        }
    }

    /// Default compression level when the profile omits `level`.
    pub const fn default_level(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Lz4 => 4,
            Self::Zstd => 3,
        }
    }
}

/// Optional second-stage compressor applied after semantic encoding.
pub trait Compressor: Send + Sync {
    /// Stable id for metrics and format metadata.
    fn id(&self) -> CompressorId;

    /// Codec tag written into the block frame.
    fn codec(&self) -> CompressionCodec;

    /// Compresses encoded bytes.
    fn compress(&self, input: &[u8]) -> Result<Vec<u8>>;

    /// Decompresses bytes previously produced by [`compress`](Self::compress).
    ///
    /// `uncompressed_len` is the known original size from the block frame.
    fn decompress(&self, input: &[u8], uncompressed_len: usize) -> Result<Vec<u8>>;
}

/// Identity compressor: returns input unchanged.
#[derive(Debug, Default, Clone, Copy)]
pub struct IdentityCompressor;

impl Compressor for IdentityCompressor {
    fn id(&self) -> CompressorId {
        CompressorId("none")
    }

    fn codec(&self) -> CompressionCodec {
        CompressionCodec::None
    }

    fn compress(&self, input: &[u8]) -> Result<Vec<u8>> {
        Ok(input.to_vec())
    }

    fn decompress(&self, input: &[u8], uncompressed_len: usize) -> Result<Vec<u8>> {
        if input.len() != uncompressed_len {
            return Err(Error::Decode {
                message: format!(
                    "identity decompress length mismatch: got {}, expected {uncompressed_len}",
                    input.len()
                ),
            });
        }
        Ok(input.to_vec())
    }
}

/// LZ4 block compressor (`lz4_flex`).
#[derive(Debug, Clone, Copy)]
pub struct Lz4Compressor {
    /// Reserved for future acceleration/level knobs; `lz4_flex` uses a fixed default.
    pub level: i32,
}

impl Lz4Compressor {
    /// Creates an LZ4 compressor (level is recorded for config symmetry).
    pub fn new(level: i32) -> Self {
        Self { level }
    }
}

impl Default for Lz4Compressor {
    fn default() -> Self {
        Self::new(CompressionCodec::Lz4.default_level())
    }
}

impl Compressor for Lz4Compressor {
    fn id(&self) -> CompressorId {
        CompressorId("lz4")
    }

    fn codec(&self) -> CompressionCodec {
        CompressionCodec::Lz4
    }

    fn compress(&self, input: &[u8]) -> Result<Vec<u8>> {
        let _ = self.level;
        Ok(lz4_flex::block::compress(input))
    }

    fn decompress(&self, input: &[u8], uncompressed_len: usize) -> Result<Vec<u8>> {
        crate::limits::check_bytes(uncompressed_len)?;
        let out =
            lz4_flex::block::decompress(input, uncompressed_len).map_err(|e| Error::Decode {
                message: format!("lz4 decompress failed: {e}"),
            })?;
        if out.len() != uncompressed_len {
            return Err(Error::Decode {
                message: "lz4 length mismatch".into(),
            });
        }
        Ok(out)
    }
}

/// Zstandard block compressor (`zstd` crate).
#[derive(Debug, Clone, Copy)]
pub struct ZstdCompressor {
    /// Compression level (typically 1–22; default 3).
    pub level: i32,
}

impl ZstdCompressor {
    /// Creates a Zstd compressor at `level`.
    pub fn new(level: i32) -> Self {
        Self { level }
    }
}

impl Default for ZstdCompressor {
    fn default() -> Self {
        Self::new(CompressionCodec::Zstd.default_level())
    }
}

impl Compressor for ZstdCompressor {
    fn id(&self) -> CompressorId {
        CompressorId("zstd")
    }

    fn codec(&self) -> CompressionCodec {
        CompressionCodec::Zstd
    }

    fn compress(&self, input: &[u8]) -> Result<Vec<u8>> {
        ZSTD_ENCODER.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.as_ref().is_none_or(|(level, _)| *level != self.level) {
                *slot = Some((
                    self.level,
                    zstd::bulk::Compressor::new(self.level).map_err(|e| Error::InvalidInput {
                        message: e.to_string(),
                    })?,
                ));
            }
            slot.as_mut()
                .unwrap()
                .1
                .compress(input)
                .map_err(|e| Error::InvalidInput {
                    message: format!("zstd compress failed: {e}"),
                })
        })
    }

    fn decompress(&self, input: &[u8], uncompressed_len: usize) -> Result<Vec<u8>> {
        crate::limits::check_bytes(uncompressed_len)?;
        let out = ZSTD_DECODER.with(|slot| -> Result<Vec<u8>> {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                *slot = Some(zstd::bulk::Decompressor::new().map_err(|e| Error::Decode {
                    message: e.to_string(),
                })?);
            }
            slot.as_mut()
                .unwrap()
                .decompress(input, uncompressed_len)
                .map_err(|e| Error::Decode {
                    message: format!("zstd decompress failed: {e}"),
                })
        })?;
        if out.len() != uncompressed_len {
            return Err(Error::Decode {
                message: "zstd length mismatch".into(),
            });
        }
        Ok(out)
    }
}

/// Builds a compressor for `codec` at `level` (or the codec default level).
pub fn compressor_for(codec: CompressionCodec, level: Option<i32>) -> Box<dyn Compressor> {
    let level = level.unwrap_or(codec.default_level());
    match codec {
        CompressionCodec::None => Box::new(IdentityCompressor),
        CompressionCodec::Lz4 => Box::new(Lz4Compressor::new(level)),
        CompressionCodec::Zstd => Box::new(ZstdCompressor::new(level)),
    }
}

/// Looks up a named compressor (`none` / `lz4` / `zstd`).
pub fn lookup_compressor(name: &str) -> Result<Box<dyn Compressor>> {
    let codec = CompressionCodec::parse_name(name).ok_or_else(|| Error::NotImplemented {
        id: name.to_string(),
    })?;
    Ok(compressor_for(codec, None))
}

/// Looks up a named compressor with an explicit level.
pub fn lookup_compressor_with_level(name: &str, level: Option<i32>) -> Result<Box<dyn Compressor>> {
    let codec = CompressionCodec::parse_name(name).ok_or_else(|| Error::NotImplemented {
        id: name.to_string(),
    })?;
    Ok(compressor_for(codec, level))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_roundtrip() {
        let c = IdentityCompressor;
        let out = c.compress(b"hello").unwrap();
        assert_eq!(c.decompress(&out, out.len()).unwrap(), b"hello");
    }

    #[test]
    fn lz4_roundtrip() {
        let c = Lz4Compressor::default();
        let input = b"aaaaaaaaaaaaaaaaaaaaaaaahello world".repeat(20);
        let out = c.compress(&input).unwrap();
        assert!(out.len() < input.len());
        assert_eq!(c.decompress(&out, input.len()).unwrap(), input);
    }

    #[test]
    fn zstd_roundtrip() {
        let c = ZstdCompressor::new(3);
        let input = b"bbbbbbbbbbbbbbbbbbbbbbbbhello strata".repeat(20);
        let out = c.compress(&input).unwrap();
        assert!(out.len() < input.len());
        assert_eq!(c.decompress(&out, input.len()).unwrap(), input);
    }

    #[test]
    fn unknown_compressor() {
        let err = lookup_compressor("snappy").err().expect("expected error");
        assert!(matches!(err, Error::NotImplemented { .. }));
    }
}
