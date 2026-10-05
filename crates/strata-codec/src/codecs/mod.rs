//! Semantic codec implementations.

mod bit_packing;
mod boolean_bitpack;
mod delta;
mod delta_of_delta;
mod dictionary;
mod enum_bitpack;
mod frame_of_reference;
mod plain;
mod rle;
mod varint;
mod zigzag;

use crate::error::Result;
use crate::plan::{EncodingStep, StepMetadata};
use crate::stream::Stream;

/// Stable codec identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CodecId(pub &'static str);

/// Result of applying one encoding step.
#[derive(Debug, Clone)]
pub struct StepOutput {
    /// Output stream (may still be intermediate).
    pub stream: Stream,
    /// Optional step metadata required for lossless decode.
    pub meta: Option<StepMetadata>,
}

impl StepOutput {
    /// Byte size of the output stream.
    pub fn encoded_size(&self) -> usize {
        encoded_size(&self.stream)
    }

    /// Metadata emitted by this step, if any.
    pub fn metadata(&self) -> Option<&StepMetadata> {
        self.meta.as_ref()
    }
}

/// Lossless semantic codec: transform an intermediate [`Stream`].
pub trait Codec: Send + Sync {
    /// Stable step identity.
    fn id(&self) -> EncodingStep;

    /// Encodes `input` → output stream (+ optional metadata).
    fn encode(&self, input: Stream) -> Result<StepOutput>;

    /// Decodes `input` using optional step metadata.
    fn decode(&self, input: Stream, meta: Option<&StepMetadata>) -> Result<Stream>;

    /// Size of an encode result.
    fn encoded_size(&self, output: &StepOutput) -> usize {
        output.encoded_size()
    }

    /// Metadata from an encode result.
    fn metadata<'a>(&self, output: &'a StepOutput) -> Option<&'a StepMetadata> {
        output.metadata()
    }
}

macro_rules! define_codec {
    ($name:ident, $step:ident, $mod:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Default, Clone, Copy)]
        pub struct $name;

        impl Codec for $name {
            fn id(&self) -> EncodingStep {
                EncodingStep::$step
            }

            fn encode(&self, input: Stream) -> Result<StepOutput> {
                $mod::encode(input)
            }

            fn decode(&self, input: Stream, meta: Option<&StepMetadata>) -> Result<Stream> {
                $mod::decode(input, meta)
            }
        }
    };
}

define_codec!(PlainCodec, Plain, plain, "PLAIN serializer codec.");
define_codec!(
    DeltaCodec,
    Delta,
    delta,
    "Adjacent-difference (delta) codec."
);
define_codec!(
    DeltaOfDeltaCodec,
    DeltaOfDelta,
    delta_of_delta,
    "Second-order delta codec."
);
define_codec!(
    ZigZagCodec,
    ZigZag,
    zigzag,
    "Signed ↔ zigzag unsigned codec."
);
define_codec!(VarintCodec, Varint, varint, "LEB128 unsigned varint codec.");
define_codec!(
    FrameOfReferenceCodec,
    FrameOfReference,
    frame_of_reference,
    "Frame-of-reference codec."
);
define_codec!(
    BitPackingCodec,
    BitPacking,
    bit_packing,
    "Dense bit-packing codec."
);
define_codec!(
    RunLengthEncodingCodec,
    RunLengthEncoding,
    rle,
    "Run-length encoding codec."
);
define_codec!(
    DictionaryEncodingCodec,
    DictionaryEncoding,
    dictionary,
    "Dictionary encoding codec."
);
define_codec!(
    BooleanBitpackCodec,
    BooleanBitpack,
    boolean_bitpack,
    "1-bit boolean packing codec."
);
define_codec!(
    EnumBitpackCodec,
    EnumBitpack,
    enum_bitpack,
    "Enum variant bit-packing codec."
);

/// Looks up a codec implementation by step.
pub fn codec_for(step: EncodingStep) -> &'static dyn Codec {
    match step {
        EncodingStep::Plain => &PlainCodec,
        EncodingStep::Delta => &DeltaCodec,
        EncodingStep::DeltaOfDelta => &DeltaOfDeltaCodec,
        EncodingStep::ZigZag => &ZigZagCodec,
        EncodingStep::Varint => &VarintCodec,
        EncodingStep::FrameOfReference => &FrameOfReferenceCodec,
        EncodingStep::BitPacking => &BitPackingCodec,
        EncodingStep::RunLengthEncoding => &RunLengthEncodingCodec,
        EncodingStep::DictionaryEncoding => &DictionaryEncodingCodec,
        EncodingStep::BooleanBitpack => &BooleanBitpackCodec,
        EncodingStep::EnumBitpack => &EnumBitpackCodec,
    }
}

/// Applies a single encoding step.
pub fn encode_step(step: EncodingStep, input: Stream) -> Result<StepOutput> {
    codec_for(step).encode(input)
}

/// Reverses a single encoding step.
pub fn decode_step(
    step: EncodingStep,
    input: Stream,
    meta: Option<&StepMetadata>,
) -> Result<Stream> {
    codec_for(step).decode(input, meta)
}

/// Returns the byte size of an encoded stream (or estimates for intermediates).
pub fn encoded_size(stream: &Stream) -> usize {
    match stream {
        Stream::Encoded(b) => b.len(),
        Stream::I64(v) => v.len() * 8,
        Stream::U64(v) => v.len() * 8,
        Stream::Bool(v) => v.len().div_ceil(8),
        Stream::Bytes(v) => v.iter().map(|b| 4 + b.len()).sum(),
    }
}

/// Reverse a terminal serializer directly from borrowed column bytes.
pub(crate) fn decode_step_bytes(
    step: EncodingStep,
    bytes: &[u8],
    meta: Option<&StepMetadata>,
) -> Result<Stream> {
    match step {
        EncodingStep::Plain => plain::decode_bytes(bytes, meta),
        EncodingStep::Varint => varint::decode_bytes(bytes, meta),
        EncodingStep::EnumBitpack => enum_bitpack::decode_bytes(bytes, meta),
        EncodingStep::BooleanBitpack => boolean_bitpack::decode_bytes(bytes, meta),
        EncodingStep::BitPacking => bit_packing::decode_bytes(bytes, meta),
        EncodingStep::RunLengthEncoding => rle::decode_bytes(bytes, meta),
        _ => Err(crate::Error::Decode {
            message: "column must end in a serializer".into(),
        }),
    }
}
