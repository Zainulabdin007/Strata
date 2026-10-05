//! One OS-held mutation lock shared by ingest, append, recovery and compaction.
use crate::{Error, Result};
use std::{
    fs::{File, OpenOptions},
    path::Path,
};
/// Acquire the dataset's nonblocking writer lock. Dropping the file releases it.
pub fn lock_dataset(root: &Path) -> Result<File> {
    let path = root.join(".ingest.lock");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
    file.try_lock().map_err(|e| Error::InvalidState {
        message: format!("dataset already has an active writer: {e}"),
    })?;
    Ok(file)
}
