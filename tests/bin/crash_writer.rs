//! Crash-injection writer used by durability integration tests.
//!
//! Prints `ACK <n>` whenever the durability barrier advances, then optionally
//! aborts or waits to be killed to simulate a hard process crash.

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use strata_codec::{compressor_for, CompressionCodec};
use strata_core::{
    DurabilityMode, Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit,
};
use strata_storage::{DatasetWriter, DatasetWriterOptions};

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    output: PathBuf,

    #[arg(long, default_value = "strict")]
    durability: String,

    #[arg(long, default_value_t = 20_000)]
    records: u64,

    #[arg(long, default_value_t = 512)]
    block_rows: u32,

    /// Abort / wait after the durability ack reaches at least this many records.
    #[arg(long)]
    crash_after_ack: Option<u64>,

    /// Sleep forever after the target ack (parent will SIGKILL).
    #[arg(long, default_value_t = false)]
    wait_for_kill: bool,

    #[arg(long, default_value_t = 4)]
    sync_every_blocks: u32,
}

fn emit(line: &str) {
    let mut out = io::stdout().lock();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

fn main() {
    let args = Args::parse();
    let mode = DurabilityMode::parse(&args.durability).expect("durability");
    let schema = Schema::new(
        "crash_ticks",
        vec![
            Field::required("seq", LogicalType::unsigned_integer(IntegerWidth::Bits64)),
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required("symbol", LogicalType::categorical_string(Some(8))),
        ],
    )
    .expect("schema");

    let _ = std::fs::remove_dir_all(&args.output);
    let mut writer = DatasetWriter::create(
        &args.output,
        schema,
        DatasetWriterOptions {
            block_row_count: args.block_rows,
            segment_target_size_bytes: 64 * 1024 * 1024,
            compressor: Arc::from(compressor_for(CompressionCodec::None, None)),
            durability: mode,
            sync_every_blocks: args.sync_every_blocks,
            ..Default::default()
        },
    )
    .expect("create writer");

    let mut last_ack = 0u64;
    let mut batch = Vec::with_capacity(args.block_rows as usize);
    for i in 0..args.records {
        batch.push(
            Record::new()
                .with("seq", FieldValue::UnsignedInteger(i))
                .with("ts", FieldValue::Timestamp(i as i64 * 1000))
                .with(
                    "symbol",
                    FieldValue::CategoricalString(format!("S{}", i % 8)),
                ),
        );
        if batch.len() >= args.block_rows as usize {
            writer.write_records(&batch).expect("write");
            batch.clear();
            let ack = writer.acknowledged_records();
            if ack > last_ack {
                emit(&format!("ACK {ack}"));
                last_ack = ack;
                if let Some(target) = args.crash_after_ack {
                    if ack >= target {
                        if args.wait_for_kill {
                            emit(&format!("READY_FOR_KILL {ack}"));
                            loop {
                                std::thread::sleep(std::time::Duration::from_secs(60));
                            }
                        } else {
                            emit(&format!("CRASHING {ack}"));
                            // Hard abort: no destructors, no finish(), no manifest.
                            std::process::abort();
                        }
                    }
                }
            }
        }
    }
    if !batch.is_empty() {
        writer.write_records(&batch).expect("write tail");
    }
    let _ = writer.finish().expect("finish");
    emit(&format!("ACK {}", last_ack.max(args.records)));
    emit("CLEAN_EXIT");
}
