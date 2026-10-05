//! Persist measured benchmark results (JSON + markdown).

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::meta::BenchMeta;

/// One suite result document.
#[derive(Debug, Clone, Serialize)]
pub struct BenchReport {
    /// Suite name.
    pub suite: String,
    /// Shared metadata.
    pub meta: BenchMeta,
    /// Suite-specific measured metrics (already-populated numbers only).
    pub metrics: serde_json::Value,
}

impl BenchReport {
    /// Writes `suite-…json` and appends a markdown section under `out_dir`.
    pub fn persist(&self, out_dir: &Path) -> Result<std::path::PathBuf> {
        fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;
        let stamp = self.meta.started_unix_secs;
        let base = format!(
            "{}-{}-{}-{}",
            self.suite, self.meta.workload, self.meta.size_label, stamp
        );
        let json_path = out_dir.join(format!("{base}.json"));
        let text = serde_json::to_string_pretty(self)?;
        fs::write(&json_path, text).with_context(|| format!("write {}", json_path.display()))?;

        let md_path = out_dir.join("RESULTS.md");
        let section = self.to_markdown_section();
        let mut md = if md_path.exists() {
            fs::read_to_string(&md_path)?
        } else {
            "# Strata bench results\n\nMeasured runs only. Re-run `strata-bench` to refresh.\n\n"
                .into()
        };
        md.push_str(&section);
        md.push('\n');
        fs::write(&md_path, md)?;
        Ok(json_path)
    }

    fn to_markdown_section(&self) -> String {
        let mut out = format!(
            "## {} / {} / {}\n\n",
            self.suite, self.meta.workload, self.meta.size_label
        );
        out.push_str(&format!(
            "- git: {}\n- rustc: {}\n- profile: {} (debug_assertions={})\n- compiler: {}\n- os/arch: {}/{}\n- seed: {}\n- workers: {}\n- block_size: {}\n- segment_size_bytes: {}\n- codec: {}\n- records: {}\n\n",
            self.meta.git_commit.as_deref().unwrap_or("unknown"),
            self.meta.rustc_version.as_deref().unwrap_or("unknown"),
            self.meta.cargo_profile,
            self.meta.debug_assertions,
            self.meta.compiler_settings,
            self.meta.os,
            self.meta.arch,
            self.meta.dataset_seed,
            self.meta.worker_count,
            self.meta.block_size,
            self.meta.segment_size_bytes,
            self.meta.compression_codec,
            self.meta.record_count,
        ));
        out.push_str("```json\n");
        out.push_str(&serde_json::to_string_pretty(&self.metrics).unwrap_or_default());
        out.push_str("\n```\n");
        out
    }
}
