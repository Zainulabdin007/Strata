//! Strata command-line interface.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use strata_analyze::{format_human, format_json, AnalyzeOptions, Profiler};
use strata_codec::{resolve_profile_plans, CompressionCodec, EncodingPlan};
use strata_core::{load_profile, CompressionProfile};
use strata_input::{discover_inputs, open_parser, ParseOptions, RecordParser};
use strata_metrics::{MetricsHandle, MetricsReport};
use strata_optimize::{
    format_human as format_optimize_human, format_json as format_optimize_json, optimize_dataset,
    OptimizationConstraints, OptimizationGoal, OptimizeOptions,
};
use strata_query::{execute_with_options, format_explain, parse_query, QueryOptions};
use strata_storage::{
    compact_dataset, rebuild_indexes, recover_dataset, verify_dataset, CompactOptions,
    DatasetIndexes, DatasetLayout, DatasetReader, SegmentReadMode,
};

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Dbn(args) => run_dbn(args),
        Commands::Version => {
            println!("strata {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Commands::Ingest(args) => run_ingest(args),
        Commands::Analyze(args) => run_analyze(args),
        Commands::Optimize(args) => run_optimize(args),
        Commands::Inspect(args) => run_inspect(args),
        Commands::Verify(args) => run_verify(args),
        Commands::Recover(args) => run_recover(args),
        Commands::Index { command } => run_index(command),
        Commands::Compact(args) => run_compact(args),
        Commands::Query(args) => run_query(args),
        Commands::Profile { command } => run_profile(command),
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "strata",
    about = "Compression-first storage engine for structured event data",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Inspect native DBN MBO headers and observed records; emit a lossless schema/profile.
    Dbn(DbnArgs),
    /// Print the Strata version.
    Version,
    /// Ingest a file or directory into a Strata dataset.
    Ingest(IngestArgs),
    /// Profile column statistics for a file or directory (read-only).
    Analyze(AnalyzeArgs),
    /// Automatically choose per-column encoding pipelines from sample benches.
    Optimize(OptimizeArgs),
    /// Inspect a dataset layout.
    Inspect(InspectArgs),
    /// Verify segment/block integrity and manifest consistency.
    Verify(VerifyArgs),
    /// Recover from incomplete tails and rebuild manifest metadata.
    Recover(RecoverArgs),
    /// Build or rebuild dataset indexes.
    Index {
        #[command(subcommand)]
        command: IndexCmd,
    },
    /// Compact a keyed dataset (fold versions/tombstones into new sealed segments).
    Compact(CompactArgs),
    /// Run a typed Strata query (not SQL). See QUERY.md.
    Query(QueryArgs),
    /// Inspect or validate compression profiles.
    Profile {
        #[command(subcommand)]
        command: ProfileCmd,
    },
}

#[derive(Debug, Parser)]
struct IngestArgs {
    /// Input file or directory.
    input: PathBuf,

    /// Path to a TOML compression profile.
    #[arg(long)]
    profile: PathBuf,

    /// Output dataset directory.
    #[arg(long)]
    output: PathBuf,

    /// Encode/compress worker threads (bounded parallel pipeline).
    #[arg(long, default_value_t = default_workers())]
    workers: usize,

    /// Capacity of the parser → encode queue (blocks).
    #[arg(long)]
    parse_queue: Option<usize>,

    /// Capacity of the encode → writer queue (blocks).
    #[arg(long)]
    encode_queue: Option<usize>,

    /// Durability mode: fast | durable | strict (overrides profile).
    #[arg(long)]
    durability: Option<String>,

    /// For durable mode: fsync every N blocks (overrides profile).
    #[arg(long)]
    sync_every_blocks: Option<u32>,

    /// Resume verified committed files; interrupted files are retried without duplicate rows.
    #[arg(long, default_value_t = false)]
    resume: bool,

    /// Continue past per-file errors; write ingest-errors.json.
    #[arg(long, default_value_t = false)]
    continue_on_error: bool,

    /// Include glob(s) relative to a directory input (repeatable).
    #[arg(long = "include")]
    include_globs: Vec<String>,

    /// Exclude glob(s) relative to a directory input (repeatable).
    #[arg(long = "exclude")]
    exclude_globs: Vec<String>,
    /// Override extensions (comma separated).
    #[arg(long, value_delimiter = ',')]
    extensions: Vec<String>,
}

fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 32)
}

