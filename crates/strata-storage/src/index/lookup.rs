//! Load and query rebuilt indexes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::catalog::IndexCatalog;
use super::types::{
    BlockMinMaxIndex, BlockRef, CategoricalIndex, IndexKind, LatestVersionIndex, SequenceIndex,
    TimeIndex,
};
use crate::error::{Error, Result};
use crate::path::DatasetLayout;

/// Opened dataset indexes for lookup / pruning.
#[derive(Debug, Clone)]
pub struct DatasetIndexes {
    root: PathBuf,
    catalog: IndexCatalog,
    block_minmax: Option<BlockMinMaxIndex>,
    sequence: BTreeMap<String, SequenceIndex>,
    time: BTreeMap<String, TimeIndex>,
    categorical: BTreeMap<String, CategoricalIndex>,
    latest_version: Option<LatestVersionIndex>,
}

impl DatasetIndexes {
    /// Loads indexes from `<dataset>/indexes/`. Returns an error if missing.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let layout = DatasetLayout::new(root.as_ref());
        let catalog_path = layout.indexes_dir().join(IndexCatalog::FILE_NAME);
        if !catalog_path.exists() {
            return Err(Error::InvalidState {
                message: format!(
                    "indexes not found at {}; run `strata index rebuild`",
                    layout.indexes_dir().display()
                ),
            });
        }
        let catalog: IndexCatalog = read_json(&catalog_path)?;

        if catalog.version != super::catalog::INDEX_FORMAT_VERSION
            || catalog.dataset_crc
                != Some(strata_format::crc32(&crate::io::read_file(
                    &layout.manifest_path(),
                )?))
        {
            return Err(Error::corrupt(
                &catalog_path,
                "stale or unsupported index catalog; rebuild required",
            ));
        }
        let mut block_minmax = None;
        let mut sequence = BTreeMap::new();
        let mut time = BTreeMap::new();
        let mut categorical = BTreeMap::new();
        let mut latest_version = None;

        for art in &catalog.artifacts {
            if art.path.is_empty()
                || !Path::new(&art.path)
                    .components()
                    .all(|c| matches!(c, std::path::Component::Normal(_)))
            {
                return Err(Error::corrupt(&catalog_path, "invalid index artifact path"));
            }
            let path = layout.indexes_dir().join(&art.path);
            let canonical = path.canonicalize().map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            let base = layout
                .indexes_dir()
                .canonicalize()
                .map_err(|source| Error::Io {
                    path: layout.indexes_dir(),
                    source,
                })?;
            if !canonical.starts_with(&base) {
                return Err(Error::corrupt(&path, "index path escapes dataset"));
            }
            if catalog.checksums.get(&art.path).copied()
                != Some(strata_format::crc32(&crate::io::read_file_with_limit(
                    &path,
                    64 * 1024 * 1024,
                )?))
            {
                return Err(Error::corrupt(&path, "index checksum mismatch"));
            }
            match art.kind {
                IndexKind::BlockMinMax => {
                    block_minmax = Some(read_json(&path)?);
                }
                IndexKind::Sequence => {
                    let idx: SequenceIndex = read_json(&path)?;
                    sequence.insert(idx.field.clone(), idx);
                }
                IndexKind::Time => {
                    let idx: TimeIndex = read_json(&path)?;
                    time.insert(idx.field.clone(), idx);
                }
                IndexKind::Categorical => {
                    let idx: CategoricalIndex = read_json(&path)?;
                    categorical.insert(idx.field.clone(), idx);
                }
                IndexKind::LatestVersion => {
                    latest_version = Some(read_json(&path)?);
                }
            }
        }

