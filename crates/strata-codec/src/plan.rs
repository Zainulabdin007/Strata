//! Encoding plans: ordered, composable semantic codec steps.

use crate::error::{Error, Result};

/// One step in a composable encoding pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EncodingStep {
    /// Typed present values with no further transform (serializer).
    Plain = 1,
    /// Adjacent differences.
    Delta = 2,
    /// Second-order differences.
    DeltaOfDelta = 3,
    /// Signed ↔ zigzag unsigned mapping.
    ZigZag = 4,
    /// Unsigned LEB128 varints.
    Varint = 5,
    /// Subtract frame (min) then typically bit-pack.
    FrameOfReference = 6,
    /// Dense bit packing of unsigned integers.
    BitPacking = 7,
    /// Run-length encoding of unsigned integers.
    RunLengthEncoding = 8,
    /// Dictionary for byte/string values → indices.
    DictionaryEncoding = 9,
    /// 1-bit packing for booleans.
    BooleanBitpack = 10,
    /// Enum variants → small indices → bit-pack.
    EnumBitpack = 11,
}

impl EncodingStep {
    /// Stable canonical name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Delta => "delta",
            Self::DeltaOfDelta => "delta_of_delta",
            Self::ZigZag => "zigzag",
            Self::Varint => "varint",
            Self::FrameOfReference => "frame_of_reference",
            Self::BitPacking => "bit_packing",
            Self::RunLengthEncoding => "run_length_encoding",
            Self::DictionaryEncoding => "dictionary_encoding",
            Self::BooleanBitpack => "boolean_bitpack",
            Self::EnumBitpack => "enum_bitpack",
        }
    }

    /// Parses a profile / CLI step name (accepts short aliases).
    ///
    /// Accepted aliases include `bitpack`, `dictionary`, `rle`, `for`, `dod`.
    pub fn parse_name(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "plain" => Some(Self::Plain),
            "delta" => Some(Self::Delta),
            "delta_of_delta" | "dod" => Some(Self::DeltaOfDelta),
            "zigzag" | "zig_zag" => Some(Self::ZigZag),
            "varint" => Some(Self::Varint),
            "frame_of_reference" | "for" | "frame" => Some(Self::FrameOfReference),
            "bit_packing" | "bitpack" | "bit_pack" => Some(Self::BitPacking),
            "run_length_encoding" | "run_length" | "rle" => Some(Self::RunLengthEncoding),
            "dictionary_encoding" | "dictionary" | "dict" => Some(Self::DictionaryEncoding),
            "boolean_bitpack" | "bool_bitpack" | "boolean" => Some(Self::BooleanBitpack),
            "enum_bitpack" | "enum" => Some(Self::EnumBitpack),
            _ => None,
        }
    }

    /// Wire tag.
    pub const fn tag(self) -> u8 {
        self as u8
    }

    /// Parse wire tag.
    pub fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            1 => Some(Self::Plain),
            2 => Some(Self::Delta),
            3 => Some(Self::DeltaOfDelta),
            4 => Some(Self::ZigZag),
            5 => Some(Self::Varint),
            6 => Some(Self::FrameOfReference),
            7 => Some(Self::BitPacking),
            8 => Some(Self::RunLengthEncoding),
            9 => Some(Self::DictionaryEncoding),
            10 => Some(Self::BooleanBitpack),
            11 => Some(Self::EnumBitpack),
            _ => None,
        }
    }

    /// Returns true when this step produces final bytes (terminal serializer).
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Plain
                | Self::Varint
                | Self::BitPacking
                | Self::RunLengthEncoding
                | Self::BooleanBitpack
                | Self::EnumBitpack
        )
    }
}

/// Ordered list of encoding steps applied to one column block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingPlan {
    /// Pipeline steps in application order.
    pub steps: Vec<EncodingStep>,
}

impl EncodingPlan {
    /// Creates a plan from steps.
    pub fn new(steps: impl Into<Vec<EncodingStep>>) -> Result<Self> {
        let steps = steps.into();
        let plan = Self { steps };
        plan.validate()?;
        Ok(plan)
    }

    /// Single-step PLAIN plan.
    pub fn plain() -> Self {
        Self {
            steps: vec![EncodingStep::Plain],
        }
    }

    /// Validates composition rules.
    pub fn validate(&self) -> Result<()> {
        if self.steps.is_empty() || self.steps.len() > 32 {
            return Err(Error::InvalidPlan {
                message: "encoding plan must contain 1..32 steps".into(),
            });
        }
        let last = *self.steps.last().unwrap();
        if !last.is_terminal() {
            return Err(Error::InvalidPlan {
                message: format!(
                    "plan must end with a terminal codec, found `{}`",
                    last.as_str()
                ),
            });
        }
        for step in &self.steps[..self.steps.len() - 1] {
            if step.is_terminal() {
                return Err(Error::InvalidPlan {
                    message: format!("terminal step `{}` must be last in the plan", step.as_str()),
                });
            }
        }
        Ok(())
    }

    /// Serializes the plan to a compact LE byte form (tags only).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + self.steps.len());
        out.push(self.steps.len() as u8);
        for step in &self.steps {
            out.push(step.tag());
        }
        out
    }

    /// Parses a plan from [`to_bytes`](Self::to_bytes).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::Decode {
                message: "empty encoding plan".into(),
            });
        }
        let n = bytes[0] as usize;
        if bytes.len() != 1 + n {
            return Err(Error::Decode {
                message: format!(
                    "encoding plan length mismatch: header {n}, bytes {}",
                    bytes.len()
                ),
            });
        }
        let mut steps = Vec::with_capacity(n);
        for tag in &bytes[1..] {
            let step = EncodingStep::from_tag(*tag).ok_or_else(|| Error::Decode {
                message: format!("unknown encoding step tag {tag}"),
            })?;
            steps.push(step);
        }
        Self::new(steps)
    }
}

/// Per-step metadata written alongside encoded payloads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepMetadata {
    /// Step that produced this metadata.
    pub step: EncodingStep,
    /// Opaque step-specific blob (LE structured by the codec).
    pub blob: Vec<u8>,
}

/// Full metadata for an encoded column body (excluding null bitmap).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodecMetadata {
    /// Plan that was applied.
    pub plan: EncodingPlan,
    /// Per-step metadata in plan order (only steps that emit meta).
    pub steps: Vec<StepMetadata>,
}

impl CodecMetadata {
    /// Total encoded metadata size contribution (plan + step blobs framing).
    pub fn encoded_size(&self) -> usize {
        let mut n = self.plan.to_bytes().len();
        n += 4; // step meta count
        for step in &self.steps {
            n += 1; // tag
            n += 4; // blob len
            n += step.blob.len();
        }
        n
    }
}
