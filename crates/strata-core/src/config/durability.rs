//! Explicit durability modes and their documented guarantees.
//!
//! See also the repository root `DURABILITY.md` for the full contract.

use serde::{Deserialize, Serialize};

/// How aggressively Strata persists acknowledged data to stable storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurabilityMode {
    /// Prioritize ingest speed. Data may be lost on crash.
    ///
    /// **Guarantee:** None for recently written records. Valid earlier segments
    /// that were fully finalized are readable after recovery, but the active
    /// segment and any buffered blocks may be truncated.
    #[default]
    Fast,

    /// Periodically flush/sync with a bounded loss window.
    ///
    /// **Guarantee:** After a successful acknowledgment, at most
    /// `sync_every_blocks` blocks (the configured sync window) of the active
    /// segment may be lost on crash. Fully finalized segments survive.
    Durable,

    /// Acknowledged completed data must survive a process crash.
    ///
    /// **Guarantee:** Once Strata acknowledges `N` records under STRICT,
    /// a crash after that acknowledgment must leave those `N` records
    /// recoverable (via `strata recover` if the active segment was open).
    /// Incomplete in-flight block payloads never corrupt earlier committed blocks.
    Strict,
}

impl DurabilityMode {
    /// Stable name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Durable => "durable",
            Self::Strict => "strict",
        }
    }

    /// Parse a CLI / TOML name.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "fast" => Some(Self::Fast),
            "durable" => Some(Self::Durable),
            "strict" => Some(Self::Strict),
            _ => None,
        }
    }

    /// One-line guarantee summary for CLI help / verify output.
    pub const fn guarantee_summary(self) -> &'static str {
        match self {
            Self::Fast => "no durability for recent writes; finalized segments remain recoverable",
            Self::Durable => {
                "acked data survives except at most one sync window of the active segment"
            }
            Self::Strict => {
                "acked records survive process crash; incomplete tails never corrupt prior blocks"
            }
        }
    }
}

/// Optional `[durability]` section inside a compression profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurabilityConfig {
    /// Selected mode (default `fast`).
    #[serde(default)]
    pub mode: DurabilityMode,
    /// For [`DurabilityMode::Durable`]: sync after this many blocks (default 16).
    #[serde(default = "default_sync_every_blocks")]
    pub sync_every_blocks: u32,
}

fn default_sync_every_blocks() -> u32 {
    16
}

impl Default for DurabilityConfig {
    fn default() -> Self {
        Self {
            mode: DurabilityMode::Fast,
            sync_every_blocks: default_sync_every_blocks(),
        }
    }
}
