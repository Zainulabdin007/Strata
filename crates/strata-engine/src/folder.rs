//! Per-file transactions and restart checkpoints around the production writer.
use super::*;
use std::io::Read;
use strata_input::{discover_inputs_with, DiscoverOptions};
use strata_storage::{dataset::ManifestSegment, DatasetManifest};
#[derive(Serialize, Deserialize)]
struct Completed {
    index: usize,
    path: PathBuf,
    size: u64,
    crc: u32,
}
#[derive(Serialize, Deserialize)]
struct Checkpoint {
    version: u32,
    profile: String,
    inputs: Vec<PathBuf>,
    files: Vec<Completed>,
    manifest: DatasetManifest,
    metrics: MetricsSnapshot,
}
fn fingerprint(path: &Path) -> Result<(u64, u32)> {
    let mut file = std::fs::File::open(path)?;
    let mut hash = crc32fast::Hasher::new();
    let mut size = 0;
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
        size += n as u64;
    }
    Ok((size, hash.finalize()))
}
fn publish(path: &Path, value: &impl Serialize) -> Result<()> {
    strata_storage::durability::publish_file(
        path,
        &serde_json::to_vec_pretty(value)?,
        DurabilityMode::Strict,
    )?;
    Ok(())
}
/// Ingest files in deterministic order, committing only completely parsed files.
/// Resume checks completed sources before reusing their records. Partial files are retried.
pub fn ingest(
    input: &Path,
    output: &Path,
    profile: &CompressionProfile,
    options: &IngestOptions,
    metrics: MetricsHandle,
) -> Result<MetricsSnapshot> {
    profile.validate()?;
    anyhow::ensure!((1..=32).contains(&options.workers), "workers must be 1..32");
    let mut profile = profile.clone();
    if !options.extensions.is_empty() {
        profile.input.extensions = options.extensions.clone();
    }
    let discovered = discover_inputs_with(
        input,
        &profile,
        &DiscoverOptions {
            include_globs: options.include_globs.clone(),
            exclude_globs: options.exclude_globs.clone(),
        },
    )?;
    let inputs = discovered
        .files
        .iter()
        .map(|p| p.canonicalize())
        .collect::<std::io::Result<Vec<_>>>()?;
    let signature = format!("{profile:?}");
    if output.exists() && !options.resume && std::fs::read_dir(output)?.next().is_some() {
        bail!("output must be new or empty; use --resume for a checkpointed ingest");
    }
    std::fs::create_dir_all(output)?;
    let _lock = strata_storage::writer_lock::lock_dataset(output)?;
    let checkpoint_path = output.join(".ingest-checkpoint.json");
    let mut state = if options.resume {
        let state: Checkpoint = serde_json::from_str(
            &read_small(&checkpoint_path).context("safe resume checkpoint required")?,
        )?;
        anyhow::ensure!(
            state.version == 1 && state.profile == signature && state.inputs == inputs,
            "resume inputs/profile differ from checkpoint"
        );
        for file in &state.files {
            anyhow::ensure!(
                inputs.get(file.index) == Some(&file.path),
                "invalid checkpoint file entry"
            );
            anyhow::ensure!(
                fingerprint(&file.path)? == (file.size, file.crc),
                "completed source changed: {}",
                file.path.display()
            );
        }
        let manifest_path = output.join("manifest.json");
        if manifest_path.exists() {
            let visible: DatasetManifest = serde_json::from_str(&read_small(&manifest_path)?)?;
            anyhow::ensure!(
                visible.schema == state.manifest.schema
                    && visible.key_fields == state.manifest.key_fields
                    && visible.compaction_generation == state.manifest.compaction_generation
                    && visible
                        .segments
                        .iter()
                        .all(|s| state.manifest.segments.contains(s)),
                "dataset changed outside ingest; refusing to roll back its manifest"
            );
        }
        // Validate references before any cleanup or publication.
        for seg in &state.manifest.segments {
            anyhow::ensure!(
                Path::new(&seg.file).components().count() == 1 && seg.file.starts_with('f'),
                "invalid checkpoint segment"
            );
            let path = output.join("segments").join(&seg.file);
            anyhow::ensure!(path.is_file(), "checkpoint segment missing");
            strata_storage::SegmentReader::open_buffered(strata_storage::SegmentPath::new(path))?;
        }
        metrics.with_mut(|m| m.restore(&state.metrics));
        publish(&output.join("manifest.json"), &state.manifest)?;
        state
    } else {
        metrics.set_total_files(inputs.len() as u64);
        let schema = if profile.key_fields().is_empty() {
            profile.schema()?
        } else {
            strata_storage::ensure_tombstone_field(profile.schema()?)?
        };
        let state = Checkpoint {
            version: 1,
            profile: signature,
            inputs: inputs.clone(),
            files: Vec::new(),
            manifest: DatasetManifest {
                manifest_version: DatasetManifest::VERSION,
                format_version: strata_format::FORMAT_VERSION,
                schema,
                block_row_count: profile.block.row_count,
                segment_target_size_bytes: profile.segment.target_size_bytes,
                segments: Vec::new(),
                record_count: 0,
                key_fields: profile.key_fields().to_vec(),
                compaction_generation: 0,
            },
            metrics: metrics.snapshot(),
        };
        publish(&checkpoint_path, &state)?;
        state
    };
    std::fs::create_dir_all(output.join("segments"))?;
    let stage = output.join(".ingest-current");
    let mut errors = Vec::new();
    for (index, file) in inputs.iter().enumerate() {
        if state.files.iter().any(|f| f.index == index) {
            continue;
        }
        if stage.exists() {
            std::fs::remove_dir_all(&stage)?;
        }
        // A crash can move a file's segments before publishing its checkpoint.
        // Only this uncommitted input index may be retried/replaced.
        let prefix = format!("f{index:08}-");
        for entry in std::fs::read_dir(output.join("segments"))? {
            let entry = entry?;
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                std::fs::remove_file(entry.path())?;
            }
        }
        let before = fingerprint(file)?;
        let measured = ingest_file(file, &stage, &profile, options, metrics.clone())
            .with_context(|| format!("input file {}", file.display()));
        let manifest = match measured {
            Ok(m) => m,
            Err(e) => {
                metrics.with_mut(|m| m.restore(&state.metrics));
                metrics.add_error();
                errors.push(json!({"file":file,"error":format!("{e:#}"),"committed":false}));
                publish(&output.join("ingest-errors.json"), &errors)?;
                if options.continue_on_error {
                    continue;
                }
                return Err(e);
            }
        };
        anyhow::ensure!(
            fingerprint(file)? == before,
            "source changed during ingest: {}",
            file.display()
        );
        for segment in manifest.segments {
            let name = format!("f{index:08}-{}", segment.file);
            std::fs::rename(
                stage.join("segments").join(&segment.file),
                output.join("segments").join(&name),
            )?;
            std::fs::File::open(output.join("segments").join(&name))?.sync_all()?;
            state.manifest.segments.push(ManifestSegment {
                file: name,
                ..segment
            });
        }
        // Sync moved directory entries before publishing a checkpoint that references them.
        std::fs::File::open(output.join("segments"))?.sync_all()?;
        state.manifest.segments.sort_by(|a, b| a.file.cmp(&b.file));
        state.manifest.record_count = state.manifest.segments.iter().map(|s| s.record_count).sum();
        state.files.push(Completed {
            index,
            path: file.clone(),
            size: before.0,
            crc: before.1,
        });
        state.metrics = metrics.snapshot();
        // Checkpoint is authoritative after a crash; manifest may lag it but never lead it.
        publish(&checkpoint_path, &state)?;
        publish(&output.join("manifest.json"), &state.manifest)?;
        let actual = storage_bytes(output)?;
        metrics.with_mut(|m| m.set_physical_bytes(actual));
        state.metrics = metrics.snapshot();
        publish(&checkpoint_path, &state)?;
        std::fs::remove_dir_all(&stage)?;
    }
    if !errors.is_empty() {
        bail!(
            "{} input files failed; see ingest-errors.json; successful files are committed",
            errors.len()
        );
    }
    let actual = storage_bytes(output)?;
    metrics.with_mut(|m| m.set_physical_bytes(actual));
    Ok(metrics.finish())
}
