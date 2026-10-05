//! Baseline storage formats for fair size / throughput comparison.
//!
//! These are **not** Strata competitors claimed to be inferior. They are
//! controlled baselines written from the *same* generated records so size and
//! encode/decode cost can be compared honestly. See `benchmarks/COMPARISON.md`.

pub mod csv_baseline;
pub mod fixed;
pub mod jsonl;
pub mod logical;
#[cfg(feature = "parquet")]
pub mod parquet_baseline;

use std::fs::File;
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};

/// One measured baseline (or Strata) outcome.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FormatMeasurement {
    /// Format label.
    pub format: String,
    /// Output path (relative or absolute).
    pub path: String,
    /// On-disk size in bytes after encode.
    pub size_bytes: u64,
    /// Wall time to encode all records.
    pub encode_elapsed_ms: u64,
    /// Encode records/sec.
    pub encode_records_per_sec: f64,
    /// Wall time to fully decode / scan all records (when measured).
    pub decode_elapsed_ms: Option<u64>,
    /// Decode records/sec (when measured).
    pub decode_records_per_sec: Option<f64>,
    /// Records observed on decode (sanity).
    pub decode_records: Option<u64>,
    /// Short methodology note for this format.
    pub notes: String,
}

/// Open buffered writer helper.
pub fn buf_writer(path: &Path) -> Result<BufWriter<File>> {
    let f = File::create(path).with_context(|| format!("create {}", path.display()))?;
    Ok(BufWriter::with_capacity(1 << 20, f))
}

/// Open buffered reader helper.
pub fn buf_reader(path: &Path) -> Result<BufReader<File>> {
    let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    Ok(BufReader::with_capacity(1 << 20, f))
}

/// File size helper.
pub fn file_size(path: &Path) -> Result<u64> {
    Ok(std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .len())
}

/// Directory size (sum of regular files, non-recursive extras ignored beyond one level).
pub fn dir_size(path: &Path) -> Result<u64> {
    let mut total = 0u64;
    if path.is_file() {
        return file_size(path);
    }
    for entry in std::fs::read_dir(path).with_context(|| format!("read_dir {}", path.display()))? {
        let entry = entry?;
        let meta = entry.metadata()?;
        if meta.is_file() {
            total += meta.len();
        } else if meta.is_dir() {
            total += dir_size(&entry.path())?;
        }
    }
    Ok(total)
}

/// Timing helper.
pub fn elapsed_ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

pub fn rps(records: u64, d: Duration) -> f64 {
    let secs = d.as_secs_f64().max(1e-12);
    records as f64 / secs
}

/// Paths used by the compare suite under a work root.
#[derive(Debug, Clone)]
pub struct BaselinePaths {
    pub jsonl: PathBuf,
    pub csv: PathBuf,
    pub fixed: PathBuf,
    pub parquet: PathBuf,
    pub strata: PathBuf,
}

impl BaselinePaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            jsonl: root.join("baseline.jsonl"),
            csv: root.join("baseline.csv"),
            fixed: root.join("baseline.fixed"),
            parquet: root.join("baseline.parquet"),
            strata: root.join("strata"),
        }
    }
}
