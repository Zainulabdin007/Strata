//! Immutable anonymous mapped snapshots of sealed segment files.
//!
//! # Safety model
//!
//! Memory-mapping is inherently `unsafe` at the OS boundary. This module
//! concentrates that unsafety behind a small API. Callers must only map
//! **sealed** Strata segments. Bytes are copied into private anonymous pages
//! before exposure, so external file changes cannot cause undefined behavior.
//! This intentionally trades file-backed zero-copy for a safe snapshot.
//!
//! On non-Unix targets this module provides no mapping; callers should use
//! buffered [`std::fs::read`].

use std::fs::File;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;

use crate::error::{Error, Result};

/// How a sealed segment's bytes are sourced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SegmentReadMode {
    /// Always `read(2)` the full file into a heap buffer.
    Buffered,
    /// Always memory-map the file (errors if mmap is unavailable).
    Mmap,
    /// Prefer mmap; fall back to buffered on failure or unsupported platforms.
    #[default]
    Auto,
}

impl SegmentReadMode {
    /// Stable name for CLI / benchmarks.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Buffered => "buffered",
            Self::Mmap => "mmap",
            Self::Auto => "auto",
        }
    }
}

/// Which backend ultimately provided segment bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SegmentBytesBackend {
    /// Owned heap buffer from a full-file read.
    Buffered,
    /// Read-only memory map of a sealed file.
    Mmap,
}

/// Byte source for a sealed segment: either an owned buffer or a read-only map.
#[derive(Debug)]
pub enum SealedSegmentBytes {
    /// Full-file heap copy.
    Buffered(Vec<u8>),
    /// Memory-mapped sealed file.
    Mapped(MappedFile),
}

impl SealedSegmentBytes {
    /// Opens sealed segment bytes according to `mode`.
    pub fn open(path: &Path, mode: SegmentReadMode) -> Result<Self> {
        match mode {
            SegmentReadMode::Buffered => {
                let bytes = crate::io::read_file(path)?;
                Ok(Self::Buffered(bytes))
            }
            SegmentReadMode::Mmap => {
                let mapped = MappedFile::open(path)?;
                Ok(Self::Mapped(mapped))
            }
            SegmentReadMode::Auto => match MappedFile::open(path) {
                Ok(mapped) => Ok(Self::Mapped(mapped)),
                Err(_) => {
                    let bytes = crate::io::read_file(path)?;
                    Ok(Self::Buffered(bytes))
                }
            },
        }
    }

    /// Borrowed view of the full segment file.
    ///
    /// For [`Self::Mapped`], this is a zero-copy view into the mapped region.
    pub fn as_slice(&self) -> &[u8] {
        match self {
            Self::Buffered(v) => v.as_slice(),
            Self::Mapped(m) => m.as_slice(),
        }
    }

    /// Backend in use.
    pub fn backend(&self) -> SegmentBytesBackend {
        match self {
            Self::Buffered(_) => SegmentBytesBackend::Buffered,
            Self::Mapped(_) => SegmentBytesBackend::Mmap,
        }
    }

    /// Byte length.
    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    /// True when the segment file is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Read-only memory mapping of a sealed immutable file.
///
/// # Invariants
///
/// 1. `ptr` points to a mapping of exactly `len` bytes created by `mmap`.
/// 2. The mapping is `PROT_READ` only.
/// 3. Anonymous pages hold a complete initialized snapshot; no writable alias
///    or mutable file-backed mapping is exposed.
/// 4. `Drop` unmaps exactly once via `munmap`.
pub struct MappedFile {
    path: PathBuf,
    len: usize,
    ptr: NonNull<u8>,
    /// Kept so the file descriptor lifetime is obvious in debug tooling; the
    /// mapping itself owns the pages after `mmap` on Unix.
    _file: File,
}

// SAFETY:
// - Invariant: the mapping is `PROT_READ` only and never mutated through
//   `&MappedFile` or `&[u8]` views. Shared concurrent reads of immutable
//   sealed-segment bytes are data-race free.
// - Why unsafe is required: auto traits cannot prove OS-backed mappings are
//   Sync; we assert the read-only sealed-segment contract instead.
unsafe impl Send for MappedFile {}
// SAFETY: same invariants as `Send` — read-only shared access to sealed bytes.
unsafe impl Sync for MappedFile {}

impl std::fmt::Debug for MappedFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappedFile")
            .field("path", &self.path)
            .field("len", &self.len)
            .finish_non_exhaustive()
    }
}

impl MappedFile {
    /// Memory-maps `path` read-only.
    ///
    /// Empty files are rejected (use a buffered empty `Vec` instead); zero-length
    /// `mmap` is not portable.
    pub fn open(path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            open_unix(path)
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            Err(Error::InvalidState {
                message: "memory mapping is not supported on this platform".into(),
            })
        }
    }

    /// Path that was mapped.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of mapped bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// True when the mapping is empty (should not occur for successful opens).
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Zero-copy view of the mapped bytes.
    pub fn as_slice(&self) -> &[u8] {
        // SAFETY:
        // - Invariant: `ptr`/`len` describe a live `PROT_READ` mapping created in
        //   `open` and not yet `munmap`'d (that happens only in `Drop`).
        // - Why unsafe is required: turning a raw mapping pointer into a Rust
        //   slice requires `from_raw_parts`, which the language treats as unsafe.
        // - Aliasing: only shared references are handed out; the mapping is
        //   read-only at the OS level, matching `&[u8]`.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }

    /// Subslice view with bounds checks (still zero-copy into the map).
    pub fn view(&self, start: usize, end: usize) -> Result<&[u8]> {
        let slice = self.as_slice();
        slice.get(start..end).ok_or_else(|| Error::InvalidState {
            message: format!(
                "mapped view [{start},{end}) out of range for {} (len={})",
                self.path.display(),
                self.len
            ),
        })
    }
}

