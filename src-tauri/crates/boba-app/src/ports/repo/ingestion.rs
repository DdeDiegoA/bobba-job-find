//! `IngestionRepo` — owns `posting`, `posting_source`, `company`,
//! `posting_features` (AD-16, AD-19). Adapter: `boba-store`.
//!
//! Writes are one posting plus its provenance atomically: dedup merges Offers,
//! so a provenance row is appended, never rewritten (AD-19).

use boba_domain::posting::{Offer, Provenance};

/// Errors from the ingestion repository.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IngestionRepoError {
    #[error("store error: {0}")]
    Store(String),
    #[error("invalid offer: {0}")]
    Invalid(String),
}

/// The ingestion owner's write and read surface (AD-16).
pub trait IngestionRepo: Send + Sync {
    /// Store one Offer and the provenance row it came from.
    fn insert_offer(
        &self,
        offer: &Offer,
        provenance: &Provenance,
    ) -> Result<(), IngestionRepoError>;

    fn get_offer(&self, id: &str) -> Result<Option<Offer>, IngestionRepoError>;

    /// Every source payload that contributed to one posting, oldest first.
    fn provenance(&self, posting_id: &str) -> Result<Vec<Provenance>, IngestionRepoError>;
}
