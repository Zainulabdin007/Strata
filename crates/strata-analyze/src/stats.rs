//! Per-column streaming statistics.

use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use strata_core::{FieldValue, LogicalType, TimestampUnit};

/// Display / JSON scalar for min/max and mode values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StatScalar {
    /// Boolean.
    Bool(bool),
    /// Signed integer / timestamp / decimal.
    I64(i64),
    /// Unsigned integer.
    U64(u64),
    /// String / enum.
    String(String),
    /// Binary length (bytes are not inlined in reports).
    BinaryLen(usize),
}

impl StatScalar {
    fn from_field(value: &FieldValue) -> Option<Self> {
        match value {
            FieldValue::Null => None,
            FieldValue::Boolean(v) => Some(Self::Bool(*v)),
            FieldValue::UnsignedInteger(v) => Some(Self::U64(*v)),
            FieldValue::SignedInteger(v)
            | FieldValue::Timestamp(v)
            | FieldValue::FixedDecimal(v) => Some(Self::I64(*v)),
            FieldValue::CategoricalString(s) | FieldValue::Enum(s) => Some(Self::String(s.clone())),
            FieldValue::Binary(b) => Some(Self::BinaryLen(b.len())),
        }
    }
}

impl std::fmt::Display for StatScalar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(v) => write!(f, "{v}"),
            Self::I64(v) => write!(f, "{v}"),
            Self::U64(v) => write!(f, "{v}"),
            Self::String(v) => write!(f, "{v}"),
            Self::BinaryLen(n) => write!(f, "<{n} bytes>"),
        }
    }
}

/// Qualitative label for delta variance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaVarianceLabel {
    /// Near-constant deltas.
    Low,
    /// Moderate spread.
    Medium,
    /// Highly variable deltas.
    High,
    /// Not applicable.
    None,
}

impl DeltaVarianceLabel {
    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::None => "n/a",
        }
    }

    fn from_cv(coefficient_of_variation: f64) -> Self {
        if !coefficient_of_variation.is_finite() {
            return Self::None;
        }
        if coefficient_of_variation < 0.25 {
            Self::Low
        } else if coefficient_of_variation < 1.0 {
            Self::Medium
        } else {
            Self::High
        }
    }
}

/// Run-length encoding oriented statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunLengthStats {
    /// Number of runs observed.
    pub run_count: u64,
    /// Average run length.
    pub avg_run_length: f64,
    /// Longest run length.
    pub max_run_length: u64,
    /// Share of values covered by the longest run.
    pub max_run_fraction: f64,
}

/// Local (per-block) numeric range statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LocalRangeStats {
    /// Block row size used for windowing.
    pub block_rows: u32,
    /// Number of non-empty blocks observed.
    pub block_count: u64,
    /// Average (max-min) within a block.
    pub avg_range: f64,
    /// Maximum (max-min) across blocks.
    pub max_range: f64,
    /// Global (max-min) across the column.
    pub global_range: f64,
    /// Qualitative label: narrow when avg local range ≪ global range.
    pub label: String,
}

/// Value repetition / frequency statistics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RepetitionStats {
    /// Most frequent present value.
    pub mode: StatScalar,
    /// Occurrences of the mode.
    pub mode_count: u64,
    /// `mode_count / present_count`.
    pub mode_frequency: f64,
    /// Top frequencies (value → count), highest first, capped.
    pub top: Vec<(StatScalar, u64)>,
}

/// Optional numeric bound used in reports.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NumericBound {
    /// Bound value.
    pub value: StatScalar,
}

