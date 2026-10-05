//! Telemetry event generator.

use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};

use super::rng::Rng;

const DEVICES: &[&str] = &[
    "dev-001", "dev-002", "dev-003", "dev-004", "dev-005", "dev-006", "dev-007", "dev-008",
];
const METRICS: &[&str] = &["cpu", "mem", "disk", "net_in", "net_out", "temp"];
const SEVERITY: &[&str] = &["info", "warn", "error"];

/// Telemetry schema.
pub fn telemetry_schema() -> Schema {
    Schema::new(
        "bench_telemetry",
        vec![
            Field::required(
                "event_id",
                LogicalType::unsigned_integer(IntegerWidth::Bits64),
            ),
            Field::required("ts", LogicalType::timestamp(TimestampUnit::Microseconds)),
            Field::required("device_id", LogicalType::categorical_string(Some(16))),
            Field::required("metric", LogicalType::categorical_string(Some(16))),
            Field::required(
                "value",
                LogicalType::fixed_decimal(18, 4, IntegerWidth::Bits64),
            ),
            Field::required(
                "severity",
                LogicalType::enum_type(SEVERITY.iter().map(|s| (*s).to_string()).collect()),
            ),
            Field::required("ok", LogicalType::boolean()),
        ],
    )
    .expect("telemetry schema")
}

/// Deterministic telemetry chunk.
pub fn generate_telemetry_chunk(seed: u64, start: u64, count: u64, out: &mut Vec<Record>) {
    let mut rng = Rng::new(seed ^ start.wrapping_mul(0xA5A5_A5A5_5A5A_5A5A));
    out.reserve(count as usize);
    let base_ts = 1_700_000_000_000_000i64;
    for i in 0..count {
        let id = start + i;
        let _ = rng.next_u64();
        out.push(
            Record::new()
                .with("event_id", FieldValue::UnsignedInteger(id))
                .with("ts", FieldValue::Timestamp(base_ts + id as i64 * 250))
                .with(
                    "device_id",
                    FieldValue::CategoricalString((*rng.choose(DEVICES)).into()),
                )
                .with(
                    "metric",
                    FieldValue::CategoricalString((*rng.choose(METRICS)).into()),
                )
                .with(
                    "value",
                    FieldValue::FixedDecimal(rng.gen_range(10_000) as i64),
                )
                .with("severity", FieldValue::Enum((*rng.choose(SEVERITY)).into()))
                .with("ok", FieldValue::Boolean(rng.gen_range(10) > 0)),
        );
    }
}
