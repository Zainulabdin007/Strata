//! Strata serious benchmark suite.
//!
//! Heavy workloads are **not** run by `cargo test`. Invoke explicitly:
//!
//! ```bash
//! cargo run -p strata-bench --release -- --suite ingest --size 100k --workload market
//! ```

mod baselines;
mod config;
mod gen;
mod meta;
mod report;
mod size;
mod suites;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::Parser;
use strata_codec::CompressionCodec;
use strata_storage::SegmentReadMode;

use crate::config::BenchConfig;
use crate::gen::Workload;
use crate::size::BenchSize;
use crate::suites::{run_suite, Suite};

#[derive(Debug, Parser)]
#[command(
    name = "strata-bench",
    about = "Serious Strata benchmarks (ingest, compression, decode, query, recovery, compaction)"
)]
struct Args {
    /// Suites: ingest,compression,decode,query,recovery,compaction,compare,all
    #[arg(long, default_value = "ingest")]
    suite: String,

    /// Size: 100k | 1m | 10m | 100m
    #[arg(long, default_value = "100k")]
    size: String,

    /// Workload generator: market | telemetry | logs
    #[arg(long, default_value = "market")]
    workload: String,

    /// Deterministic dataset seed.
    #[arg(long, default_value_t = 42)]
    seed: u64,

    /// Ingest worker threads.
    #[arg(long, default_value_t = default_workers())]
    workers: usize,

    /// Block row count.
    #[arg(long, default_value_t = 65_536)]
    block_size: u32,

    /// Segment target size in bytes.
    #[arg(long, default_value_t = 256 * 1024 * 1024)]
    segment_size: u64,

    /// Compression codec: none | lz4 | zstd
    #[arg(long, default_value = "lz4")]
    codec: String,

    /// Optional codec level.
    #[arg(long)]
    codec_level: Option<i32>,

    /// Segment read mode for decode/query: auto | buffered | mmap
    #[arg(long, default_value = "auto")]
    segment_read: String,

    /// Directory for JSON + RESULTS.md (measured numbers only).
    #[arg(long, default_value = "benchmarks/strata-bench")]
    out: PathBuf,

    /// Scratch directory for temporary datasets.
    #[arg(long)]
    work_dir: Option<PathBuf>,
}

fn default_workers() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 32)
}

fn main() -> Result<()> {
    anyhow::ensure!(
        !cfg!(debug_assertions),
        "performance benchmarks require --release"
    );
    let args = Args::parse();
    let size =
        BenchSize::parse(&args.size).with_context(|| format!("unknown size `{}`", args.size))?;
    let workload = Workload::parse(&args.workload)
        .with_context(|| format!("unknown workload `{}`", args.workload))?;
    let codec = match args.codec.to_ascii_lowercase().as_str() {
        "none" => CompressionCodec::None,
        "lz4" => CompressionCodec::Lz4,
        "zstd" => CompressionCodec::Zstd,
        other => bail!("unknown codec `{other}`"),
    };
    let segment_read_mode = match args.segment_read.as_str() {
        "buffered" => SegmentReadMode::Buffered,
        "mmap" => SegmentReadMode::Mmap,
        _ => SegmentReadMode::Auto,
    };

    let suites = parse_suites(&args.suite)?;
    let work_dir = args.work_dir.unwrap_or_else(|| {
        std::env::temp_dir().join(format!("strata-bench-{}", std::process::id()))
    });
    std::fs::create_dir_all(&work_dir)?;
    std::fs::create_dir_all(&args.out)?;

    let config = BenchConfig {
        out_dir: args.out.clone(),
        work_dir,
        workload,
        size,
        seed: args.seed,
        workers: args.workers.max(1),
        block_size: args.block_size.max(1),
        segment_size_bytes: args.segment_size.max(1),
        codec,
        codec_level: args.codec_level,
        segment_read_mode,
    };

    println!(
        "strata-bench: suites={} workload={} size={} seed={} workers={} out={}",
        suites
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(","),
        workload.as_str(),
        size.as_str(),
        args.seed,
        config.workers,
        config.out_dir.display()
    );

    for suite in suites {
        println!("→ running {} …", suite.as_str());
        let report = run_suite(suite, &config)
            .with_context(|| format!("suite `{}` failed", suite.as_str()))?;
        let path = report.persist(&config.out_dir)?;
        println!(
            "  wrote {} (records_per_sec-ish metrics in JSON)",
            path.display()
        );
    }

    println!(
        "done. measured results appended to {}",
        config.out_dir.join("RESULTS.md").display()
    );
    Ok(())
}

fn parse_suites(s: &str) -> Result<Vec<Suite>> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("all") {
        return Ok(Suite::all().to_vec());
    }
    let mut out = Vec::new();
    for part in s.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let suite = Suite::parse(part).with_context(|| format!("unknown suite `{part}`"))?;
        out.push(suite);
    }
    if out.is_empty() {
        bail!("no suites selected");
    }
    Ok(out)
}
