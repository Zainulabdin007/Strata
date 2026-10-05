//! Measure lossless compression candidates for the bench market generator.
//!
//! ```bash
//! cargo run -p strata-bench --release --bin tune-market -- --records 100000
//! cargo run -p strata-bench --release --bin tune-market -- --records 1000000
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use clap::Parser;
use strata_bench::gen::{generate_chunk, Workload};
use strata_codec::{compressor_for, resolve_profile_plans, CompressionCodec};
use strata_core::{
    parse_profile_str, ColumnEncodingSpec, CompressionCodecName, CompressionProfile,
    DurabilityMode, IntegerWidth, Record, TimestampUnit,
};
use strata_metrics::MetricsHandle;
use strata_storage::{
    write_records_parallel, DatasetReader, DatasetWriterOptions, PipelineOptions,
};

#[derive(Parser, Debug)]
#[command(
    name = "tune-market",
    about = "Lossless compression tuning for bench market data"
)]
struct Args {
    /// Record count to generate and measure.
    #[arg(long, default_value_t = 100_000)]
    records: u64,
    #[arg(long, default_value_t = 42)]
    seed: u64,
    #[arg(long, default_value_t = 8)]
    workers: usize,
    #[arg(long, default_value = "benchmarks/tune")]
    out: PathBuf,
    #[arg(
        long,
        default_value = "examples/profiles/bench-market-max-compression.toml"
    )]
    write_profile: PathBuf,
}

#[derive(Clone)]
struct Candidate {
    name: String,
    profile_toml: String,
}

struct Trial {
    name: String,
    physical_bytes: u64,
    logical_bytes: u64,
    compressed_bytes: u64,
    semantic_bytes: u64,
    ratio: f64,
    encode_ms: u64,
    decode_ms: u64,
    verified: u64,
    profile_toml: String,
}

