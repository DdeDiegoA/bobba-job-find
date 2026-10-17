//! `IngestionRepo` — owns `posting`, `posting_source`, `company`,
//! `posting_features` (AD-16, AD-19). Adapter: `boba-store`.
//!
//! Writes are one posting plus its provenance atomically. Dedup merges Offers
//! into one posting with one provenance row per source, and re-seeing the same
//! source item refreshes that row (its url, raw payload and fetch time) rather
//! than adding a copy; a source's row is never deleted (AD-19).
//!
//! The feature methods (S6.2) are keyed by `(posting, profile version)`, which
//! is the cache FR-61 asks for: a version that has been read is read again from
//! here, never re-extracted.

use std::collections::HashMap;

use boba_domain::dedup::DedupProbe;
use boba_domain::posting::lifecycle::{ClosedWarning, PostingLifecycle};
use boba_domain::posting::{Offer, Provenance};
use boba_domain::posting_features::{PendingPosting, PostingFeatures};

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

    /// One page of every Offer, **newest first** (`first_seen` descending, `id`
    /// ascending to break a tie) — what the Inbox and Today list (S6.4, FR-63).
    ///
    /// A page and not the table: the store returns what the caller asked for, so
    /// a screen decides how much of the list it reads rather than the query
    /// deciding by accident.
    fn offers(&self, limit: usize, offset: usize) -> Result<Vec<Offer>, IngestionRepoError>;

    /// The Sources an Offer was seen on, each once (AD-19). Dedup merges Offers
    /// into one posting with one provenance row per source, so this is the
    /// "Sources" line on an Offer card (FR-64) without reading each row's raw
    /// payload.
    fn offer_sources(&self, posting_id: &str) -> Result<Vec<String>, IngestionRepoError>;

    /// The Sources of a whole page of Offers, keyed by posting id (S5.2; the
    /// S6.4 follow-up that keeps the Inbox from querying once per card).
    ///
    /// The default is the per-Offer read — correct, but not batched; an adapter
    /// overrides it when its storage can answer in one query (`boba-store`
    /// does). An Offer with no Sources has no entry, not an empty one.
    fn offer_sources_many(
        &self,
        posting_ids: &[String],
    ) -> Result<HashMap<String, Vec<String>>, IngestionRepoError> {
        let mut sources = HashMap::with_capacity(posting_ids.len());
        for posting_id in posting_ids {
            sources.insert(posting_id.clone(), self.offer_sources(posting_id)?);
        }
        Ok(sources)
    }

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

    /// The features stored for one (Offer, Profile version), `None` when that
    /// version has never been read. An empty `Some` is a real answer: a posting
    /// with nothing to extract is cached as such (FR-61).
    fn features(
        &self,
        posting_id: &str,
        profile_version: u32,
    ) -> Result<Option<PostingFeatures>, IngestionRepoError>;

    /// Whether an agent pass has been served for one (Offer, Profile version).
    /// A served pass is never run again (FR-61), including one that proposed
    /// nothing.
    fn features_agent_done(
        &self,
        posting_id: &str,
        profile_version: u32,
    ) -> Result<bool, IngestionRepoError>;

    /// Store the deterministic pass for one (Offer, Profile version),
    /// `extracted_at` being UTC RFC 3339 from the caller's clock (AD-17). In one
    /// transaction it upserts the claims, the cache marker and the posting's
    /// `requirements_json` (which S4.2 declared this slice's to fill).
    fn save_deterministic_features(
        &self,
        features: &PostingFeatures,
        extracted_at: &str,
    ) -> Result<(), IngestionRepoError>;

    /// Merge an agent run's claims into one (Offer, Profile version) and mark
    /// the agent pass done — even when the run proposed nothing at all, which is
    /// a real answer too. A deterministic claim is never replaced (AD-5).
    fn save_agent_features(
        &self,
        features: &PostingFeatures,
        extracted_at: &str,
    ) -> Result<(), IngestionRepoError>;

    /// Postings still waiting for an agent pass at this Profile version, newest
    /// first, at most `limit`.
    fn features_pending(
        &self,
        profile_version: u32,
        limit: usize,
    ) -> Result<Vec<PendingPosting>, IngestionRepoError>;

    // --- lifecycle (S4.4, FR-50/51/52) ---------------------------------------

    /// One posting's lifecycle row, `None` for a posting stored before the row
    /// existed (the migration backfills those, so in practice never).
    fn lifecycle(&self, posting_id: &str) -> Result<Option<PostingLifecycle>, IngestionRepoError>;

    /// The lifecycle rows of `posting_ids`, keyed by posting id (S6.6): the
    /// Inbox reads which Offers the user dismissed for the whole page in one
    /// query, never one Offer at a time. Postings without a row are absent.
    fn lifecycle_many(
        &self,
        posting_ids: &[String],
    ) -> Result<HashMap<String, PostingLifecycle>, IngestionRepoError>;

    /// Upsert one posting's lifecycle row (S4.4).
    fn save_lifecycle(&self, lifecycle: &PostingLifecycle) -> Result<(), IngestionRepoError>;

    /// Postings of `source_ids` last observed before `before`: the ones a run
    /// that started at that instant did not see (S4.4, FR-52).
    fn postings_missing_from_run(
        &self,
        source_ids: &[String],
        before: &str,
    ) -> Result<Vec<Offer>, IngestionRepoError>;

    /// Postings of `source_ids` observed at or after `since` (S4.4, FR-52).
    fn postings_seen_by_run(
        &self,
        source_ids: &[String],
        since: &str,
    ) -> Result<Vec<Offer>, IngestionRepoError>;

    /// Every *Possibly closed* posting with the Applications saved against it,
    /// so the tracker can warn (S4.4, FR-52). A read-only projection over the
    /// apply owner's table; nothing here writes it (AD-16).
    fn possibly_closed_with_applications(&self) -> Result<Vec<ClosedWarning>, IngestionRepoError>;
}