/// Finalized statistics for one column.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ColumnStats {
    /// Column / field name.
    pub name: String,
    /// Logical type name.
    pub logical_type: String,
    /// Timestamp unit when `logical_type` is timestamp (`s` / `ms` / `us` / `ns`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp_unit: Option<String>,
    /// Total rows observed (including nulls).
    pub row_count: u64,
    /// Null row count.
    pub null_count: u64,
    /// Non-null row count.
    pub present_count: u64,
    /// Minimum present value, when ordered comparison is defined.
    pub min: Option<StatScalar>,
    /// Maximum present value, when ordered comparison is defined.
    pub max: Option<StatScalar>,
    /// Number of distinct present values.
    pub cardinality: u64,
    /// `cardinality / present_count` (0 when empty).
    pub cardinality_ratio: f64,
    /// Fraction of adjacent present pairs that are non-decreasing (`None` if <2 present).
    pub sortedness: Option<f64>,
    /// Fraction of adjacent present pairs that are strictly increasing.
    pub monotonicity: Option<f64>,
    /// Average adjacent delta (numeric columns).
    pub avg_delta: Option<f64>,
    /// Median adjacent delta (numeric columns).
    pub median_delta: Option<f64>,
    /// Sample variance of adjacent deltas.
    pub delta_variance: Option<f64>,
    /// Qualitative delta variance label.
    pub delta_variance_label: Option<DeltaVarianceLabel>,
    /// Run-length statistics over present values.
    pub run_length: Option<RunLengthStats>,
    /// Minimum bits to store FOR residuals (`max-min`), when numeric.
    pub min_bit_width: Option<u8>,
    /// Bits to store the absolute max magnitude (unsigned view).
    pub absolute_bit_width: Option<u8>,
    /// Local block range statistics for numeric columns.
    pub local_block_ranges: Option<LocalRangeStats>,
    /// Distinct string/enum/binary dictionary size (same as cardinality for those types).
    pub dictionary_size: Option<u64>,
    /// Repetition / mode statistics.
    pub repetition: Option<RepetitionStats>,
}