#[derive(Debug, Parser)]
struct AnalyzeArgs {
    /// Input file or directory to analyze (not modified).
    input: PathBuf,

    /// Path to a TOML compression profile (schema + input format).
    #[arg(long)]
    profile: PathBuf,

    /// Output format: `text` (default) or `json`.
    #[arg(long, default_value = "text", value_parser = ["text", "json"])]
    format: String,

    /// Block window size for local range statistics (defaults to profile block.row_count).
    #[arg(long)]
    block_rows: Option<u32>,
}

#[derive(Debug, Parser)]
struct OptimizeArgs {
    /// Input file or directory to optimize against (not modified).
    input: PathBuf,

    /// Path to a TOML compression profile (schema + input format).
    #[arg(long)]
    profile: PathBuf,

    /// Write the winning CompressionProfile TOML to this path.
    #[arg(long)]
    output: PathBuf,

    /// Optimization goal: max_compression | max_ingest | max_decode | balanced.
    #[arg(long, default_value = "balanced")]
    goal: String,

    /// Minimum encode throughput constraint (MB/s).
    #[arg(long)]
    min_ingest_mb_s: Option<f64>,

    /// Minimum decode throughput constraint (MB/s).
    #[arg(long)]
    min_decode_mb_s: Option<f64>,

    /// Maximum estimated temporary encode memory (bytes).
    #[arg(long)]
    max_memory: Option<u64>,

    /// Optional target compression ratio (logical / encoded).
    #[arg(long)]
    target_ratio: Option<f64>,

    /// Sample rows for encode/decode benches (defaults to min(block_rows, 4096)).
    #[arg(long)]
    sample_rows: Option<u32>,

    /// Block window for profiler local ranges.
    #[arg(long)]
    block_rows: Option<u32>,

    /// Also print machine-readable JSON to stdout (`text` still printed unless `json`).
    #[arg(long, default_value = "text", value_parser = ["text", "json"])]
    format: String,
}

#[derive(Debug, Parser)]
struct InspectArgs {
    /// Dataset directory to inspect.
    #[arg(long)]
    dataset: PathBuf,
}

#[derive(Debug, Parser)]
struct VerifyArgs {
    /// Dataset directory to verify.
    dataset: PathBuf,
}

#[derive(Debug, Parser)]
struct RecoverArgs {
    /// Dataset directory to recover.
    dataset: PathBuf,
}

#[derive(Debug, Subcommand)]
enum IndexCmd {
    /// Rebuild sequence, time, categorical, and block min/max indexes.
    Rebuild {
        /// Dataset directory.
        dataset: PathBuf,
    },
}

#[derive(Debug, Parser)]
struct CompactArgs {
    /// Keyed dataset directory to compact.
    dataset: PathBuf,
}

#[derive(Debug, Parser)]
struct QueryArgs {
    /// Dataset directory.
    #[arg(long)]
    dataset: PathBuf,

    /// Inline Strata query text.
    #[arg(long)]
    query: Option<String>,

    /// Path to a file containing Strata query text.
    #[arg(long)]
    file: Option<PathBuf>,

    /// Print plan + stats.
    #[arg(long, default_value_t = false)]
    explain: bool,

    /// Disable index block skipping.
    #[arg(long, default_value_t = false)]
    full_scan: bool,

    /// Output format: `text` or `json`.
    #[arg(long, default_value = "text", value_parser = ["text", "json"])]
    format: String,

    /// Max rows to print in text mode (0 = unlimited).
    #[arg(long, default_value_t = 50)]
    max_print: u64,

    /// Sealed segment byte source: auto | buffered | mmap.
    #[arg(long, default_value = "auto", value_parser = ["auto", "buffered", "mmap"])]
    segment_read: String,
}

#[derive(Debug, Subcommand)]
enum ProfileCmd {
    /// Validate a compression profile (schema + encoding pipelines).
    Validate {
        /// Path to a TOML compression profile.
        profile: PathBuf,
    },
    /// Show resolved encoding plans for a compression profile.
    Show {
        /// Path to a TOML compression profile.
        profile: PathBuf,
    },
}

fn load_profile_with_plans(
    path: &PathBuf,
) -> Result<(CompressionProfile, HashMap<String, EncodingPlan>)> {
    let profile =
        load_profile(path).with_context(|| format!("failed to load profile {}", path.display()))?;
    let plans = resolve_profile_plans(&profile)
        .with_context(|| format!("invalid encoding pipeline(s) in profile {}", path.display()))?;
    Ok((profile, plans))
}

