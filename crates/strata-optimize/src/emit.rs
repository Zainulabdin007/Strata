//! Emit a CompressionProfile TOML from an optimized plan.

use strata_codec::EncodingPlan;
use strata_core::{
    CompressionCodecName, CompressionProfile, IntegerWidth, NumericBound, SemanticHint,
    TimestampUnit,
};

use crate::error::Result;
use crate::report::ColumnDecision;

/// Renders a full profile TOML using `base` metadata and optimized encodings.
pub fn emit_profile_toml(
    base: &CompressionProfile,
    decisions: &[ColumnDecision],
    profile_name: &str,
) -> Result<String> {
    let mut out = String::new();
    out.push_str(&format!("name = \"{}\"\n", escape(profile_name)));
    out.push_str(&format!(
        "input_format = \"{}\"\n\n",
        base.input_format.as_str()
    ));

    out.push_str("[input]\n");
    out.push_str(&format!(
        "recursive = {}\n",
        bool_toml(base.input.recursive)
    ));
    if !base.input.extensions.is_empty() {
        out.push_str("extensions = [");
        out.push_str(
            &base
                .input
                .extensions
                .iter()
                .map(|e| format!("\"{}\"", escape(e)))
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push_str("]\n");
    }
    out.push('\n');

    for (key, values) in [
        ("include_globs", &base.input.include_globs),
        ("exclude_globs", &base.input.exclude_globs),
    ] {
        if !values.is_empty() {
            out.push_str(&format!(
                "{key} = [{}]\n",
                values
                    .iter()
                    .map(|v| format!("\"{}\"", escape(v)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    out.push_str("[schema]\n");
    out.push_str(&format!("id = \"{}\"\n", escape(&base.schema.id)));
    if let Some(desc) = &base.schema.description {
        out.push_str(&format!("description = \"{}\"\n", escape(desc)));
    }
    out.push('\n');

    if !base.schema.key.is_empty() {
        out.push_str("key = [");
        out.push_str(
            &base
                .schema
                .key
                .iter()
                .map(|v| format!("\"{}\"", escape(v)))
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push_str("]\n\n");
    }
    for column in &base.columns {
        let decision = decisions.iter().find(|d| d.name == column.name);
        out.push_str("[[columns]]\n");
        out.push_str(&format!("name = \"{}\"\n", escape(&column.name)));
        out.push_str(&format!(
            "logical_type = \"{}\"\n",
            escape(&column.logical_type)
        ));
        out.push_str(&format!("nullable = {}\n", bool_toml(column.nullable)));
        if let Some(w) = column.width {
            out.push_str(&format!("width = \"{}\"\n", width_str(w)));
        }
        if let Some(u) = column.unit {
            out.push_str(&format!("unit = \"{}\"\n", unit_str(u)));
        }
        if let Some(p) = column.precision {
            out.push_str(&format!("precision = {p}\n"));
        }
        if let Some(s) = column.scale {
            out.push_str(&format!("scale = {s}\n"));
        }
        if let Some(m) = column.max_bytes {
            out.push_str(&format!("max_bytes = {m}\n"));
        }
        if !column.variants.is_empty() {
            out.push_str("variants = [");
            out.push_str(
                &column
                    .variants
                    .iter()
                    .map(|v| format!("\"{}\"", escape(v)))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push_str("]\n");
        }
        let hints = decision
            .map(|d| d.hints.as_slice())
            .unwrap_or(column.hints.as_slice());
        if !hints.is_empty() {
            out.push_str("hints = [");
            out.push_str(
                &hints
                    .iter()
                    .map(|h| format!("\"{}\"", hint_str(*h)))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            out.push_str("]\n");
        }
        if let Some(min) = column.expected_min {
            out.push_str(&format!("expected_min = {}\n", bound_toml(min)));
        }
        if let Some(max) = column.expected_max {
            out.push_str(&format!("expected_max = {}\n", bound_toml(max)));
        }

        let plan =
            decision
                .map(|d| &d.selected.plan)
                .ok_or_else(|| crate::error::Error::Invalid {
                    message: format!("missing decision for {}", column.name),
                })?;
        out.push_str(&format!("encoding = {}\n\n", pipeline_toml(plan)));
    }

    out.push_str("[block]\n");
    out.push_str(&format!("row_count = {}\n\n", base.block.row_count));

    out.push_str("[segment]\n");
    out.push_str(&format!(
        "target_size_bytes = {}\n\n",
        base.segment.target_size_bytes
    ));

    out.push_str("[compression]\n");
    out.push_str(&format!(
        "codec = \"{}\"\n",
        codec_str(base.compression.codec)
    ));
    if let Some(level) = base.compression.level {
        out.push_str(&format!("level = {level}\n"));
    }

    out.push_str(&format!(
        "\n[durability]\nmode = \"{}\"\nsync_every_blocks = {}\n",
        base.durability.mode.as_str(),
        base.durability.sync_every_blocks
    ));
    Ok(out)
}

fn pipeline_toml(plan: &EncodingPlan) -> String {
    let steps: Vec<String> = plan
        .steps
        .iter()
        .map(|s| format!("\"{}\"", profile_step_name(*s)))
        .collect();
    format!("[{}]", steps.join(", "))
}

fn profile_step_name(step: strata_codec::EncodingStep) -> &'static str {
    use strata_codec::EncodingStep::*;
    // Prefer the short aliases used in example profiles.
    match step {
        Plain => "plain",
        Delta => "delta",
        DeltaOfDelta => "delta_of_delta",
        ZigZag => "zigzag",
        Varint => "varint",
        FrameOfReference => "frame_of_reference",
        BitPacking => "bitpack",
        RunLengthEncoding => "run_length",
        DictionaryEncoding => "dictionary",
        BooleanBitpack => "boolean_bitpack",
        EnumBitpack => "enum_bitpack",
    }
}

fn width_str(w: IntegerWidth) -> &'static str {
    match w {
        IntegerWidth::Bits8 => "bits8",
        IntegerWidth::Bits16 => "bits16",
        IntegerWidth::Bits32 => "bits32",
        IntegerWidth::Bits64 => "bits64",
    }
}

fn unit_str(u: TimestampUnit) -> &'static str {
    match u {
        TimestampUnit::Seconds => "seconds",
        TimestampUnit::Milliseconds => "milliseconds",
        TimestampUnit::Microseconds => "microseconds",
        TimestampUnit::Nanoseconds => "nanoseconds",
    }
}

fn hint_str(h: SemanticHint) -> &'static str {
    match h {
        SemanticHint::Monotonic => "monotonic",
        SemanticHint::Sorted => "sorted",
        SemanticHint::LowCardinality => "low_cardinality",
        SemanticHint::DeltaFriendly => "delta_friendly",
        SemanticHint::FixedPrecision => "fixed_precision",
    }
}

fn codec_str(c: CompressionCodecName) -> &'static str {
    match c {
        CompressionCodecName::None => "none",
        CompressionCodecName::Lz4 => "lz4",
        CompressionCodecName::Zstd => "zstd",
    }
}

fn bound_toml(b: NumericBound) -> String {
    match b {
        NumericBound::Integer(v) => v.to_string(),
        NumericBound::Float(v) => {
            if (v - v.round()).abs() < f64::EPSILON {
                format!("{}", v as i64)
            } else {
                format!("{v}")
            }
        }
    }
}

fn bool_toml(v: bool) -> &'static str {
    if v {
        "true"
    } else {
        "false"
    }
}

fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\\' => "\\\\".into(),
            '"' => "\\\"".into(),
            '\n' => "\\n".into(),
            '\r' => "\\r".into(),
            '\t' => "\\t".into(),
            c if c.is_control() => format!("\\u{:04x}", c as u32),
            c => c.to_string(),
        })
        .collect()
}
