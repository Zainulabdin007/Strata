//! Crash recovery: truncate incomplete tails and rebuild metadata.

use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use strata_core::{DurabilityMode, Schema};
use strata_format::{
    BlockId, BlockMeta, FormatVersion, SegmentFooter, SegmentHeader, SegmentId, SegmentMeta,
    FOOTER_MAGIC, FORMAT_VERSION, SEGMENT_MAGIC,
};

use crate::block::{probe_block_frame, BlockReader};
use crate::dataset::{DatasetManifest, ManifestSegment};
use crate::durability::publish_file;
use crate::error::{Error, Result};
use crate::io::{read_file, write_bytes, write_string, write_u32, write_u64, BufCursor};
use crate::path::{DatasetLayout, SegmentPath};
use crate::segment::SegmentReader;

/// Outcome of recovering one segment file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentRecovery {
    /// Segment path.
    pub path: PathBuf,
    /// Whether the file was modified.
    pub repaired: bool,
    /// Records retained after recovery.
    pub record_count: u64,
    /// Blocks retained after recovery.
    pub block_count: u32,
    /// Human summary.
    pub message: String,
}

/// Outcome of recovering a dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    /// Dataset root.
    pub root: PathBuf,
    /// Per-segment results.
    pub segments: Vec<SegmentRecovery>,
    /// Whether manifest.json was rewritten.
    pub manifest_rebuilt: bool,
    /// Total recoverable records.
    pub record_count: u64,
}

impl RecoveryReport {
    /// CLI summary.
    pub fn to_cli_summary(&self) -> String {
        let mut out = format!(
            "strata recover: {}\n  segments={}\n  records={}\n  manifest_rebuilt={}\n",
            self.root.display(),
            self.segments.len(),
            self.record_count,
            self.manifest_rebuilt
        );
        for seg in &self.segments {
            out.push_str(&format!(
                "  - {} repaired={} blocks={} records={} ({})\n",
                seg.path.file_name().and_then(|s| s.to_str()).unwrap_or("?"),
                seg.repaired,
                seg.block_count,
                seg.record_count,
                seg.message
            ));
        }
        out
    }
}

/// Recovers a dataset after a crash: salvage block prefixes, rebuild metadata.
pub fn recover_dataset(root: impl AsRef<Path>) -> Result<RecoveryReport> {
    let layout = DatasetLayout::new(root.as_ref());
    layout.ensure_dirs()?;

    let previous = if layout.manifest_path().exists() {
        Some(
            serde_json::from_slice::<DatasetManifest>(&crate::io::read_file(
                &layout.manifest_path(),
            )?)
            .map_err(|e| {
                Error::corrupt(
                    layout.manifest_path(),
                    format!("cannot safely recover dataset metadata: {e}"),
                )
            })?,
        )
    } else {
        None
    };
    let seg_dir = layout.segments_dir();
    let mut files: Vec<PathBuf> = fs::read_dir(&seg_dir)
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
        .collect();
    files.sort();

    let mut segment_results = Vec::new();
    let mut manifest_segments = Vec::new();
    let mut schema: Option<Schema> = None;
    let mut total_records = 0u64;

    for path in files {
        let result = recover_segment_file(&path)?;
        if result.block_count > 0 {
            let reader = SegmentReader::open(SegmentPath::new(&path))?;
            if schema.as_ref().is_some_and(|s| s != reader.schema())
                || previous
                    .as_ref()
                    .is_some_and(|m| &m.schema != reader.schema())
            {
                return Err(Error::corrupt(&path, "recovery schema mismatch"));
            }
            if schema.is_none() {
                schema = Some(reader.schema().clone());
            }
            total_records += result.record_count;
            manifest_segments.push(ManifestSegment {
                file: path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown.strata")
                    .to_string(),
                segment_id: reader.meta().header.segment_id.as_str().to_string(),
                record_count: result.record_count,
                block_count: result.block_count,
            });
        } else {
            // Remove empty / unsalvageable crash stubs.
            let _ = fs::remove_file(&path);
        }
        segment_results.push(result);
    }

    let schema = schema.ok_or_else(|| Error::InvalidState {
        message: "no recoverable segments found".into(),
    })?;

    let manifest = DatasetManifest {
        manifest_version: DatasetManifest::VERSION,
        format_version: FORMAT_VERSION,
        schema,
        block_row_count: previous.as_ref().map_or(65_536, |m| m.block_row_count),
        segment_target_size_bytes: previous
            .as_ref()
            .map_or(256 * 1024 * 1024, |m| m.segment_target_size_bytes),
        segments: manifest_segments,
        record_count: total_records,
        key_fields: previous
            .as_ref()
            .map_or_else(Vec::new, |m| m.key_fields.clone()),
        compaction_generation: previous.as_ref().map_or(0, |m| m.compaction_generation),
    };
    let text = serde_json::to_string_pretty(&manifest).map_err(|e| Error::InvalidState {
        message: format!("failed to serialize recovered manifest: {e}"),
    })?;
    publish_file(
        &layout.manifest_path(),
        text.as_bytes(),
        DurabilityMode::Durable,
    )?;

    Ok(RecoveryReport {
        root: layout.root,
        segments: segment_results,
        manifest_rebuilt: true,
        record_count: total_records,
    })
}

