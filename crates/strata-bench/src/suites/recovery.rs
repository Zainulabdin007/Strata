//! Recovery suite: truncate a sealed segment, recover, measure.

use std::fs;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::time::Instant;

use anyhow::{bail, Result};
use serde_json::json;
use strata_storage::{recover_dataset, verify_dataset, DatasetReader};

use crate::config::{ingest_streaming, BenchConfig};
use crate::report::BenchReport;

pub fn run_recovery(config: &BenchConfig) -> Result<BenchReport> {
    let root = config.dataset_path("recovery");
    let _ = fs::remove_dir_all(&root);
    let _ = ingest_streaming(config, &root, false, None)?;

    let reader = DatasetReader::open(&root)?;
    if reader.segment_count() == 0 {
        bail!("recovery bench requires at least one segment");
    }
    let seg_path = root
        .join("segments")
        .join(&reader.manifest().segments[0].file);
    let meta_len = fs::metadata(&seg_path)?.len();
    if meta_len < 64 {
        bail!("segment too small to truncate meaningfully");
    }
    // Truncate away the footer / tail to force recovery.
    let new_len = meta_len.saturating_sub(48).max(32);
    {
        let mut f = OpenOptions::new().write(true).open(&seg_path)?;
        f.set_len(new_len)?;
        f.seek(SeekFrom::End(0))?;
        f.write_all(&[0xDE, 0xAD])?; // incomplete tail bytes
        f.sync_all()?;
    }
    // Remove manifest so recover rebuilds it.
    let _ = fs::remove_file(root.join("manifest.json"));

    let t0 = Instant::now();
    let report = recover_dataset(&root)?;
    let recover_elapsed = t0.elapsed();
    let verify = verify_dataset(&root)?;
    let verify_elapsed = {
        let t1 = Instant::now();
        let _ = verify_dataset(&root)?;
        t1.elapsed()
    };

    let metrics = json!({
        "truncate_from_bytes": meta_len,
        "truncate_to_bytes": new_len,
        "recover_elapsed_ms": recover_elapsed.as_millis() as u64,
        "recovered_records": report.record_count,
        "segments_seen": report.segments.len(),
        "manifest_rebuilt": report.manifest_rebuilt,
        "verify_ok": verify.ok(),
        "verify_elapsed_ms": verify_elapsed.as_millis() as u64,
        "verify_healthy_records": verify.healthy_records,
    });

    Ok(BenchReport {
        suite: "recovery".into(),
        meta: config.meta(),
        metrics,
    })
}