impl Drop for MappedFile {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            // SAFETY:
            // - Invariant: `ptr`/`len` are exactly the arguments from a successful
            //   `mmap` in `open`; this `Drop` runs at most once.
            // - Why unsafe is required: `munmap` is an FFI syscall with raw
            //   pointer/length arguments.
            let rc = unsafe { libc::munmap(self.ptr.as_ptr() as *mut libc::c_void, self.len) };
            debug_assert_eq!(rc, 0, "munmap failed for {}", self.path.display());
        }
    }
}

#[cfg(unix)]
fn open_unix(path: &Path) -> Result<MappedFile> {
    use std::io::Read;

    let mut file = File::open(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let meta = file.metadata().map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let len = usize::try_from(meta.len()).map_err(|_| Error::InvalidState {
        message: format!("segment file too large to map: {}", path.display()),
    })?;
    if len == 0 || len > 1024 * 1024 * 1024 {
        return Err(Error::InvalidState {
            message: format!("refusing to mmap empty file: {}", path.display()),
        });
    }

    // SAFETY: anonymous private pages are owned exclusively by this object.
    // They are initialized before exposure and made read-only afterwards.
    // External truncation or modification cannot invalidate any borrowed slice.
    let raw = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            len,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        )
    };
    if raw == libc::MAP_FAILED {
        let err = std::io::Error::last_os_error();
        return Err(Error::Io {
            path: path.to_path_buf(),
            source: err,
        });
    }

    let ptr = NonNull::new(raw as *mut u8).ok_or_else(|| Error::InvalidState {
        message: format!("mmap returned null for {}", path.display()),
    })?;

    // SAFETY: fresh mapping spans len initialized anonymous bytes, exclusively owned.
    let bytes = unsafe { std::slice::from_raw_parts_mut(ptr.as_ptr(), len) };
    let initialized = file.read_exact(bytes);
    // SAFETY: this is our live, page-aligned mapping. No slice is exposed yet.
    let protected = unsafe { libc::mprotect(raw, len, libc::PROT_READ) };
    if initialized.is_err() || protected != 0 {
        let source = initialized
            .err()
            .unwrap_or_else(std::io::Error::last_os_error);
        // SAFETY: release the mapping on initialization failure, exactly once.
        unsafe {
            libc::munmap(raw, len);
        }
        return Err(Error::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(MappedFile {
        path: path.to_path_buf(),
        len,
        ptr,
        _file: file,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "strata-mmap-{}-{}-{}",
            name,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    #[test]
    #[cfg(unix)]
    fn mapped_file_roundtrip_bytes() {
        let path = temp_path("roundtrip");
        let payload = b"STRATA-SEALED-SEGMENT-BYTES-0123456789";
        {
            let mut f = File::create(&path).unwrap();
            f.write_all(payload).unwrap();
            f.sync_all().unwrap();
        }

        let mapped = MappedFile::open(&path).unwrap();
        assert_eq!(mapped.len(), payload.len());
        assert_eq!(mapped.as_slice(), payload);
        assert_eq!(mapped.view(0, 6).unwrap(), b"STRATA");
        assert!(mapped.view(0, payload.len() + 1).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[cfg(unix)]
    fn sealed_bytes_auto_prefers_mmap() {
        let path = temp_path("auto");
        {
            let mut f = File::create(&path).unwrap();
            f.write_all(&[1u8; 64]).unwrap();
            f.sync_all().unwrap();
        }
        let bytes = SealedSegmentBytes::open(&path, SegmentReadMode::Auto).unwrap();
        assert_eq!(bytes.backend(), SegmentBytesBackend::Mmap);
        assert_eq!(bytes.as_slice().len(), 64);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn sealed_bytes_buffered_mode() {
        let path = temp_path("buffered");
        {
            let mut f = File::create(&path).unwrap();
            f.write_all(b"hello-buffered").unwrap();
            f.sync_all().unwrap();
        }
        let bytes = SealedSegmentBytes::open(&path, SegmentReadMode::Buffered).unwrap();
        assert_eq!(bytes.backend(), SegmentBytesBackend::Buffered);
        assert_eq!(bytes.as_slice(), b"hello-buffered");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[cfg(unix)]
    fn mmap_empty_file_rejected() {
        let path = temp_path("empty");
        File::create(&path).unwrap();
        let err = MappedFile::open(&path).unwrap_err();
        assert!(err.to_string().contains("empty"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[cfg(unix)]
    fn drop_unmaps_without_panic() {
        let path = temp_path("drop");
        {
            let mut f = File::create(&path).unwrap();
            f.write_all(&[7u8; 128]).unwrap();
            f.sync_all().unwrap();
        }
        let mapped = MappedFile::open(&path).unwrap();
        let len = mapped.len();
        drop(mapped); // must munmap cleanly
        assert_eq!(len, 128);
        let _ = std::fs::remove_file(&path);
    }
}
