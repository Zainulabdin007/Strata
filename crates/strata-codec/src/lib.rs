//! Encoding and compression for Strata.
//!
//! Semantic encodings are composable via [`EncodingPlan`]. Optional general
//! compressors ([`CompressionCodec::Lz4`], [`CompressionCodec::Zstd`]) wrap
//! semantically encoded block bodies.

#![deny(missing_docs)]

pub mod codecs;
pub mod compress;
pub mod encode;
pub mod error;
pub mod limits;
pub mod nulls;
pub mod pipeline;
pub mod plan;
pub mod planner;
pub mod resolve;
pub mod stream;

pub use codecs::{
    codec_for, decode_step, encode_step, encoded_size, BitPackingCodec, BooleanBitpackCodec, Codec,
    CodecId, DeltaCodec, DeltaOfDeltaCodec, DictionaryEncodingCodec, EnumBitpackCodec,
    FrameOfReferenceCodec, PlainCodec, RunLengthEncodingCodec, StepOutput, VarintCodec,
    ZigZagCodec,
};
pub use compress::{
    compressor_for, lookup_compressor, lookup_compressor_with_level, CompressionCodec, Compressor,
    CompressorId, IdentityCompressor, Lz4Compressor, ZstdCompressor,
};
pub use encode::{ColumnDecoder, ColumnEncoder, EncoderId, PlainEncoder};
pub use error::{Error, Result};
pub use pipeline::{
    decode_column, encode_column, encode_column_refs, EncodedColumn, COLUMN_FORMAT_VERSION,
};
pub use plan::{CodecMetadata, EncodingPlan, EncodingStep, StepMetadata};
pub use planner::{examples as plan_examples, plan_for_field};
pub use resolve::{
    resolve_column_plan, resolve_profile_plans, validate_plan_for_type, validate_profile_encodings,
};
pub use stream::Stream;
