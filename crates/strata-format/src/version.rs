//! Format version constants.

use serde::{Deserialize, Serialize};

/// Current Strata on-disk format version.
pub const FORMAT_VERSION: FormatVersion = FormatVersion { major: 0, minor: 2 };

/// Semantic version of the Strata segment format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FormatVersion {
    /// Incompatible format changes bump major.
    pub major: u16,
    /// Compatible additions bump minor.
    pub minor: u16,
}

impl FormatVersion {
    /// Returns true when `other` can be read by a reader that understands `self`.
    pub const fn is_compatible_with(self, other: FormatVersion) -> bool {
        self.major == other.major && self.minor >= other.minor
    }

    /// Formats as `major.minor`.
    pub fn as_str(self) -> String {
        format!("{}.{}", self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility() {
        let v = FormatVersion { major: 0, minor: 2 };
        assert!(v.is_compatible_with(FormatVersion { major: 0, minor: 1 }));
        assert!(!v.is_compatible_with(FormatVersion { major: 1, minor: 0 }));
    }
}