/// Repairs a single segment file in place when needed.
pub fn recover_segment_file(path: &Path) -> Result<SegmentRecovery> {
    let bytes = read_file(path)?;
    if bytes.len() < SEGMENT_MAGIC.len() + 16 {
        return Ok(SegmentRecovery {
            path: path.to_path_buf(),
            repaired: false,
            record_count: 0,
            block_count: 0,
            message: "file too short; ignored".into(),
        });
    }

    if SegmentReader::open(SegmentPath::new(path)).is_ok() {
        let reader = SegmentReader::open(SegmentPath::new(path))?;
        let mut records = 0u64;
        for block in 0..reader.block_count() {
            records += reader.read_block(block)?.len() as u64;
        }
        return Ok(SegmentRecovery {
            path: path.to_path_buf(),
            repaired: false,
            record_count: records,
            block_count: reader.block_count() as u32,
            message: "already valid".into(),
        });
    }

    let (schema, segment_id, header_end, counts_offset) = parse_header_prefix(&bytes, path)?;
    let scanned = scan_blocks(&bytes, header_end, &schema, path)?;
    if scanned.is_empty() {
        return Ok(SegmentRecovery {
            path: path.to_path_buf(),
            repaired: true,
            record_count: 0,
            block_count: 0,
            message: "no intact blocks; discarded incomplete segment".into(),
        });
    }

    rewrite_segment(
        path,
        &bytes[..header_end],
        counts_offset,
        &schema,
        &segment_id,
        &scanned,
    )?;

    let reader = SegmentReader::open(SegmentPath::new(path))?;
    let records = reader.read_all_records()?;
    Ok(SegmentRecovery {
        path: path.to_path_buf(),
        repaired: true,
        record_count: records.len() as u64,
        block_count: reader.block_count() as u32,
        message: format!(
            "rebuilt commit from {} intact block(s); truncated incomplete tail",
            scanned.len()
        ),
    })
}

struct ScannedBlock {
    meta: BlockMeta,
    bytes: Vec<u8>,
}

fn parse_header_prefix(bytes: &[u8], path: &Path) -> Result<(Schema, SegmentId, usize, u64)> {
    let mut cur = BufCursor::new(bytes, path);
    let magic = cur.read_exact(8)?;
    if magic != SEGMENT_MAGIC {
        return Err(Error::corrupt(path, "invalid segment magic"));
    }
    let major = cur.read_u16()?;
    let minor = cur.read_u16()?;
    let version = FormatVersion { major, minor };
    if !FORMAT_VERSION.is_compatible_with(version) {
        return Err(Error::UnsupportedVersion {
            found_major: major,
            found_minor: minor,
            supported_major: FORMAT_VERSION.major,
            supported_minor: FORMAT_VERSION.minor,
        });
    }
    let _flags = cur.read_u32()?;
    let schema_len = cur.read_u32()? as usize;
    let schema_bytes = cur.read_exact(schema_len)?;
    let schema: Schema = serde_json::from_slice(schema_bytes)
        .map_err(|e| Error::corrupt(path, format!("invalid schema JSON: {e}")))?;
    let segment_id = SegmentId::new(cur.read_string()?);
    let counts_offset = cur.position() as u64;
    let _record_count = cur.read_u64()?;
    let _block_count = cur.read_u32()?;
    let _reserved = cur.read_u32()?;
    Ok((schema, segment_id, cur.position(), counts_offset))
}

