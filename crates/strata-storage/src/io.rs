//! Little-endian binary read/write helpers.

use std::io::Write;
use std::path::Path;

use crate::error::{Error, Result};

/// Writes a `u16` little-endian.
pub fn write_u16(w: &mut impl Write, path: &Path, v: u16) -> Result<()> {
    w.write_all(&v.to_le_bytes()).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Writes a `u32` little-endian.
pub fn write_u32(w: &mut impl Write, path: &Path, v: u32) -> Result<()> {
    w.write_all(&v.to_le_bytes()).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Writes a `u64` little-endian.
pub fn write_u64(w: &mut impl Write, path: &Path, v: u64) -> Result<()> {
    w.write_all(&v.to_le_bytes()).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Writes a length-prefixed UTF-8 string (`u32` length + bytes).
pub fn write_string(w: &mut impl Write, path: &Path, s: &str) -> Result<()> {
    let len = u32::try_from(s.len()).map_err(|_| Error::InvalidState {
        message: "string exceeds u32 length".into(),
    })?;
    write_u32(w, path, len)?;
    w.write_all(s.as_bytes()).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Writes raw bytes.
pub fn write_bytes(w: &mut impl Write, path: &Path, bytes: &[u8]) -> Result<()> {
    w.write_all(bytes).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Cursor over a byte slice with structured truncation errors.
#[derive(Debug, Clone)]
pub struct BufCursor<'a> {
    data: &'a [u8],
    pos: usize,
    path: &'a Path,
}

impl<'a> BufCursor<'a> {
    /// Creates a cursor.
    pub fn new(data: &'a [u8], path: &'a Path) -> Self {
        Self { data, pos: 0, path }
    }

    /// Current absolute position.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Remaining bytes.
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// Reads exactly `len` bytes.
    pub fn read_exact(&mut self, len: usize) -> Result<&'a [u8]> {
        if self.remaining() < len {
            return Err(Error::corrupt(
                self.path,
                format!(
                    "unexpected EOF at offset {}: need {len} bytes, have {}",
                    self.pos,
                    self.remaining()
                ),
            ));
        }
        let start = self.pos;
        self.pos += len;
        Ok(&self.data[start..self.pos])
    }

    /// Reads a `u8`.
    pub fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_exact(1)?[0])
    }

    /// Reads a `u16` LE.
    pub fn read_u16(&mut self) -> Result<u16> {
        let mut buf = [0u8; 2];
        buf.copy_from_slice(self.read_exact(2)?);
        Ok(u16::from_le_bytes(buf))
    }

    /// Reads a `u32` LE.
    pub fn read_u32(&mut self) -> Result<u32> {
        let mut buf = [0u8; 4];
        buf.copy_from_slice(self.read_exact(4)?);
        Ok(u32::from_le_bytes(buf))
    }

    /// Reads a `u64` LE.
    pub fn read_u64(&mut self) -> Result<u64> {
        let mut buf = [0u8; 8];
        buf.copy_from_slice(self.read_exact(8)?);
        Ok(u64::from_le_bytes(buf))
    }

    /// Reads a length-prefixed UTF-8 string.
    pub fn read_string(&mut self) -> Result<String> {
        let len = self.read_u32()? as usize;
        let bytes = self.read_exact(len)?;
        String::from_utf8(bytes.to_vec()).map_err(|e| {
            Error::corrupt(
                self.path,
                format!("invalid UTF-8 string at {}: {e}", self.pos),
            )
        })
    }
}

/// Reads a bounded file snapshot. File growth cannot increase the allocation.
pub fn read_file(path: &Path) -> Result<Vec<u8>> {
    read_file_with_limit(path, 1024 * 1024 * 1024)
}

/// Reads at most `limit` bytes, rejecting oversized or concurrently resized files.
pub fn read_file_with_limit(path: &Path, limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let length = file
        .metadata()
        .map_err(|source| Error::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    if length > limit {
        return Err(Error::corrupt(
            path,
            format!("file exceeds {limit} byte read limit"),
        ));
    }
    let length = usize::try_from(length)
        .map_err(|_| Error::corrupt(path, "file does not fit address space"))?;
    let mut bytes = vec![0; length];
    file.read_exact(&mut bytes).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut extra = [0];
    if file.read(&mut extra).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })? != 0
    {
        return Err(Error::corrupt(path, "file grew while reading snapshot"));
    }
    Ok(bytes)
}

/// Ensures parent directories exist for `path`.
pub fn ensure_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    Ok(())
}

/// CRC32 of the file prefix up to its current position, without allocating the file.
/// Leaves the cursor at the original position.
pub fn checksum_prefix(file: &mut std::fs::File, path: &Path) -> Result<u32> {
    use std::io::{Read, Seek, SeekFrom};
    let run = || -> std::io::Result<u32> {
        let end = file.stream_position()?;
        file.seek(SeekFrom::Start(0))?;
        let mut left = end;
        let mut buffer = [0u8; 65536];
        let mut hash = crc32fast::Hasher::new();
        while left > 0 {
            let count = (left as usize).min(buffer.len());
            file.read_exact(&mut buffer[..count])?;
            hash.update(&buffer[..count]);
            left -= count as u64;
        }
        Ok(hash.finalize())
    };
    let mut run = run;
    run().map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}
