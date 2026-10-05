//! Structured log event generator.

use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};

use super::rng::Rng;

const SERVICES: &[&str] = &[
    "api", "worker", "ingest", "query", "auth", "billing", "notify", "search",
];
const LEVELS: &[&str] = &["debug", "info", "warn", "error"];
const MESSAGES: &[&str] = &[
    "request_ok",
    "request_slow",
    "cache_miss",
    "retry",
    "timeout",
    "auth_fail",
];

/// Logs schema.
pub fn logs_schema() -> Schema {
    Schema::new(
        "bench_logs",
        vec![
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Nanoseconds)),
            Field::required(
                "request_id",
                LogicalType::unsigned_integer(IntegerWidth::Bits64),
            ),
            Field::required("service", LogicalType::categorical_string(Some(16))),
            Field::required(
                "level",
                LogicalType::enum_type(LEVELS.iter().map(|s| (*s).to_string()).collect()),
            ),
            Field::required("message", LogicalType::categorical_string(Some(32))),
            Field::required(
                "latency_us",
                LogicalType::unsigned_integer(IntegerWidth::Bits32),
            ),
            Field::required(
                "status",
                LogicalType::unsigned_integer(IntegerWidth::Bits16),
            ),
        ],
    )
    .expect("logs schema")
}

/// Deterministic logs chunk.
pub fn generate_logs_chunk(seed: u64, start: u64, count: u64, out: &mut Vec<Record>) {
    let mut rng = Rng::new(seed ^ start.wrapping_mul(0xC0FF_EE00_BAD0_F00D));
    out.reserve(count as usize);
    let base_ts = 1_700_000_000_000_000_000i64;
    for i in 0..count {
        let id = start + i;
        let _ = rng.next_u64();
        let status = match rng.gen_range(20) {
            0 => 500,
            1 => 404,
            2 => 429,
            _ => 200,
        };
        out.push(
            Record::new()
                .with("ts", FieldValue::Timestamp(base_ts + id as i64 * 500))
                .with(
                    "request_id",
                    FieldValue::UnsignedInteger(10_000_000 + (id % 200_000)),
                )
                .with(
                    "service",
                    FieldValue::CategoricalString((*rng.choose(SERVICES)).into()),
                )
                .with("level", FieldValue::Enum((*rng.choose(LEVELS)).into()))
                .with(
                    "message",
                    FieldValue::CategoricalString((*rng.choose(MESSAGES)).into()),
                )
                .with(
                    "latency_us",
                    FieldValue::UnsignedInteger(50 + rng.gen_range(5_000)),
                )
                .with("status", FieldValue::UnsignedInteger(status)),
        );
    }
}