fn scan_blocks(
    bytes: &[u8],
    header_end: usize,
    schema: &Schema,
    path: &Path,
) -> Result<Vec<ScannedBlock>> {
    let mut offset = header_end;
    let mut out = Vec::new();
    let mut block_id = 0u32;
    while offset < bytes.len() {
        match probe_block_frame(&bytes[offset..]) {
            Some(len) => {
                let frame = &bytes[offset..offset + len];
                match BlockReader::new().read_columns(schema, frame, path) {
                    Ok((mut meta, _)) => {
                        meta.id = BlockId(block_id);
                        meta.file_offset = offset as u64;
                        meta.block_length = len as u64;
                        out.push(ScannedBlock {
                            meta,
                            bytes: frame.to_vec(),
                        });
                        offset += len;
                        block_id += 1;
                    }
                    Err(_) => break,
                }
            }
            None => break,
        }
    }
    Ok(out)
}

fn rewrite_segment(
    path: &Path,
    header_bytes: &[u8],
    counts_offset: u64,
    schema: &Schema,
    segment_id: &SegmentId,
    blocks: &[ScannedBlock],
) -> Result<()> {
    let tmp = path.with_extension("recover-tmp");
    {
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp)
            .map_err(|source| Error::Io {
                path: tmp.clone(),
                source,
            })?;
        let mut header_bytes = header_bytes.to_vec();
        header_bytes[10..12].copy_from_slice(&FORMAT_VERSION.minor.to_le_bytes());
        write_bytes(&mut file, &tmp, &header_bytes)?;

        let mut meta = SegmentMeta {
            header: SegmentHeader {
                version: FORMAT_VERSION,
                schema_id: schema.id.clone(),
                segment_id: segment_id.clone(),
                record_count: 0,
                block_count: 0,
            },
            blocks: Vec::new(),
            footer: SegmentFooter {
                block_directory_offset: 0,
                logical_bytes: 0,
                encoded_bytes: 0,
                checksum: None,
            },
        };

        let mut hasher = crc32fast::Hasher::new();
        let mut offset = header_bytes.len() as u64;
        for (i, block) in blocks.iter().enumerate() {
            let mut bm = block.meta.clone();
            bm.id = BlockId(i as u32);
            bm.file_offset = offset;
            bm.block_length = block.bytes.len() as u64;
            write_bytes(&mut file, &tmp, &block.bytes)?;
            hasher.update(&block.bytes);
            meta.footer.logical_bytes += bm.logical_bytes;
            meta.footer.encoded_bytes += bm.block_length;
            meta.header.record_count += u64::from(bm.row_count);
            meta.header.block_count += 1;
            offset += bm.block_length;
            meta.blocks.push(bm);
        }

        let directory_offset = offset;
        meta.footer.block_directory_offset = directory_offset;
        write_directory(&mut file, &tmp, &meta)?;
        let _ = hasher.finalize();

        file.seek(SeekFrom::Start(counts_offset))
            .map_err(|source| Error::Io {
                path: tmp.clone(),
                source,
            })?;
        write_u64(&mut file, &tmp, meta.header.record_count)?;
        write_u32(&mut file, &tmp, meta.header.block_count)?;

        file.seek(SeekFrom::End(0)).map_err(|source| Error::Io {
            path: tmp.clone(),
            source,
        })?;
        write_u64(&mut file, &tmp, directory_offset)?;
        write_u64(&mut file, &tmp, meta.footer.logical_bytes)?;
        write_u64(&mut file, &tmp, meta.footer.encoded_bytes)?;
        let checksum = crate::io::checksum_prefix(&mut file, &tmp)?;
        write_u32(&mut file, &tmp, checksum)?;
        write_bytes(&mut file, &tmp, FOOTER_MAGIC)?;
        file.flush().map_err(|source| Error::Io {
            path: tmp.clone(),
            source,
        })?;
        file.sync_all().map_err(|source| Error::Io {
            path: tmp.clone(),
            source,
        })?;
    }

    fs::rename(&tmp, path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

fn write_directory(file: &mut File, path: &Path, meta: &SegmentMeta) -> Result<()> {
    for block in &meta.blocks {
        write_u32(file, path, block.id.0)?;
        write_u64(file, path, block.file_offset)?;
        write_u64(file, path, block.block_length)?;
        write_u32(file, path, block.row_count)?;
        write_u32(file, path, block.columns.len() as u32)?;
        for col in &block.columns {
            write_string(file, path, &col.field_name)?;
            file.write_all(&[col.encoding.as_u8()])
                .map_err(|source| Error::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
            write_u32(file, path, col.data_rel_offset)?;
            write_u32(file, path, col.data_len)?;
            write_u64(file, path, col.logical_bytes)?;
        }
    }
    Ok(())
}