fn run_profile(cmd: ProfileCmd) -> Result<()> {
    match cmd {
        ProfileCmd::Validate { profile } => {
            let (profile, plans) = load_profile_with_plans(&profile)?;
            println!("profile OK: {}", profile.name);
            println!("  schema={}", profile.schema.id);
            println!("  input_format={}", profile.input_format.as_str());
            println!("  columns={}", profile.columns.len());
            println!("  encoding_plans={}", plans.len());
            Ok(())
        }
        ProfileCmd::Show { profile } => {
            let (profile, plans) = load_profile_with_plans(&profile)?;
            println!("name={}", profile.name);
            println!("schema.id={}", profile.schema.id);
            if let Some(desc) = &profile.schema.description {
                println!("schema.description={desc}");
            }
            println!("input_format={}", profile.input_format.as_str());
            println!("block.row_count={}", profile.block.row_count);
            println!(
                "segment.target_size_bytes={}",
                profile.segment.target_size_bytes
            );
            println!(
                "compression.codec={}",
                profile
                    .compression
                    .effective_codec()
                    .unwrap_or_default()
                    .as_str()
            );
            if let Some(level) = profile.compression.level {
                println!("compression.level={level}");
            }
            println!();
            println!("columns:");
            for column in &profile.columns {
                let declared = column.encoding.display();
                let plan = plans.get(&column.name).expect("plan for every column");
                let resolved = format_plan(plan);
                println!(
                    "  - {name}: type={ty} nullable={nullable}",
                    name = column.name,
                    ty = column.logical_type,
                    nullable = column.nullable,
                );
                println!("      encoding.declared = {declared}");
                println!("      encoding.resolved = {resolved}");
            }
            Ok(())
        }
    }
}

fn format_plan(plan: &EncodingPlan) -> String {
    plan.steps
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" → ")
}

fn run_analyze(args: AnalyzeArgs) -> Result<()> {
    let profile = load_profile(&args.profile)
        .with_context(|| format!("failed to load profile {}", args.profile.display()))?;
    // Validate encodings early so analyze fails clearly on bad profiles.
    resolve_profile_plans(&profile).with_context(|| {
        format!(
            "invalid encoding pipeline(s) in profile {}",
            args.profile.display()
        )
    })?;

    if !profile.input_format.is_supported() {
        bail!(
            "input format `{}` is not implemented yet (supported: csv, jsonl)",
            profile.input_format.as_str()
        );
    }

    let discovered = discover_inputs(&args.input, &profile)
        .with_context(|| format!("failed to discover inputs under {}", args.input.display()))?;

    let schema = profile
        .schema()
        .with_context(|| format!("invalid schema in profile {}", args.profile.display()))?;

    let block_rows = args.block_rows.unwrap_or(profile.block.row_count);
    let options = AnalyzeOptions { block_rows };
    let mut profiler = Profiler::new(args.input.display().to_string(), &schema, &options);
    profiler.set_file_count(discovered.files.len() as u64);

    let mut parser = open_parser(&profile)?;
    let mut parse_options = ParseOptions {
        max_batch_rows: Some(65_536),
    };
    eprintln!("analysis scope: first up to 65536 observed records (bounded sample)");

    for file in &discovered.files {
        if parse_options.max_batch_rows == Some(0) {
            break;
        }
        let batch = RecordParser::parse_file(parser.as_mut(), file, &schema, &parse_options)
            .with_context(|| format!("failed to parse {}", file.display()))?;
        parse_options.max_batch_rows = parse_options
            .max_batch_rows
            .map(|n| n.saturating_sub(batch.len()));
        profiler.push_records(&schema, &batch.records);
    }

    let report = profiler.finish();
    match args.format.as_str() {
        "json" => println!("{}", format_json(&report)?),
        _ => print!("{}", format_human(&report)),
    }
    Ok(())
}

