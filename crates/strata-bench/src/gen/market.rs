//! Market event generator.

use strata_core::{Field, FieldValue, IntegerWidth, LogicalType, Record, Schema, TimestampUnit};

use super::rng::Rng;

const SYMBOLS: &[&str] = &[
    "AAPL", "MSFT", "GOOG", "AMZN", "META", "NVDA", "TSLA", "AMD", "INTC", "NFLX",
];
const SIDES: &[&str] = &["buy", "sell"];
const EVENT_TYPES: &[&str] = &["new", "cancel", "fill", "replace"];

/// Market schema from Prompt 16.
pub fn market_schema() -> Schema {
    Schema::new(
        "bench_market_events",
        vec![
            Field::required(
                "timestamp",
                LogicalType::timestamp(TimestampUnit::Nanoseconds),
            ),
            Field::required(
                "sequence",
                LogicalType::unsigned_integer(IntegerWidth::Bits64),
            ),
            Field::required(
                "order_id",
                LogicalType::unsigned_integer(IntegerWidth::Bits64),
            ),
            Field::required("symbol", LogicalType::categorical_string(Some(8))),
            Field::required(
                "price",
                LogicalType::fixed_decimal(18, 4, IntegerWidth::Bits64),
            ),
            Field::required(
                "quantity",
                LogicalType::unsigned_integer(IntegerWidth::Bits32),
            ),
            Field::required(
                "side",
                LogicalType::enum_type(SIDES.iter().map(|s| (*s).to_string()).collect()),
            ),
            Field::required(
                "event_type",
                LogicalType::enum_type(EVENT_TYPES.iter().map(|s| (*s).to_string()).collect()),
            ),
        ],
    )
    .expect("market schema")
}

/// Deterministic market chunk.
pub fn generate_market_chunk(seed: u64, start: u64, count: u64, out: &mut Vec<Record>) {
    out.reserve(count as usize);
    let base_ts = 1_700_000_000_000_000_000i64;
    for i in 0..count {
        let seq = start + i;
        let mut rng = Rng::new(seed ^ seq.wrapping_mul(0xD1B5_4A32_D192_ED03));
        let jitter = rng.gen_range(900) as i64;
        let symbol = rng.gen_range(100);
        let symbol_index = if symbol < 30 {
            0
        } else if symbol < 50 {
            1
        } else {
            2 + (symbol as usize % 8)
        };
        out.push(
            Record::new()
                .with(
                    "timestamp",
                    FieldValue::Timestamp(base_ts + (seq as i64) * 1_000 + jitter),
                )
                .with("sequence", FieldValue::UnsignedInteger(seq))
                .with(
                    "order_id",
                    FieldValue::UnsignedInteger(
                        1_000_000
                            + if rng.gen_range(100) < 60 {
                                seq
                            } else {
                                seq.saturating_sub(rng.gen_range(50_000))
                            },
                    ),
                )
                .with(
                    "symbol",
                    FieldValue::CategoricalString(SYMBOLS[symbol_index].into()),
                )
                .with(
                    "price",
                    FieldValue::FixedDecimal(
                        1_000_000
                            + symbol_index as i64 * 100_000
                            + (seq / 100_000 % 1000) as i64 * 10
                            + rng.gen_range(5000) as i64,
                    ),
                )
                .with(
                    "quantity",
                    FieldValue::UnsignedInteger(if rng.gen_range(100) < 70 {
                        (1 + rng.gen_range(10)) * 100
                    } else {
                        1 + rng.gen_range(2000)
                    }),
                )
                .with("side", FieldValue::Enum((*rng.choose(SIDES)).into()))
                .with(
                    "event_type",
                    FieldValue::Enum(
                        EVENT_TYPES[match rng.gen_range(100) {
                            0..=59 => 0,
                            60..=79 => 1,
                            80..=94 => 2,
                            _ => 3,
                        }]
                        .into(),
                    ),
                ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunk_boundaries_do_not_change_dataset() {
        let mut whole = Vec::new();
        generate_market_chunk(42, 0, 1000, &mut whole);
        let mut parts = Vec::new();
        generate_market_chunk(42, 0, 317, &mut parts);
        generate_market_chunk(42, 317, 683, &mut parts);
        assert_eq!(whole, parts);
    }
}
