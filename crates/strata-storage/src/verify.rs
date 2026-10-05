//! Dataset and segment verification.

use std::fs;
use std::path::{Path, PathBuf};

use crate::dataset::DatasetManifest;
use crate::error::{Error, Result};
use crate::path::{DatasetLayout, SegmentPath};
use crate::segment::SegmentReader;

/// Severity of a verification finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifySeverity {
    /// Informational note.
    Info,
    /// Recoverable inconsistency.
    Warning,
    /// Data loss or unreadable corruption.
    Error,
}

/// One finding from [`verify_dataset`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyIssue {
    /// Severity.
    pub severity: VerifySeverity,
    /// Path related to the issue, when known.
    pub path: Option<PathBuf>,
    /// Human-readable description.
    pub message: String,
}

/// Result of verifying a dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyReport {
    /// Dataset root.
    pub root: PathBuf,
    /// Issues discovered (may be empty when healthy).
    pub issues: Vec<VerifyIssue>,
    /// Segments that opened cleanly.
    pub healthy_segments: u32,
    /// Total recoverable records across healthy segments.
    pub healthy_records: u64,
}

impl VerifyReport {
    /// True when no error-severity issues were found.
    pub fn ok(&self) -> bool {
        !self
            .issues
            .iter()
            .any(|i| i.severity == VerifySeverity::Error)
    }

    /// Formats a CLI summary.
    pub fn to_cli_summary(&self) -> String {
        let mut out = format!(
            "strata verify: {}\n  healthy_segments={}\n  healthy_records={}\n  ok={}\n",
            self.root.display(),
            self.healthy_segments,
            self.healthy_records,
            self.ok()
        );
        if self.issues.is_empty() {
            out.push_str("  issues: none\n");
        } else {
            out.push_str("  issues:\n");
            for issue in &self.issues {
                let sev = match issue.severity {
                    VerifySeverity::Info => "info",
                    VerifySeverity::Warning => "warn",
                    VerifySeverity::Error => "error",
                };
                let path = issue
                    .path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "-".into());
                out.push_str(&format!("    [{sev}] {path}: {}\n", issue.message));
            }
        }
        out
    }
}

/// Validates segments, block checksums, and manifest consistency.
pub fn verify_dataset(root: impl AsRef<Path>) -> Result<VerifyReport> {
    let layout = DatasetLayout::new(root.as_ref());
    let mut issues = Vec::new();
    let mut healthy_segments = 0u32;
    let mut healthy_records = 0u64;

    let seg_dir = layout.segments_dir();
    let mut files: Vec<PathBuf> = if seg_dir.is_dir() {
        fs::read_dir(&seg_dir)
            .map_err(|source| Error::Io {
                path: seg_dir.clone(),
                source,
            })?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e == "strata")
            })
            .collect()
    } else {
        issues.push(VerifyIssue {
            severity: VerifySeverity::Error,
            path: Some(seg_dir.clone()),
            message: "segments directory missing".into(),
        });
        Vec::new()
    };
    files.sort();

    let mut opened: Vec<(String, u64, u32)> = Vec::new();
    for path in &files {
        match SegmentReader::open(SegmentPath::new(path)) {
            Ok(reader) => {
                // Force-decode every block to validate per-block CRCs + payloads.
                match reader.read_all_records() {
                    Ok(records) => {
                        let meta = reader.meta();
                        if records.len() as u64 != meta.header.record_count {
                            issues.push(VerifyIssue {
                                severity: VerifySeverity::Warning,
                                path: Some(path.clone()),
                                message: format!(
                                    "decoded records {} != header record_count {}",
                                    records.len(),
                                    meta.header.record_count
                                ),
                            });
                        }
                        healthy_segments += 1;
                        healthy_records += records.len() as u64;
                        let name = path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("unknown")
                            .to_string();
                        opened.push((name, records.len() as u64, meta.header.block_count));
                    }
                    Err(err) => {
                        issues.push(VerifyIssue {
                            severity: VerifySeverity::Error,
                            path: Some(path.clone()),
                            message: format!("block decode failed: {err}"),
                        });
                    }
                }
            }
            Err(err) => {
                issues.push(VerifyIssue {
                    severity: VerifySeverity::Error,
                    path: Some(path.clone()),
                    message: format!("segment open failed: {err}"),
                });
            }
        }
    }

    let manifest_path = layout.manifest_path();
    if manifest_path.exists() {
        match fs::read_to_string(&manifest_path) {
            Ok(text) => match serde_json::from_str::<DatasetManifest>(&text) {
                Ok(manifest) => {
                    if manifest.record_count != healthy_records
                        && issues.iter().all(|i| i.severity != VerifySeverity::Error)
                    {
                        issues.push(VerifyIssue {
                            severity: VerifySeverity::Warning,
                            path: Some(manifest_path.clone()),
                            message: format!(
                                "manifest record_count {} != healthy decoded records {healthy_records}",
                                manifest.record_count
                            ),
                        });
                    }
                    for entry in &manifest.segments {
                        let expected = layout.segments_dir().join(&entry.file);
                        if !expected.exists() {
                            issues.push(VerifyIssue {
                                severity: VerifySeverity::Error,
                                path: Some(expected),
                                message: format!(
                                    "manifest references missing segment `{}`",
                                    entry.file
                                ),
                            });
                        }
                    }
                    for (name, _, _) in &opened {
                        if !manifest.segments.iter().any(|s| s.file == *name) {
                            issues.push(VerifyIssue {
                                severity: VerifySeverity::Warning,
                                path: Some(layout.segments_dir().join(name)),
                                message: "segment file present but missing from manifest".into(),
                            });
                        }
                    }
                }
                Err(err) => {
                    issues.push(VerifyIssue {
                        severity: VerifySeverity::Error,
                        path: Some(manifest_path),
                        message: format!("manifest JSON invalid: {err}"),
                    });
                }
            },
            Err(source) => {
                issues.push(VerifyIssue {
                    severity: VerifySeverity::Error,
                    path: Some(manifest_path),
                    message: format!("failed to read manifest: {source}"),
                });
            }
        }
    } else {
        issues.push(VerifyIssue {
            severity: VerifySeverity::Warning,
            path: Some(manifest_path),
            message: "manifest.json missing (run `strata recover`)".into(),
        });
    }

    Ok(VerifyReport {
        root: layout.root,
        issues,
        healthy_segments,
        healthy_records,
    })
}
