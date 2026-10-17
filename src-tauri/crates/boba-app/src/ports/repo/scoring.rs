//! `ScoringRepo` — owns `score_card`, `score_narration`, `market_benchmark`
//! (AD-8, AD-16).
//!
//! This slice reads and writes `score_card` only. `score_narration` lands with
//! S6.5 (the run that writes it) and `market_benchmark` with the importer that
//! fills it; both are this repo's tables, so neither needs a new port.
//!
//! The one rule the port enforces is AC3's: **a card is immutable**. A row is
//! written once, and a re-score inserts a *new* row for the new
//! [`CardVersions`] instead of updating the old one, so the history of what the
//! user was told stays readable (FR-13/FR-14).

use boba_domain::scoring::rank::{CardVersions, ScoreCardRow};

/// A storage failure the use case reports with its own message key (AD-3).
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct ScoringRepoError(pub String);

impl ScoringRepoError {
    /// A failure as the adapter describes it.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Read and write ScoreCards.
pub trait ScoringRepo: Send + Sync {
    /// Store one card. The unique key is `(posting_id, profile_version,
    /// prefs_version, model_version)`: storing a key that already exists is a
    /// storage error, not an overwrite (AC3 — cards are never rewritten).
    fn insert_card(&self, row: &ScoreCardRow) -> Result<(), ScoringRepoError>;

    /// The card for one exact key, `None` when this Offer has not been scored
    /// for those versions yet (FR-61: a re-run reuses what is there).
    fn card(
        &self,
        posting_id: &str,
        versions: CardVersions,
    ) -> Result<Option<ScoreCardRow>, ScoringRepoError>;

    /// The newest card for an Offer, whatever versions it was computed for.
    fn latest_card(&self, posting_id: &str) -> Result<Option<ScoreCardRow>, ScoringRepoError>;

    /// Every card an Offer has, oldest first: what a re-score leaves behind
    /// (AC3), and what the Offer detail can show as history.
    fn cards_for(&self, posting_id: &str) -> Result<Vec<ScoreCardRow>, ScoringRepoError>;
}