/// Streaming accumulator for one column.
#[derive(Debug)]
pub struct ColumnAccumulator {
    name: String,
    logical_type: LogicalType,
    block_rows: u32,
    row_count: u64,
    null_count: u64,
    // Frequency map for cardinality + mode (stringified key for mixed kinds).
    freq: HashMap<ValueKey, u64>,
    // Display scalar for each key (first seen).
    key_display: HashMap<ValueKey, StatScalar>,
    min: Option<OrderedValue>,
    max: Option<OrderedValue>,
    prev: Option<OrderedValue>,
    sorted_pairs: u64,
    mono_pairs: u64,
    adj_pairs: u64,
    deltas: Vec<i128>,
    // Run-length
    run_value: Option<ValueKey>,
    run_len: u64,
    run_count: u64,
    max_run_len: u64,
    // Local blocks
    block_min: Option<OrderedValue>,
    block_max: Option<OrderedValue>,
    block_index_in_window: u32,
    block_ranges: Vec<f64>,
    prev_string: Option<ValueKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ValueKey {
    Bool(bool),
    U64(u64),
    I64(i64),
    Str(String),
    Bin(Vec<u8>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum OrderedValue {
    Bool(bool),
    U64(u64),
    I64(i64),
    // Strings ordered lexicographically via separate path — store index into process
}

impl ColumnAccumulator {
    /// Creates an accumulator for a schema field.
    pub fn new(name: impl Into<String>, logical_type: LogicalType, block_rows: u32) -> Self {
        Self {
            name: name.into(),
            logical_type,
            block_rows: block_rows.max(1),
            row_count: 0,
            null_count: 0,
            freq: HashMap::new(),
            key_display: HashMap::new(),
            min: None,
            max: None,
            prev: None,
            sorted_pairs: 0,
            mono_pairs: 0,
            adj_pairs: 0,
            deltas: Vec::new(),
            run_value: None,
            run_len: 0,
            run_count: 0,
            max_run_len: 0,
            block_min: None,
            block_max: None,
            block_index_in_window: 0,
            block_ranges: Vec::new(),
            prev_string: None,
        }
    }

    /// Observes one row value (null allowed).
    pub fn push(&mut self, value: &FieldValue) {
        self.row_count += 1;
        if value.is_null() {
            self.null_count += 1;
            // Nulls break runs and do not participate in numeric windows.
            self.flush_run();
            self.run_value = None;
            self.run_len = 0;
            self.prev = None;
            self.prev_string = None;
            return;
        }

        let key = value_key(value);
        let display = StatScalar::from_field(value).expect("non-null");
        *self.freq.entry(key.clone()).or_insert(0) += 1;
        self.key_display.entry(key.clone()).or_insert(display);

        // Runs
        match &self.run_value {
            Some(prev) if prev == &key => {
                self.run_len += 1;
            }
            _ => {
                self.flush_run();
                self.run_value = Some(key);
                self.run_len = 1;
            }
        }

        if let Some(ord) = ordered_value(value) {
            self.min = Some(match self.min {
                Some(m) => m.min(ord),
                None => ord,
            });
            self.max = Some(match self.max {
                Some(m) => m.max(ord),
                None => ord,
            });

            if let Some(prev) = self.prev {
                self.adj_pairs += 1;
                if ord >= prev {
                    self.sorted_pairs += 1;
                }
                if ord > prev {
                    self.mono_pairs += 1;
                }
                if let Some(delta) = signed_delta(prev, ord) {
                    self.deltas.push(delta);
                }
            }
            self.prev = Some(ord);

            // Local block window over present numeric values.
            self.block_min = Some(match self.block_min {
                Some(m) => m.min(ord),
                None => ord,
            });
            self.block_max = Some(match self.block_max {
                Some(m) => m.max(ord),
                None => ord,
            });
            self.block_index_in_window += 1;
            if self.block_index_in_window >= self.block_rows {
                self.flush_block_range();
            }
        } else if matches!(
            value,
            FieldValue::CategoricalString(_) | FieldValue::Enum(_)
        ) {
            // Lexicographic sortedness / monotonicity for strings & enums.
            let s = match value {
                FieldValue::CategoricalString(s) | FieldValue::Enum(s) => s.as_str(),
                _ => unreachable!(),
            };
            if let Some(ValueKey::Str(prev)) = self.prev_string.as_ref() {
                self.adj_pairs += 1;
                if s >= prev.as_str() {
                    self.sorted_pairs += 1;
                }
                if s > prev.as_str() {
                    self.mono_pairs += 1;
                }
            }
            self.prev_string = Some(ValueKey::Str(s.to_string()));
        }
    }

    fn flush_run(&mut self) {
        if self.run_len > 0 {
            self.run_count += 1;
            self.max_run_len = self.max_run_len.max(self.run_len);
        }
    }

    fn flush_block_range(&mut self) {
        if let (Some(lo), Some(hi)) = (self.block_min, self.block_max) {
            if let Some(range) = ordered_range(lo, hi) {
                self.block_ranges.push(range);
            }
        }
        self.block_min = None;
        self.block_max = None;
        self.block_index_in_window = 0;
    }

    /// Finalizes statistics.
    pub fn finish(mut self) -> ColumnStats {
        self.flush_run();
        if self.block_index_in_window > 0 {
            self.flush_block_range();
        }

        let present_count = self.row_count - self.null_count;
        let cardinality = self.freq.len() as u64;
        let cardinality_ratio = if present_count == 0 {
            0.0
        } else {
            cardinality as f64 / present_count as f64
        };

        let (sortedness, monotonicity) = if self.adj_pairs == 0 {
            (None, None)
        } else {
            (
                Some(self.sorted_pairs as f64 / self.adj_pairs as f64),
                Some(self.mono_pairs as f64 / self.adj_pairs as f64),
            )
        };

        let (avg_delta, median_delta, delta_variance, delta_variance_label) =
            summarize_deltas(&self.deltas);

        let run_length = if present_count == 0 {
            None
        } else {
            let run_count = self.run_count.max(1);
            Some(RunLengthStats {
                run_count,
                avg_run_length: present_count as f64 / run_count as f64,
                max_run_length: self.max_run_len.max(1),
                max_run_fraction: self.max_run_len as f64 / present_count as f64,
            })
        };

        let (min_bit_width, absolute_bit_width, local_block_ranges) =
            numeric_widths_and_ranges(&self);

        let dictionary_size = match &self.logical_type {
            LogicalType::CategoricalString(_) | LogicalType::Enum(_) | LogicalType::Binary(_) => {
                Some(cardinality)
            }
            _ => None,
        };

        let repetition = repetition_from_freq(&self.freq, &self.key_display, present_count);

        let timestamp_unit = match &self.logical_type {
            LogicalType::Timestamp(ts) => Some(
                match ts.unit {
                    TimestampUnit::Seconds => "s",
                    TimestampUnit::Milliseconds => "ms",
                    TimestampUnit::Microseconds => "us",
                    TimestampUnit::Nanoseconds => "ns",
                }
                .to_string(),
            ),
            _ => None,
        };

        ColumnStats {
            name: self.name,
            logical_type: self.logical_type.name().to_string(),
            timestamp_unit,
            row_count: self.row_count,
            null_count: self.null_count,
            present_count,
            min: self.min.map(ordered_to_scalar),
            max: self.max.map(ordered_to_scalar),
            cardinality,
            cardinality_ratio,
            sortedness,
            monotonicity,
            avg_delta,
            median_delta,
            delta_variance,
            delta_variance_label,
            run_length,
            min_bit_width,
            absolute_bit_width,
            local_block_ranges,
            dictionary_size,
            repetition,
        }
    }
}

fn value_key(value: &FieldValue) -> ValueKey {
    match value {
        FieldValue::Boolean(v) => ValueKey::Bool(*v),
        FieldValue::UnsignedInteger(v) => ValueKey::U64(*v),
        FieldValue::SignedInteger(v) | FieldValue::Timestamp(v) | FieldValue::FixedDecimal(v) => {
            ValueKey::I64(*v)
        }
        FieldValue::CategoricalString(s) | FieldValue::Enum(s) => ValueKey::Str(s.clone()),
        FieldValue::Binary(b) => ValueKey::Bin(b.clone()),
        FieldValue::Null => unreachable!("null filtered"),
    }
}

fn ordered_value(value: &FieldValue) -> Option<OrderedValue> {
    match value {
        FieldValue::Boolean(v) => Some(OrderedValue::Bool(*v)),
        FieldValue::UnsignedInteger(v) => Some(OrderedValue::U64(*v)),
        FieldValue::SignedInteger(v) | FieldValue::Timestamp(v) | FieldValue::FixedDecimal(v) => {
            Some(OrderedValue::I64(*v))
        }
        _ => None,
    }
}

fn ordered_to_scalar(v: OrderedValue) -> StatScalar {
    match v {
        OrderedValue::Bool(b) => StatScalar::Bool(b),
        OrderedValue::U64(n) => StatScalar::U64(n),
        OrderedValue::I64(n) => StatScalar::I64(n),
    }
}

fn signed_delta(prev: OrderedValue, next: OrderedValue) -> Option<i128> {
    match (prev, next) {
        (OrderedValue::U64(a), OrderedValue::U64(b)) => Some(i128::from(b) - i128::from(a)),
        (OrderedValue::I64(a), OrderedValue::I64(b)) => Some(i128::from(b) - i128::from(a)),
        (OrderedValue::Bool(a), OrderedValue::Bool(b)) => {
            Some(i128::from(b as u8) - i128::from(a as u8))
        }
        _ => None,
    }
}

fn ordered_range(lo: OrderedValue, hi: OrderedValue) -> Option<f64> {
    match (lo, hi) {
        (OrderedValue::U64(a), OrderedValue::U64(b)) => Some((b - a) as f64),
        (OrderedValue::I64(a), OrderedValue::I64(b)) => Some((b as i128 - a as i128) as f64),
        (OrderedValue::Bool(a), OrderedValue::Bool(b)) => {
            Some(f64::from(u8::from(b)) - f64::from(u8::from(a)))
        }
        _ => None,
    }
}

fn summarize_deltas(
    deltas: &[i128],
) -> (
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<DeltaVarianceLabel>,
) {
    if deltas.is_empty() {
        return (None, None, None, None);
    }
    let n = deltas.len() as f64;
    let sum: f64 = deltas.iter().map(|d| *d as f64).sum();
    let avg = sum / n;
    let mut sorted = deltas.to_vec();
    sorted.sort_unstable();
    let median = if sorted.len() % 2 == 1 {
        sorted[sorted.len() / 2] as f64
    } else {
        let hi = sorted[sorted.len() / 2] as f64;
        let lo = sorted[sorted.len() / 2 - 1] as f64;
        (lo + hi) / 2.0
    };
    let var = if deltas.len() == 1 {
        0.0
    } else {
        deltas
            .iter()
            .map(|d| {
                let x = *d as f64 - avg;
                x * x
            })
            .sum::<f64>()
            / (n - 1.0)
    };
    let std = var.sqrt();
    let mean_abs = avg.abs().max(1e-12);
    let cv = std / mean_abs;
    (
        Some(avg),
        Some(median),
        Some(var),
        Some(DeltaVarianceLabel::from_cv(cv)),
    )
}

fn bit_width_u128(v: u128) -> u8 {
    if v == 0 {
        1
    } else {
        (128 - v.leading_zeros()) as u8
    }
}

fn numeric_widths_and_ranges(
    acc: &ColumnAccumulator,
) -> (Option<u8>, Option<u8>, Option<LocalRangeStats>) {
    let (Some(lo), Some(hi)) = (acc.min, acc.max) else {
        return (None, None, None);
    };
    let global_range = match ordered_range(lo, hi) {
        Some(r) => r,
        None => return (None, None, None),
    };
    let for_span = match (lo, hi) {
        (OrderedValue::U64(a), OrderedValue::U64(b)) => u128::from(b - a),
        (OrderedValue::I64(a), OrderedValue::I64(b)) => (b as i128 - a as i128).unsigned_abs(),
        (OrderedValue::Bool(_), OrderedValue::Bool(_)) => {
            if lo == hi {
                0
            } else {
                1
            }
        }
        _ => 0,
    };
    let abs_max = match hi {
        OrderedValue::U64(v) => u128::from(v),
        OrderedValue::I64(v) => {
            let a = lo;
            let candidates = [
                v,
                match a {
                    OrderedValue::I64(x) => x,
                    _ => v,
                },
            ];
            candidates
                .iter()
                .map(|x| (*x as i128).unsigned_abs())
                .max()
                .unwrap_or(0)
        }
        OrderedValue::Bool(_) => 1,
    };
    let min_bit_width = Some(bit_width_u128(for_span));
    let absolute_bit_width = Some(bit_width_u128(abs_max));

    let local = if acc.block_ranges.is_empty() {
        None
    } else {
        let avg_range = acc.block_ranges.iter().sum::<f64>() / acc.block_ranges.len() as f64;
        let max_range = acc.block_ranges.iter().copied().fold(0.0_f64, f64::max);
        let label = if global_range <= f64::EPSILON {
            "degenerate".to_string()
        } else {
            let ratio = avg_range / global_range;
            if ratio <= 0.05 {
                "narrow".into()
            } else if ratio <= 0.25 {
                "moderate".into()
            } else {
                "wide".into()
            }
        };
        Some(LocalRangeStats {
            block_rows: acc.block_rows,
            block_count: acc.block_ranges.len() as u64,
            avg_range,
            max_range,
            global_range,
            label,
        })
    };

    (min_bit_width, absolute_bit_width, local)
}

fn repetition_from_freq(
    freq: &HashMap<ValueKey, u64>,
    display: &HashMap<ValueKey, StatScalar>,
    present_count: u64,
) -> Option<RepetitionStats> {
    if present_count == 0 || freq.is_empty() {
        return None;
    }
    let mut items: Vec<_> = freq.iter().collect();
    items.sort_by(|a, b| {
        b.1.cmp(a.1)
            .then_with(|| format!("{:?}", a.0).cmp(&format!("{:?}", b.0)))
    });
    let (mode_key, &mode_count) = items[0];
    let mode = display
        .get(mode_key)
        .cloned()
        .unwrap_or(StatScalar::String("?".into()));
    let top: Vec<_> = items
        .into_iter()
        .take(5)
        .map(|(k, &c)| {
            (
                display
                    .get(k)
                    .cloned()
                    .unwrap_or(StatScalar::String("?".into())),
                c,
            )
        })
        .collect();
    Some(RepetitionStats {
        mode,
        mode_count,
        mode_frequency: mode_count as f64 / present_count as f64,
        top,
    })
}

/// Profiles an ordered list of columns from row-oriented records.
pub fn profile_columns(
    field_order: &[(String, LogicalType)],
    rows: impl IntoIterator<Item = IndexMap<String, FieldValue>>,
    block_rows: u32,
) -> Vec<ColumnStats> {
    let mut acc: Vec<ColumnAccumulator> = field_order
        .iter()
        .map(|(name, ty)| ColumnAccumulator::new(name.clone(), ty.clone(), block_rows))
        .collect();

    for row in rows {
        for (i, (name, _)) in field_order.iter().enumerate() {
            let value = row.get(name).cloned().unwrap_or(FieldValue::Null);
            acc[i].push(&value);
        }
    }

    acc.into_iter().map(ColumnAccumulator::finish).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use strata_core::IntegerWidth;

    #[test]
    fn monotonic_timestamps() {
        let mut acc = ColumnAccumulator::new(
            "ts",
            LogicalType::timestamp(strata_core::TimestampUnit::Nanoseconds),
            4,
        );
        for t in [100i64, 200, 300, 400, 500] {
            acc.push(&FieldValue::Timestamp(t));
        }
        let s = acc.finish();
        assert_eq!(s.present_count, 5);
        assert_eq!(s.cardinality, 5);
        assert!((s.monotonicity.unwrap() - 1.0).abs() < f64::EPSILON);
        assert_eq!(s.median_delta, Some(100.0));
        assert_eq!(s.delta_variance_label, Some(DeltaVarianceLabel::Low));
        assert_eq!(s.min_bit_width, Some(bit_width_u128(400)));
    }

    #[test]
    fn low_cardinality_enum() {
        let mut acc = ColumnAccumulator::new(
            "side",
            LogicalType::enum_type(vec!["buy".into(), "sell".into()]),
            8,
        );
        for v in ["buy", "buy", "sell", "buy", "sell", "sell"] {
            acc.push(&FieldValue::Enum(v.into()));
        }
        let s = acc.finish();
        assert_eq!(s.cardinality, 2);
        assert_eq!(s.dictionary_size, Some(2));
        assert!(s.repetition.unwrap().mode_frequency > 0.4);
        let rl = s.run_length.unwrap();
        assert!(rl.max_run_length >= 2);
    }

    #[test]
    fn narrow_local_ranges() {
        let mut acc = ColumnAccumulator::new(
            "price",
            LogicalType::fixed_decimal(18, 4, IntegerWidth::Bits64),
            3,
        );
        // Global span large, but each block of 3 is tight.
        for v in [1000i64, 1001, 1002, 5000, 5001, 5002, 9000, 9001, 9002] {
            acc.push(&FieldValue::FixedDecimal(v));
        }
        let s = acc.finish();
        let local = s.local_block_ranges.unwrap();
        assert_eq!(local.block_count, 3);
        assert_eq!(local.label, "narrow");
    }

    #[test]
    fn bit_width_constant_column() {
        let mut acc = ColumnAccumulator::new(
            "qty",
            LogicalType::unsigned_integer(IntegerWidth::Bits32),
            8,
        );
        for _ in 0..10 {
            acc.push(&FieldValue::UnsignedInteger(0));
        }
        let s = acc.finish();
        assert_eq!(s.min_bit_width, Some(1));
        assert_eq!(s.absolute_bit_width, Some(1));
        assert_eq!(s.cardinality, 1);
    }
}
