//! Dataset directory layout helpers.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// Path to a single segment file within a dataset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentPath {
    /// Absolute or relative filesystem path.
    pub path: PathBuf,
}

impl SegmentPath {
    /// Creates a segment path from any path-like value.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Returns the file name if present.
    pub fn file_name(&self) -> Option<&str> {
        self.path.file_name().and_then(|s| s.to_str())
    }
}

/// Conventional directory layout for a Strata dataset on disk.
///
/// ```text
/// <root>/
///   manifest.json
///   segments/
///     000000.strata
///     000001.strata
///   indexes/
///     manifest.json
///     block_minmax.json
///     sequence/
///     time/
///     categorical/
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetLayout {
    /// Dataset root directory.
    pub root: PathBuf,
}

impl DatasetLayout {
    /// Creates a layout rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the segments directory path.
    pub fn segments_dir(&self) -> PathBuf {
        self.root.join("segments")
    }

    /// Returns the indexes directory path.
    pub fn indexes_dir(&self) -> PathBuf {
        self.root.join("indexes")
    }

    /// Staging directory used while rebuilding indexes.
    pub fn indexes_building_dir(&self) -> PathBuf {
        self.root.join("indexes.building")
    }

    /// Staging directory for compaction output.
    pub fn compaction_building_dir(&self) -> PathBuf {
        self.root.join("compaction.building")
    }

    /// Backup directory for segments during compaction cutover.
    pub fn segments_old_dir(&self) -> PathBuf {
        self.root.join("segments.old")
    }

    /// Returns the manifest path.
    pub fn manifest_path(&self) -> PathBuf {
        self.root.join("manifest.json")
    }

    /// Builds a segment path for the given ordinal.
    pub fn segment_path(&self, ordinal: u32) -> SegmentPath {
        let name = format!("{ordinal:06}.strata");
        SegmentPath::new(self.segments_dir().join(name))
    }

    /// Ensures the root and segments directories exist.
    pub fn ensure_dirs(&self) -> Result<()> {
        create_dir_all(&self.root)?;
        create_dir_all(&self.segments_dir())?;
        Ok(())
    }
}

fn create_dir_all(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_naming() {
        let layout = DatasetLayout::new("/tmp/dataset");
        let seg = layout.segment_path(42);
        assert_eq!(seg.file_name(), Some("000042.strata"));
        assert_eq!(
            layout.segments_dir(),
            PathBuf::from("/tmp/dataset/segments")
        );
    }
}
