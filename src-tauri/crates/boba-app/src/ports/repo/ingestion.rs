//! `IngestionRepo` — owns `posting`, `posting_source`, `company`,
//! `posting_features` (AD-16, AD-19). Adapter: `boba-store`.
//!
//! Writes are one posting plus its provenance atomically. Dedup merges Offers
//! into one posting with one provenance row per source, and re-seeing the same
//! source item refreshes that row (its url, raw payload and fetch time) rather
//! than adding a copy; a source's row is never deleted (AD-19).

use boba_domain::dedup::DedupProbe;
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

    /// The postings an Offer could already be (S4.3, AD-19): those carrying its
    /// `dedup_key`, those whose canonical apply URL is its canonical apply URL,
    /// and those from the same company — the pool the description simhash is
    /// compared within. It is the caller's job to decide whether any of them is
    /// the same job (`boba_domain::dedup::duplicate_of`).
    fn dedup_candidates(&self, probe: &DedupProbe) -> Result<Vec<Offer>, IngestionRepoError>;

    /// Attach one more source's observation to a posting dedup merged it into,
    /// and refresh `last_seen` (S4.3, AD-19). Every other field of the posting
    /// stays as it was; nothing is deleted.
    fn merge_provenance(
        &self,
        posting_id: &str,
        provenance: &Provenance,
        now: &str,
    ) -> Result<(), IngestionRepoError>;
}
