//! The sources area's core logic (S4.8): custom RSS feeds and Source health.
//!
//! The adapters live in `boba-sources`; this module holds what both the
//! Connections screen and the daily run need without touching the network:
//! the saved custom feeds and the fold that turns `source_run` history into a
//! tile's health (FR-31).

pub mod custom;
pub mod health;

/// A sources-area failure, mapped to an `errors.sources.*` key (AD-3, AD-21).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourcesError {
    #[error("store error: {0}")]
    Store(String),
    #[error("invalid sources data: {0}")]
    Invalid(String),
}

impl SourcesError {
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Store(_) => "errors.sources.store",
            Self::Invalid(_) => "errors.sources.invalid",
        }
    }
}
