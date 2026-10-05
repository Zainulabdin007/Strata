//! Durability sync helpers for segment and dataset writers.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use strata_core::DurabilityMode;

use crate::error::{Error, Result};

/// Applies the durability policy after appending one block.
pub fn sync_after_block(
    file: &mut File,
    path: &Path,
    mode: DurabilityMode,
    blocks_since_sync: &mut u32,
    sync_every_blocks: u32,
) -> Result<bool> {
    match mode {
        DurabilityMode::Fast => Ok(false),
        DurabilityMode::Durable => {
            file.flush().map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;
            *blocks_since_sync = blocks_since_sync.saturating_add(1);
            let every = sync_every_blocks.max(1);
            if *blocks_since_sync >= every {
                file.sync_all().map_err(|source| Error::Io {
                    path: path.to_path_buf(),
                    source,
                })?;
                *blocks_since_sync = 0;
                Ok(true)
            } else {
                Ok(false)
            }
        }
        DurabilityMode::Strict => {
            file.flush().map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;
            file.sync_all().map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })?;
            *blocks_since_sync = 0;
            Ok(true)
        }
    }
}

/// Applies durability policy when finalizing a segment or publishing a manifest.
pub fn sync_commit(file: &mut File, path: &Path, mode: DurabilityMode) -> Result<()> {
    file.flush().map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    match mode {
        DurabilityMode::Fast => Ok(()),
        DurabilityMode::Durable | DurabilityMode::Strict => {
            file.sync_all().map_err(|source| Error::Io {
                path: path.to_path_buf(),
                source,
            })
        }
    }
}

/// Atomically publishes `contents` to `final_path` under `mode`.
pub fn publish_file(final_path: &Path, contents: &[u8], mode: DurabilityMode) -> Result<()> {
    match mode {
        DurabilityMode::Fast => std::fs::write(final_path, contents).map_err(|source| Error::Io {
            path: final_path.to_path_buf(),
            source,
        }),
        DurabilityMode::Durable | DurabilityMode::Strict => {
            let tmp = final_path.with_extension("tmp");
            {
                let mut f = OpenOptions::new()
                    .write(true)
                    .create(true)
                    .truncate(true)
                    .open(&tmp)
                    .map_err(|source| Error::Io {
                        path: tmp.clone(),
                        source,
                    })?;
                f.write_all(contents).map_err(|source| Error::Io {
                    path: tmp.clone(),
                    source,
                })?;
                sync_commit(&mut f, &tmp, mode)?;
            }
            std::fs::rename(&tmp, final_path).map_err(|source| Error::Io {
                path: final_path.to_path_buf(),
                source,
            })?;
            // Propagate directory fsync failures: a successful rename alone is not durable.
            if let Some(parent) = final_path.parent() {
                File::open(parent)
                    .and_then(|dir| dir.sync_all())
                    .map_err(|source| Error::Io {
                        path: parent.to_path_buf(),
                        source,
                    })?;
            }
            Ok(())
        }
    }
}