        Ok(Self {
            root: layout.root,
            catalog,
            block_minmax,
            sequence,
            time,
            categorical,
            latest_version,
        })
    }

    /// Dataset root.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Index catalog.
    pub fn catalog(&self) -> &IndexCatalog {
        &self.catalog
    }

    /// Block min/max statistics, if present.
    pub fn block_minmax(&self) -> Option<&BlockMinMaxIndex> {
        self.block_minmax.as_ref()
    }

    /// Latest-version index for keyed datasets, if present.
    pub fn latest_version(&self) -> Option<&LatestVersionIndex> {
        self.latest_version.as_ref()
    }

    /// Sequence index fields available.
    pub fn sequence_fields(&self) -> impl Iterator<Item = &str> {
        self.sequence.keys().map(|s| s.as_str())
    }

    /// Time index fields available.
    pub fn time_fields(&self) -> impl Iterator<Item = &str> {
        self.time.keys().map(|s| s.as_str())
    }

    /// Categorical index fields available.
    pub fn categorical_fields(&self) -> impl Iterator<Item = &str> {
        self.categorical.keys().map(|s| s.as_str())
    }

    /// Locate blocks that may contain `seq` for a sequence field.
    ///
    /// Returns every block whose `[min, max]` covers `seq` (zone-map semantics:
    /// no false negatives; rare false positives if ranges overlap).
    pub fn lookup_sequence(&self, field: &str, seq: u64) -> Result<Vec<BlockRef>> {
        let idx = self
            .sequence
            .get(field)
            .ok_or_else(|| Error::InvalidState {
                message: format!("no sequence index for field `{field}`"),
            })?;
        Ok(idx
            .entries
            .iter()
            .filter(|e| e.min <= seq && seq <= e.max)
            .map(|e| e.block.clone())
            .collect())
    }

    /// Locate blocks whose timestamp range overlaps `[start, end]` (inclusive).
    pub fn lookup_time_range(&self, field: &str, start: i64, end: i64) -> Result<Vec<BlockRef>> {
        let idx = self.time.get(field).ok_or_else(|| Error::InvalidState {
            message: format!("no time index for field `{field}`"),
        })?;
        let (lo, hi) = if start <= end {
            (start, end)
        } else {
            (end, start)
        };
        Ok(idx
            .entries
            .iter()
            .filter(|e| e.min <= hi && e.max >= lo)
            .map(|e| e.block.clone())
            .collect())
    }

    /// Locate blocks that contain the categorical / enum value.
    pub fn lookup_categorical(&self, field: &str, value: &str) -> Result<Vec<BlockRef>> {
        let idx = self
            .categorical
            .get(field)
            .ok_or_else(|| Error::InvalidState {
                message: format!("no categorical index for field `{field}`"),
            })?;
        Ok(idx
            .postings
            .iter()
            .find(|p| p.value == value)
            .map(|p| p.blocks.clone())
            .unwrap_or_default())
    }

    /// Prune blocks using min/max for an unsigned field: keep blocks that may
    /// contain any value in `[min, max]`.
    pub fn prune_unsigned_range(&self, field: &str, min: u64, max: u64) -> Result<Vec<BlockRef>> {
        let stats = self
            .block_minmax
            .as_ref()
            .ok_or_else(|| Error::InvalidState {
                message: "block min/max index missing".into(),
            })?;
        let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
        let mut out = Vec::new();
        for block in &stats.blocks {
            if let Some(fm) = block.fields.iter().find(|f| f.field == field) {
                let (Some(bmin), Some(bmax)) = (
                    fm.min.as_ref().and_then(|v| v.as_u64()),
                    fm.max.as_ref().and_then(|v| v.as_u64()),
                ) else {
                    continue;
                };
                if bmin <= hi && bmax >= lo {
                    out.push(block.block.clone());
                }
            }
        }
        Ok(out)
    }

    /// Prune blocks using min/max for a signed / timestamp field.
    pub fn prune_signed_range(&self, field: &str, min: i64, max: i64) -> Result<Vec<BlockRef>> {
        let stats = self
            .block_minmax
            .as_ref()
            .ok_or_else(|| Error::InvalidState {
                message: "block min/max index missing".into(),
            })?;
        let (lo, hi) = if min <= max { (min, max) } else { (max, min) };
        let mut out = Vec::new();
        for block in &stats.blocks {
            if let Some(fm) = block.fields.iter().find(|f| f.field == field) {
                let (Some(bmin), Some(bmax)) = (
                    fm.min.as_ref().and_then(|v| v.as_i64()),
                    fm.max.as_ref().and_then(|v| v.as_i64()),
                ) else {
                    continue;
                };
                if bmin <= hi && bmax >= lo {
                    out.push(block.block.clone());
                }
            }
        }
        Ok(out)
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let bytes = crate::io::read_file_with_limit(path, 64 * 1024 * 1024)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(Error::corrupt(path, "index exceeds 64 MiB"));
    }
    serde_json::from_slice(&bytes)
        .map_err(|e| Error::corrupt(path, format!("invalid index JSON: {e}")))
}
