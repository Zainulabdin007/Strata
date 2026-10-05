//! Environment / run metadata persisted with results.

use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::gen::Workload;
use crate::size::BenchSize;

/// Metadata describing how a benchmark was produced.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchMeta {
    /// `git rev-parse HEAD` when available.
    pub git_commit: Option<String>,
    /// `rustc --version`.
    pub rustc_version: Option<String>,
    /// Cargo profile (debug/release) inferred from `cfg!(debug_assertions)`.
    pub cargo_profile: String,
    /// Whether optimized (`opt-level` proxy via debug_assertions).
    pub debug_assertions: bool,
    /// Compiler / build settings captured at run time (`RUSTFLAGS`, opt-level hint).
    pub compiler_settings: String,
    /// OS family.
    pub os: String,
    /// CPU architecture.
    pub arch: String,
    /// Dataset RNG seed.
    pub dataset_seed: u64,
    /// Workload / profile name.
    pub workload: String,
    /// Worker count used for ingest.
    pub worker_count: usize,
    /// Block row count.
    pub block_size: u32,
    /// Segment target size bytes.
    pub segment_size_bytes: u64,
    /// Record count.
    pub record_count: u64,
    /// Size label.
    pub size_label: String,
    /// Compression codec name.
    pub compression_codec: String,
    /// UTC unix timestamp when the run started.
    pub started_unix_secs: u64,
}

impl BenchMeta {
    /// Collects host / build metadata for this run.
    pub fn collect(
        seed: u64,
        workload: Workload,
        size: BenchSize,
        workers: usize,
        block_size: u32,
        segment_size_bytes: u64,
        compression_codec: &str,
    ) -> Self {
        Self {
            git_commit: git_head(),
            rustc_version: rustc_version(),
            cargo_profile: if cfg!(debug_assertions) {
                "debug".into()
            } else {
                "release".into()
            },
            debug_assertions: cfg!(debug_assertions),
            compiler_settings: compiler_settings(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            dataset_seed: seed,
            workload: workload.as_str().to_string(),
            worker_count: workers,
            block_size,
            segment_size_bytes,
            record_count: size.records(),
            size_label: size.as_str().to_string(),
            compression_codec: compression_codec.to_string(),
            started_unix_secs: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        }
    }
}

fn git_head() -> Option<String> {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn rustc_version() -> Option<String> {
    let out = Command::new("rustc").arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn compiler_settings() -> String {
    let rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    let opt = if cfg!(debug_assertions) {
        "opt-level=0 (debug_assertions)"
    } else {
        "opt-level>=1 (release/no debug_assertions)"
    };
    if rustflags.is_empty() {
        opt.to_string()
    } else {
        format!("{opt}; RUSTFLAGS={rustflags}")
    }
}