fn run_optimize(args: OptimizeArgs) -> Result<()> {
    let goal = OptimizationGoal::parse(&args.goal).ok_or_else(|| {
        anyhow::anyhow!(
            "unknown goal `{}`; expected max_compression|max_ingest|max_decode|balanced",
            args.goal
        )
    })?;

    let profile = load_profile(&args.profile)
        .with_context(|| format!("failed to load profile {}", args.profile.display()))?;
    resolve_profile_plans(&profile).with_context(|| {
        format!(
            "invalid encoding pipeline(s) in profile {}",
            args.profile.display()
        )
    })?;

    if !profile.input_format.is_supported() {
        bail!(
            "input format `{}` is not implemented yet (supported: csv, jsonl)",
            profile.input_format.as_str()
        );
    }

    let discovered = discover_inputs(&args.input, &profile)
        .with_context(|| format!("failed to discover inputs under {}", args.input.display()))?;

    let schema = profile
        .schema()
        .with_context(|| format!("invalid schema in profile {}", args.profile.display()))?;

    let mut parser = open_parser(&profile)?;
    let mut parse_options = ParseOptions {
        max_batch_rows: Some(65_536),
    };
    eprintln!("analysis scope: first up to 65536 observed records (bounded sample)");
    let mut records = Vec::new();
    for file in &discovered.files {
        if parse_options.max_batch_rows == Some(0) {
            break;
        }
        let batch = RecordParser::parse_file(parser.as_mut(), file, &schema, &parse_options)
            .with_context(|| format!("failed to parse {}", file.display()))?;
        parse_options.max_batch_rows = parse_options
            .max_batch_rows
            .map(|n| n.saturating_sub(batch.len()));
        records.extend(batch.records);
    }

    let options = OptimizeOptions {
        goal,
        constraints: OptimizationConstraints {
            min_ingest_mb_s: args.min_ingest_mb_s,
            min_decode_mb_s: args.min_decode_mb_s,
            max_memory_bytes: args.max_memory,
            target_compression_ratio: args.target_ratio,
        },
        sample_rows: args.sample_rows,
        block_rows: args.block_rows,
        ..OptimizeOptions::default()
    };

    let report = optimize_dataset(
        args.input.display().to_string(),
        &profile,
        &records,
        discovered.files.len() as u64,
        &options,
    )
    .with_context(|| "optimization failed")?;

    if let Some(parent) = args.output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    std::fs::write(&args.output, &report.profile_toml)
        .with_context(|| format!("failed to write {}", args.output.display()))?;

    match args.format.as_str() {
        "json" => println!("{}", format_optimize_json(&report)?),
        _ => {
            print!("{}", format_optimize_human(&report));
            println!("wrote optimized profile: {}", args.output.display());
        }
    }
    Ok(())
}

fn run_ingest(args: IngestArgs) -> Result<()> {
    let (profile, _) = load_profile_with_plans(&args.profile)?;
    let metrics = MetricsHandle::new();
    let progress = metrics.clone();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let observer = std::thread::spawn(move || {
        while stop_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .is_err()
        {
            eprintln!("{}", progress.snapshot().to_cli_live());
        }
    });
    let result = strata_engine::ingest(
        &args.input,
        &args.output,
        &profile,
        &strata_engine::IngestOptions {
            workers: args.workers,
            parse_queue: args.parse_queue,
            encode_queue: args.encode_queue,
            durability: args.durability,
            sync_every_blocks: args.sync_every_blocks,
            resume: args.resume,
            continue_on_error: args.continue_on_error,
            include_globs: args.include_globs,
            exclude_globs: args.exclude_globs,
            extensions: args.extensions,
        },
        metrics,
    );
    let _ = stop_tx.send(());
    let _ = observer.join();
    let snapshot = result?;
    std::fs::copy(&args.profile, args.output.join("profile.toml"))?;
    strata_engine::save_json(&args.output.join("ingest-metrics.json"), &snapshot)?;
    println!(
        "{}",
        MetricsReport::new("ingest", snapshot).to_cli_summary()
    );
    Ok(())
}

fn run_verify(args: VerifyArgs) -> Result<()> {
    let report = verify_dataset(&args.dataset)
        .with_context(|| format!("verify failed for {}", args.dataset.display()))?;
    print!("{}", report.to_cli_summary());
    if !report.ok() {
        bail!("dataset verification found errors");
    }
    Ok(())
}

fn run_recover(args: RecoverArgs) -> Result<()> {
    let report = recover_dataset(&args.dataset)
        .with_context(|| format!("recover failed for {}", args.dataset.display()))?;
    print!("{}", report.to_cli_summary());
    let verify = verify_dataset(&args.dataset)
        .with_context(|| format!("post-recover verify failed for {}", args.dataset.display()))?;
    print!("{}", verify.to_cli_summary());
    if !verify.ok() {
        bail!("dataset still has errors after recover");
    }
    Ok(())
}

