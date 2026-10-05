//! Resource limits for untrusted codec payloads.
use crate::error::{Error, Result};
/// Maximum values in one decoded column (also enforced by writers).
pub const MAX_VALUES: usize = 1_048_576;
/// Maximum expanded byte payload for one codec operation.
pub const MAX_DECODE_BYTES: usize = 256 * 1024 * 1024;
/// Reject untrusted counts before allocation.
pub fn check_count(count: usize) -> Result<()> {
    if count > MAX_VALUES {
        return Err(Error::Decode {
            message: "value count exceeds safety limit".into(),
        });
    }
    Ok(())
}
/// Reject untrusted decompressed sizes before allocation.
pub fn check_bytes(count: usize) -> Result<()> {
    if count > MAX_DECODE_BYTES {
        return Err(Error::Decode {
            message: "decoded bytes exceed safety limit".into(),
        });
    }
    Ok(())
}