fn main() -> Result<()> {
    anyhow::ensure!(
        !cfg!(debug_assertions),
        "compression tuning requires --release"
    );
    let args = Args::parse();
    anyhow::ensure!(
        args.records <= 1_000_000 && args.records > 0,
        "tuning is sample-bounded to 1..1000000 rows; use baseline for large streaming runs"
    );
    std::fs::create_dir_all(&args.out)?;
    let scratch = args.out.join(format!("scratch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch)?;

    println!(
        "tune-market: generating {} market records (seed={}) …",
        args.records, args.seed
    );
    let mut records = Vec::new();
    let mut buf = Vec::new();
    let mut start = 0u64;
    while start < args.records {
        let n = 65_536u64.min(args.records - start);
        generate_chunk(Workload::Market, args.seed, start, n, &mut buf);
        records.append(&mut buf);
        start += n;
    }

    let base = parse_profile_str(BASE_TOML, "bench-market.toml")?;
    let candidates = build_candidates(&base)?;
    println!("measuring {} lossless candidates …", candidates.len());

    let mut trials = Vec::new();
    let mut failures = Vec::new();
    for (i, c) in candidates.iter().enumerate() {
        let root = scratch.join(format!("c{i}"));
        match measure(&root, &c.profile_toml, &records, args.workers) {
            Ok(mut t) => {
                t.name = c.name.clone();
                println!(
                    "  [{:>2}/{}] {:<42} physical={:>10}  ratio={:.3}×",
                    i + 1,
                    candidates.len(),
                    t.name,
                    t.physical_bytes,
                    t.ratio
                );
                trials.push(t);
            }
            Err(e) => {
                let msg = format!("{}: {e:#}", c.name);
                println!("  FAIL {msg}");
                failures.push(msg);
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    trials.sort_by_key(|t| t.physical_bytes);
    if trials.is_empty() {
        bail!("all candidates failed: {}", failures.join("; "));
    }

    println!(
        "\n=== smallest lossless physical sizes (n={}) ===",
        args.records
    );
    for (rank, t) in trials.iter().take(12).enumerate() {
        println!(
            "{:>2}. {:<42} physical={:<10} compressed={:<10} semantic={:<10} ratio={:.3}×  enc_ms={} dec_ms={}",
            rank + 1,
            t.name,
            t.physical_bytes,
            t.compressed_bytes,
            t.semantic_bytes,
            t.ratio,
            t.encode_ms,
            t.decode_ms
        );
    }

    let winner = &trials[0];
    std::fs::write(&args.write_profile, &winner.profile_toml)
        .with_context(|| format!("write {}", args.write_profile.display()))?;
    println!(
        "\nwrote winning profile → {} (physical {} bytes on {} rows)",
        args.write_profile.display(),
        winner.physical_bytes,
        args.records
    );

    let report = serde_json::json!({
        "release_build": !cfg!(debug_assertions),
        "records": args.records,
        "seed": args.seed,
        "baseline_lz4_auto_note": "Compare against strata-bench --suite compare lz4 auto plans on same seed",
        "winner": winner.name,
        "winner_physical_bytes": winner.physical_bytes,
        "winner_compressed_bytes": winner.compressed_bytes,
        "winner_semantic_bytes": winner.semantic_bytes,
        "winner_ratio_logical_over_physical": winner.ratio,
        "winner_profile": args.write_profile.display().to_string(),
        "top": trials.iter().take(12).map(|t| serde_json::json!({
            "name": t.name,
            "physical_bytes": t.physical_bytes,
            "logical_bytes": t.logical_bytes,
            "semantic_bytes": t.semantic_bytes,
            "compressed_bytes": t.compressed_bytes,
            "ratio_logical_over_physical": t.ratio,
            "encode_ms": t.encode_ms,
            "decode_ms": t.decode_ms,
            "verified_records": t.verified,
        })).collect::<Vec<_>>(),
        "failures": failures,
    });
    let json_path = args
        .out
        .join(format!("tune-market-{}-{}.json", args.records, args.seed));
    std::fs::write(&json_path, serde_json::to_string_pretty(&report)?)?;
    println!("wrote {}", json_path.display());

    let _ = std::fs::remove_dir_all(&scratch);
    Ok(())
}

fn build_candidates(base: &CompressionProfile) -> Result<Vec<Candidate>> {
    let mut out = Vec::new();

    // Tailored to the market generator: monotonic ts/seq, sawtooth order_id,
    // random low-cardinality symbol/side/event, bounded random price/qty.
    let families: Vec<(&str, HashMap<&str, &[&str]>)> = vec![
        (
            "stock-like",
            HashMap::from([
                ("timestamp", &["delta_of_delta", "zigzag", "bitpack"][..]),
                ("sequence", &["delta", "zigzag", "bitpack"][..]),
                ("order_id", &["delta", "zigzag", "bitpack"][..]),
                ("symbol", &["dictionary", "bitpack"][..]),
                ("price", &["frame_of_reference", "bitpack"][..]),
                ("quantity", &["varint"][..]),
                ("side", &["enum_bitpack"][..]),
                ("event_type", &["enum_bitpack"][..]),
            ]),
        ),
        (
            "orderid-for",
            HashMap::from([
                ("timestamp", &["delta_of_delta", "zigzag", "bitpack"][..]),
                ("sequence", &["delta", "zigzag", "bitpack"][..]),
                ("order_id", &["frame_of_reference", "bitpack"][..]),
                ("symbol", &["dictionary", "bitpack"][..]),
                ("price", &["frame_of_reference", "bitpack"][..]),
                ("quantity", &["bitpack"][..]),
                ("side", &["enum_bitpack"][..]),
                ("event_type", &["enum_bitpack"][..]),
            ]),
        ),
        (
            "tight-bitpack",
            HashMap::from([
                ("timestamp", &["delta_of_delta", "zigzag", "bitpack"][..]),
                ("sequence", &["delta", "zigzag", "bitpack"][..]),
                ("order_id", &["frame_of_reference", "bitpack"][..]),
                ("symbol", &["dictionary", "bitpack"][..]),
                ("price", &["frame_of_reference", "bitpack"][..]),
                ("quantity", &["frame_of_reference", "bitpack"][..]),
                ("side", &["enum_bitpack"][..]),
                ("event_type", &["enum_bitpack"][..]),
            ]),
        ),
        (
            "aggressive-delta",
            HashMap::from([
                ("timestamp", &["delta_of_delta", "zigzag", "bitpack"][..]),
                ("sequence", &["delta", "zigzag", "bitpack"][..]),
                ("order_id", &["delta", "zigzag", "bitpack"][..]),
                ("symbol", &["dictionary", "bitpack"][..]),
                ("price", &["delta", "zigzag", "bitpack"][..]),
                ("quantity", &["delta", "zigzag", "bitpack"][..]),
                ("side", &["enum_bitpack"][..]),
                ("event_type", &["enum_bitpack"][..]),
            ]),
        ),
        (
            "varint-mix",
            HashMap::from([
                ("timestamp", &["delta_of_delta", "zigzag", "varint"][..]),
                ("sequence", &["delta", "zigzag", "varint"][..]),
                ("order_id", &["frame_of_reference", "varint"][..]),
                ("symbol", &["dictionary", "varint"][..]),
                ("price", &["frame_of_reference", "varint"][..]),
                ("quantity", &["varint"][..]),
                ("side", &["enum_bitpack"][..]),
                ("event_type", &["enum_bitpack"][..]),
            ]),
        ),
        (
            "dict-rle-cats",
            HashMap::from([
                ("timestamp", &["delta_of_delta", "zigzag", "bitpack"][..]),
                ("sequence", &["delta", "zigzag", "bitpack"][..]),
                ("order_id", &["frame_of_reference", "bitpack"][..]),
                ("symbol", &["dictionary", "run_length"][..]),
                ("price", &["frame_of_reference", "bitpack"][..]),
                ("quantity", &["bitpack"][..]),
                ("side", &["dictionary", "run_length"][..]),
                ("event_type", &["dictionary", "run_length"][..]),
            ]),
        ),
        (
            "zstd-friendly-plain-nums",
            HashMap::from([
                ("timestamp", &["delta_of_delta", "zigzag", "bitpack"][..]),
                ("sequence", &["delta", "zigzag", "bitpack"][..]),
                ("order_id", &["plain"][..]),
                ("symbol", &["dictionary", "bitpack"][..]),
                ("price", &["plain"][..]),
                ("quantity", &["plain"][..]),
                ("side", &["enum_bitpack"][..]),
                ("event_type", &["enum_bitpack"][..]),
            ]),
        ),
        (
            "max-semantic",
            HashMap::from([
                ("timestamp", &["delta_of_delta", "zigzag", "bitpack"][..]),
                ("sequence", &["delta", "zigzag", "bitpack"][..]),
                ("order_id", &["delta", "zigzag", "varint"][..]),
                ("symbol", &["dictionary", "bitpack"][..]),
                ("price", &["frame_of_reference", "bitpack"][..]),
                ("quantity", &["frame_of_reference", "bitpack"][..]),
                ("side", &["enum_bitpack"][..]),
                ("event_type", &["enum_bitpack"][..]),
            ]),
        ),
        (
            "plain",
            HashMap::from([
                ("timestamp", &["plain"][..]),
                ("sequence", &["plain"][..]),
                ("order_id", &["plain"][..]),
                ("symbol", &["plain"][..]),
                ("price", &["plain"][..]),
                ("quantity", &["plain"][..]),
                ("side", &["plain"][..]),
                ("event_type", &["plain"][..]),
            ]),
        ),
    ];

    let codecs: &[(&str, CompressionCodecName, Option<i32>)] = &[
        ("lz4", CompressionCodecName::Lz4, None),
        ("zstd3", CompressionCodecName::Zstd, Some(3)),
        ("zstd6", CompressionCodecName::Zstd, Some(6)),
        ("zstd9", CompressionCodecName::Zstd, Some(9)),
        ("zstd12", CompressionCodecName::Zstd, Some(12)),
        ("zstd15", CompressionCodecName::Zstd, Some(15)),
        ("zstd19", CompressionCodecName::Zstd, Some(19)),
    ];

    let block_rows: &[u32] = &[16_384, 65_536, 131_072];

    for (fam_name, plans) in &families {
        for (codec_name, codec, level) in codecs {
            for &rows in block_rows {
                let mut profile = base.clone();
                profile.name = format!("bench-market-{fam_name}-{codec_name}-b{rows}");
                profile.compression.codec = *codec;
                profile.compression.level = *level;
                profile.block.row_count = rows;
                for col in &mut profile.columns {
                    if let Some(steps) = plans.get(col.name.as_str()) {
                        col.encoding = ColumnEncodingSpec::Pipeline(
                            steps.iter().map(|s| (*s).to_string()).collect(),
                        );
                    }
                }
                // Resolve + validate the same way the writer does (bitpack→enum_bitpack, etc.).
                resolve_profile_plans(&profile).with_context(|| {
                    format!("invalid plan family={fam_name} codec={codec_name} block={rows}")
                })?;
                let text = hand_emit(&profile);
                parse_profile_str(&text, "candidate.toml")?;
                out.push(Candidate {
                    name: format!("{fam_name} + {codec_name} + b{rows}"),
                    profile_toml: text,
                });
            }
        }
    }
    Ok(out)
}

fn measure(
    root: &std::path::Path,
    profile_toml: &str,
    records: &[Record],
    workers: usize,
) -> Result<Trial> {
    let profile = parse_profile_str(profile_toml, "candidate.toml")?;
    let schema = profile.schema()?;
    let codec_name = profile
        .compression
        .effective_codec()
        .map_err(anyhow::Error::msg)?;
    let codec = CompressionCodec::parse_name(codec_name.as_str())
        .with_context(|| format!("unknown codec {}", codec_name.as_str()))?;
    let metrics = MetricsHandle::new();
    let mut opts = PipelineOptions::with_workers(workers);
    opts.writer = DatasetWriterOptions {
        block_row_count: profile.block.row_count,
        segment_target_size_bytes: profile.segment.target_size_bytes,
        encoding_plans: resolve_profile_plans(&profile)?,
        compressor: Arc::from(compressor_for(codec, profile.compression.level)),
        metrics: Some(metrics.clone()),
        durability: DurabilityMode::Fast,
        ..Default::default()
    };
    let t0 = Instant::now();
    write_records_parallel(root, schema, records, opts)?;
    let encode_ms = t0.elapsed().as_millis() as u64;
    let snap = metrics.finish();

    let t1 = Instant::now();
    let reader = DatasetReader::open(root)?;
    let mut offset = 0usize;
    for i in 0..reader.segment_count() {
        let seg = reader.open_segment(i)?;
        for b in 0..seg.block_count() {
            let batch = seg.read_block(b)?;
            if batch.records != records[offset..offset + batch.records.len()] {
                bail!("lossless roundtrip mismatch at offset {offset}");
            }
            offset += batch.records.len();
            std::hint::black_box(&batch);
        }
    }
    if offset != records.len() {
        bail!("lost records: decoded {offset} expected {}", records.len());
    }
    let decode_ms = t1.elapsed().as_millis() as u64;
    let physical = dir_size(root)?;
    let logical = snap.logical_input_bytes.get();
    let ratio = if physical == 0 {
        0.0
    } else {
        logical as f64 / physical as f64
    };
    Ok(Trial {
        name: String::new(),
        physical_bytes: physical,
        logical_bytes: logical,
        compressed_bytes: snap.compressed_bytes.get(),
        semantic_bytes: snap.semantic_encoded_bytes.get(),
        ratio,
        encode_ms,
        decode_ms,
        verified: offset as u64,
        profile_toml: profile_toml.to_string(),
    })
}

fn dir_size(path: &std::path::Path) -> Result<u64> {
    let mut total = 0u64;
    for entry in std::fs::read_dir(path)? {
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

fn hand_emit(p: &CompressionProfile) -> String {
    let mut out = String::new();
    out.push_str(&format!("name = \"{}\"\n", p.name));
    out.push_str("input_format = \"csv\"\n\n[input]\nrecursive = true\nextensions = [\"csv\"]\n\n");
    out.push_str("[schema]\n");
    out.push_str(&format!("id = \"{}\"\n", p.schema.id.as_str()));
    if let Some(d) = &p.schema.description {
        out.push_str(&format!("description = \"{d}\"\n"));
    }
    out.push('\n');
    for c in &p.columns {
        out.push_str("[[columns]]\n");
        out.push_str(&format!("name = \"{}\"\n", c.name));
        out.push_str(&format!("logical_type = \"{}\"\n", c.logical_type));
        out.push_str(&format!("nullable = {}\n", c.nullable));
        if let Some(w) = c.width {
            out.push_str(&format!(
                "width = \"{}\"\n",
                match w {
                    IntegerWidth::Bits8 => "bits8",
                    IntegerWidth::Bits16 => "bits16",
                    IntegerWidth::Bits32 => "bits32",
                    IntegerWidth::Bits64 => "bits64",
                }
            ));
        }
        if let Some(u) = c.unit {
            out.push_str(&format!(
                "unit = \"{}\"\n",
                match u {
                    TimestampUnit::Seconds => "seconds",
                    TimestampUnit::Milliseconds => "milliseconds",
                    TimestampUnit::Microseconds => "microseconds",
                    TimestampUnit::Nanoseconds => "nanoseconds",
                }
            ));
        }
        if let Some(p0) = c.precision {
            out.push_str(&format!("precision = {p0}\n"));
        }
        if let Some(s) = c.scale {
            out.push_str(&format!("scale = {s}\n"));
        }
        if let Some(m) = c.max_bytes {
            out.push_str(&format!("max_bytes = {m}\n"));
        }
        if !c.variants.is_empty() {
            out.push_str("variants = [");
            out.push_str(
                &c.variants
                    .iter()
                    .map(|x| format!("\"{x}\""))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push_str("]\n");
        }
        if let ColumnEncodingSpec::Pipeline(steps) = &c.encoding {
            out.push_str("encoding = [");
            out.push_str(
                &steps
                    .iter()
                    .map(|s| format!("\"{s}\""))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push_str("]\n");
        }
        out.push('\n');
    }
    out.push_str(&format!(
        "[block]\nrow_count = {}\n\n[segment]\ntarget_size_bytes = {}\n\n[compression]\ncodec = \"{}\"\n",
        p.block.row_count,
        p.segment.target_size_bytes,
        p.compression.codec.as_str()
    ));
    if let Some(level) = p.compression.level {
        out.push_str(&format!("level = {level}\n"));
    }
    out
}

const BASE_TOML: &str = r#"
name = "bench-market"
input_format = "csv"

[input]
recursive = true
extensions = ["csv"]

[schema]
id = "bench_market_events"
description = "strata-bench market generator schema"

[[columns]]
name = "timestamp"
logical_type = "timestamp"
unit = "nanoseconds"
nullable = false
encoding = ["delta_of_delta", "zigzag", "bitpack"]

[[columns]]
name = "sequence"
logical_type = "unsigned_integer"
width = "bits64"
nullable = false
encoding = ["delta", "zigzag", "bitpack"]

[[columns]]
name = "order_id"
logical_type = "unsigned_integer"
width = "bits64"
nullable = false
encoding = ["delta", "zigzag", "bitpack"]

[[columns]]
name = "symbol"
logical_type = "categorical_string"
max_bytes = 8
nullable = false
encoding = ["dictionary", "bitpack"]

[[columns]]
name = "price"
logical_type = "fixed_decimal"
precision = 18
scale = 4
width = "bits64"
nullable = false
encoding = ["frame_of_reference", "bitpack"]

[[columns]]
name = "quantity"
logical_type = "unsigned_integer"
width = "bits32"
nullable = false
encoding = ["varint"]

[[columns]]
name = "side"
logical_type = "enum"
variants = ["buy", "sell"]
nullable = false
encoding = ["enum_bitpack"]

[[columns]]
name = "event_type"
logical_type = "enum"
variants = ["new", "cancel", "fill", "replace"]
nullable = false
encoding = ["enum_bitpack"]

[block]
row_count = 65536

[segment]
target_size_bytes = 268435456

[compression]
codec = "zstd"
level = 3
"#;