fn run_index(cmd: IndexCmd) -> Result<()> {
    match cmd {
        IndexCmd::Rebuild { dataset } => {
            let report = rebuild_indexes(&dataset)
                .with_context(|| format!("index rebuild failed for {}", dataset.display()))?;
            print!("{}", report.to_cli_summary());
            let indexes = DatasetIndexes::open(&dataset).with_context(|| {
                format!("failed to open rebuilt indexes for {}", dataset.display())
            })?;
            println!(
                "  catalog: schema={} artifacts={}",
                indexes.catalog().schema_id,
                indexes.catalog().artifacts.len()
            );
            Ok(())
        }
    }
}

fn run_compact(args: CompactArgs) -> Result<()> {
    let reader = DatasetReader::open(&args.dataset)
        .with_context(|| format!("failed to open {}", args.dataset.display()))?;
    if !reader.is_keyed() {
        bail!("dataset is append-only (no key_fields); compaction is not required or supported");
    }
    let before = reader.read_latest_records()?;
    let codec = CompressionCodec::None;
    let report = compact_dataset(
        &args.dataset,
        CompactOptions::default().with_compression(codec, None),
    )
    .with_context(|| format!("compact failed for {}", args.dataset.display()))?;
    print!("{}", report.to_cli_summary());
    let after = DatasetReader::open(&args.dataset)?.read_latest_records()?;
    if before.len() != after.len() {
        bail!(
            "logical record count changed after compact ({} → {})",
            before.len(),
            after.len()
        );
    }
    Ok(())
}

fn run_query(args: QueryArgs) -> Result<()> {
    let text = match (&args.query, &args.file) {
        (Some(q), None) => q.clone(),
        (None, Some(path)) => std::fs::read_to_string(path)
            .with_context(|| format!("failed to read query file {}", path.display()))?,
        (Some(_), Some(_)) => bail!("provide only one of --query or --file"),
        (None, None) => bail!("provide --query or --file (see QUERY.md)"),
    };

    let query = parse_query(&text).context("failed to parse Strata query")?;
    let segment_read_mode = match args.segment_read.as_str() {
        "buffered" => SegmentReadMode::Buffered,
        "mmap" => SegmentReadMode::Mmap,
        _ => SegmentReadMode::Auto,
    };
    let options = QueryOptions {
        use_indexes: !args.full_scan,
        force_full_scan: args.full_scan,
        segment_read_mode,
    };
    let mut stream = execute_with_options(&args.dataset, query, options)
        .with_context(|| format!("query failed for {}", args.dataset.display()))?;

    if args.explain {
        eprint!("{}", format_explain(stream.explain(), stream.stats()));
    }

    if args.format == "json" {
        let records = stream.collect_records()?;
        println!("{}", serde_json::to_string_pretty(&records)?);
        eprintln!(
            "strata query: rows_returned={} blocks_scanned={} blocks_skipped={}",
            stream.stats().rows_returned,
            stream.stats().blocks_scanned,
            stream.stats().blocks_skipped
        );
        return Ok(());
    }

    let mut printed = 0u64;
    let max_print = args.max_print;
    while let Some(batch) = stream.next_batch()? {
        for rec in &batch.records {
            if max_print > 0 && printed >= max_print {
                break;
            }
            let mut parts = Vec::new();
            for (k, v) in &rec.values {
                parts.push(format!("{k}={}", format_field_value(v)));
            }
            println!("{}", parts.join(" "));
            printed += 1;
        }
        if max_print > 0 && printed >= max_print {
            break;
        }
    }

    // Drain remaining for accurate stats when truncated print.
    if max_print > 0 && printed >= max_print {
        while stream.next_batch()?.is_some() {}
    }

    eprintln!(
        "strata query: rows_returned={} blocks_scanned={} blocks_skipped={}",
        stream.stats().rows_returned,
        stream.stats().blocks_scanned,
        stream.stats().blocks_skipped
    );
    Ok(())
}

fn format_field_value(v: &strata_core::FieldValue) -> String {
    use strata_core::FieldValue;
    match v {
        FieldValue::Null => "null".into(),
        FieldValue::Boolean(b) => b.to_string(),
        FieldValue::UnsignedInteger(n) => n.to_string(),
        FieldValue::SignedInteger(n) | FieldValue::Timestamp(n) | FieldValue::FixedDecimal(n) => {
            n.to_string()
        }
        FieldValue::CategoricalString(s) | FieldValue::Enum(s) => s.clone(),
        FieldValue::Binary(b) => format!("<{} bytes>", b.len()),
    }
}

