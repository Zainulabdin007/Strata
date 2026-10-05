//! Per-column encoding declaration in a compression profile.

use serde::Deserialize;

use super::profile::FieldEncodingHint;

/// How a column's encoding is declared in TOML.
///
/// Supports both a single hint string and an explicit pipeline array:
///
/// ```toml
/// encoding = "auto"
/// encoding = ["delta_of_delta", "zigzag", "bitpack"]
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum ColumnEncodingSpec {
    /// Single hint (`auto`, `dictionary`, `delta`, `plain`, …).
    Hint(FieldEncodingHint),
    /// Explicit ordered pipeline of codec step names.
    Pipeline(Vec<String>),
}

impl Default for ColumnEncodingSpec {
    fn default() -> Self {
        Self::Hint(FieldEncodingHint::Auto)
    }
}

impl ColumnEncodingSpec {
    /// Returns true when the profile left encoding unspecified / Auto.
    pub fn is_auto(&self) -> bool {
        matches!(self, Self::Hint(FieldEncodingHint::Auto))
    }

    /// Hint form, if this is not an explicit pipeline.
    pub fn as_hint(&self) -> Option<FieldEncodingHint> {
        match self {
            Self::Hint(h) => Some(*h),
            Self::Pipeline(_) => None,
        }
    }

    /// Explicit pipeline step names, if any.
    pub fn as_pipeline(&self) -> Option<&[String]> {
        match self {
            Self::Pipeline(steps) => Some(steps.as_slice()),
            Self::Hint(_) => None,
        }
    }

    /// Human-readable summary for CLI display.
    pub fn display(&self) -> String {
        match self {
            Self::Hint(h) => h.as_str().to_string(),
            Self::Pipeline(steps) => format!("[{}]", steps.join(", ")),
        }
    }
}

impl FieldEncodingHint {
    /// Stable name for diagnostics / display.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Dictionary => "dictionary",
            Self::RunLength => "run_length",
            Self::Delta => "delta",
            Self::BitPack => "bitpack",
            Self::Plain => "plain",
        }
    }
}
