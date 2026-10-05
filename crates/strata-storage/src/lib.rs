//! Dataset, segment, and block I/O for the Strata on-disk format.
//!
//! Values are stored column-wise inside each block using composable semantic
//! encodings (`BlockEncoding::Pipeline`). General compression is not applied
//! in this phase.

#![deny(missing_docs)]

pub mod block;
pub mod compact;
pub mod dataset;
pub mod durability;
pub mod error;
pub mod index;
pub mod io;
pub mod mmap;
pub mod path;
pub mod pipeline;
pub mod recover;
pub mod segment;
pub mod verify;
pub mod versioning;

pub use block::{
    encode_block, probe_block_frame, project_schema, BlockReader, BlockWriter, EncodedBlock,
};
pub use compact::{
    compact_dataset, compact_dataset_simple, compact_options_from_writer, logical_records,
    CompactOptions, CompactReport,
};
pub use dataset::{
    DatasetAppender, DatasetAppenderOptions, DatasetManifest, DatasetReader, DatasetWriter,
    DatasetWriterOptions,
};
pub use error::{Error, Result};
pub use index::{
    rebuild_indexes, BlockMinMaxIndex, BlockRef, BlockStats, CategoricalIndex, DatasetIndexes,
    FieldMinMax, IndexCatalog, IndexKind, IndexRebuildReport, IndexValue, LatestVersionEntry,
    LatestVersionIndex, SequenceIndex, TimeIndex, INDEX_FORMAT_VERSION,
};
pub use mmap::{MappedFile, SealedSegmentBytes, SegmentBytesBackend, SegmentReadMode};
pub use path::{DatasetLayout, SegmentPath};
pub use pipeline::{write_records_parallel, IngestPipeline, PipelineOptions};
pub use recover::{recover_dataset, recover_segment_file, RecoveryReport, SegmentRecovery};
pub use segment::{SegmentReader, SegmentWriter};
pub use verify::{verify_dataset, VerifyIssue, VerifyReport, VerifySeverity};
pub use versioning::{
    ensure_tombstone_field, is_tombstone, make_tombstone, mark_live, resolve_latest,
    validate_key_fields, KeyPart, RecordKey, TOMBSTONE_FIELD,
};

pub mod writer_lock;