fn run_inspect(args: InspectArgs) -> Result<()> {
    let layout = DatasetLayout::new(&args.dataset);
    if !layout.root.exists() {
        bail!("dataset does not exist: {}", args.dataset.display());
    }

    println!("dataset={}", layout.root.display());
    println!("segments_dir={}", layout.segments_dir().display());
    println!("manifest={}", layout.manifest_path().display());

    if layout.manifest_path().exists() {
        let reader = DatasetReader::open(&args.dataset)
            .with_context(|| format!("failed to open dataset {}", args.dataset.display()))?;
        let m = reader.manifest();
        println!("schema={}", m.schema.id.as_str());
        println!("record_count={}", m.record_count);
        println!("segment_count={}", m.segments.len());
        for (i, seg) in m.segments.iter().enumerate() {
            println!(
                "  segment[{i}] file={} records={} blocks={}",
                seg.file, seg.record_count, seg.block_count
            );
        }
        let indexes_dir = layout.indexes_dir();
        if indexes_dir.join("manifest.json").exists() {
            match DatasetIndexes::open(&args.dataset) {
                Ok(idx) => {
                    println!(
                        "indexes=present artifacts={}",
                        idx.catalog().artifacts.len()
                    );
                }
                Err(e) => println!("indexes=present but unreadable ({e})"),
            }
        } else {
            println!("indexes=absent (run `strata index rebuild`)");
        }
    } else {
        let segments_dir = layout.segments_dir();
        if segments_dir.is_dir() {
            let mut count = 0u64;
            for entry in std::fs::read_dir(&segments_dir)? {
                let entry = entry?;
                if entry.path().extension().and_then(|e| e.to_str()) == Some("strata") {
                    count += 1;
                }
            }
            println!("segment_files={count}");
        } else {
            println!("segment_files=0");
        }
    }

    Ok(())
}

#[derive(Debug, Parser)]
struct DbnArgs {
    /// File or directory containing DBN / DBN.zst sources.
    input: PathBuf,
    /// Directory for exact metadata copies, schema, profile and measured source statistics.
    #[arg(long)]
    out: PathBuf,
    /// Rows per file to inspect (0 means complete streaming scan).
    #[arg(long, default_value_t = 100_000)]
    records: u64,
}
fn run_dbn(args: DbnArgs) -> Result<()> {
    let base =
        strata_core::parse_profile_str(&strata_input::dbn::mbo_profile(false), "DBN defaults")?;
    let files = strata_input::discover_inputs(&args.input, &base)?.files;
    std::fs::create_dir_all(&args.out)?;
    let mut stats = Vec::new();
    let mut expected = None;
    for (i, path) in files.iter().enumerate() {
        let mut reader = strata_input::dbn::MboReader::open(path)?;
        let schema = reader.schema();
        if let Some(ref expected) = expected {
            anyhow::ensure!(
                &schema == expected,
                "incompatible DBN schemas across source files"
            );
        } else {
            expected = Some(schema.clone());
        }
        std::fs::write(
            args.out.join(format!("source-{i}.metadata.dbn")),
            reader.raw_metadata(),
        )?;
        let mut count = 0;
        while args.records == 0 || count < args.records {
            let n = if args.records == 0 {
                16384
            } else {
                (args.records - count).min(16384) as usize
            };
            let rows = reader.next_batch(n)?;
            if rows.is_empty() {
                break;
            }
            count += rows.len() as u64;
        }
        let observed = reader.stats();
        println!(
            "{}: {} records inspected, {} DBN bytes observed, complete={}",
            path.display(),
            observed.records,
            observed.decompressed_bytes_observed,
            observed.complete
        );
        std::fs::write(
            args.out.join("manual.toml"),
            strata_input::dbn::mbo_profile(observed.metadata["ts_out"].as_bool().unwrap_or(false)),
        )?;
        std::fs::write(
            args.out.join("schema.json"),
            serde_json::to_vec_pretty(&schema)?,
        )?;
        stats.push(observed);
    }
    std::fs::write(
        args.out.join("sources.json"),
        serde_json::to_vec_pretty(&stats)?,
    )?;
    Ok(())
}
