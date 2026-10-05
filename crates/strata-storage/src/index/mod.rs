//! On-disk block indexes for pruning and point/range lookup.
//!
//! Indexes map values to **blocks** (segment + block id), not individual rows.
//! See [`rebuild_indexes`] and [`DatasetIndexes`].

mod build;
mod catalog;
mod lookup;
mod types;

pub use build::{rebuild_indexes, IndexRebuildReport};
pub use catalog::{IndexCatalog, INDEX_FORMAT_VERSION};
pub use lookup::DatasetIndexes;
pub use types::{
    BlockMinMaxIndex, BlockRef, BlockStats, CategoricalIndex, FieldMinMax, IndexKind, IndexValue,
    LatestVersionEntry, LatestVersionIndex, SequenceIndex, TimeIndex,
};
