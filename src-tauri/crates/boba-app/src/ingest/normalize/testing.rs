//! In-memory `IngestionRepo` for use-case tests (test-only).
//!
//! It mirrors the store's contract — provenance rows accumulate, Offers are
//! fetched whole, features are cached per (posting, profile version) with the
//! agent-pass marker — without SQLite. The real SQL lives in `boba-store`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use boba_domain::dedup::{DedupProbe, probe_matches};
use boba_domain::posting::lifecycle::{
    ClosedWarning, PostingLifecycle, is_past_deadline, is_repost,
};
use boba_domain::posting::{Offer, Provenance};
use boba_domain::posting_features::{Claim, PendingPosting, PostingFeatures};

use crate::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};

/// One posting's cache entry, as the two tables together hold it.
#[derive(Clone)]
struct CacheEntry {
    features: PostingFeatures,
    agent_done: bool,
}

#[derive(Default)]
pub struct FakeIngestionRepo {
    offers: Mutex<BTreeMap<String, Offer>>,
    provenance: Mutex<Vec<Provenance>>,
    features: Mutex<HashMap<(String, u32), CacheEntry>>,
    deterministic_saves: AtomicUsize,
    lifecycle: Mutex<BTreeMap<String, PostingLifecycle>>,
    saved_applications: Mutex<Vec<(String, String)>>,
}

impl FakeIngestionRepo {
    /// Attach a saved Application to a posting, as the apply owner's table
    /// would hold it (S4.4, AC3).
    pub fn add_saved_application(&self, posting_id: &str, application_id: &str) {
        self.saved_applications
            .lock()
            .unwrap()
            .push((posting_id.to_string(), application_id.to_string()));
    }
}

impl FakeIngestionRepo {
    /// The claims stored for one (posting, profile version), without the cache
    /// marker: what an assertion about the store's rows reads.
    pub fn stored_claims(&self, posting_id: &str, profile_version: u32) -> Vec<Claim> {
        self.features
            .lock()
            .unwrap()
            .get(&(posting_id.to_string(), profile_version))
            .map(|entry| entry.features.claims.clone())
            .unwrap_or_default()
    }

    /// How many times the deterministic pass was stored, so a test can prove a
    /// cached version is never extracted twice (FR-61).
    pub fn deterministic_saves(&self) -> usize {
        self.deterministic_saves.load(Ordering::Relaxed)
    }

    fn entry(&self, posting_id: &str, profile_version: u32) -> Option<CacheEntry> {
        self.features
            .lock()
            .unwrap()
            .get(&(posting_id.to_string(), profile_version))
            .cloned()
    }

    fn put(&self, features: &PostingFeatures, extracted_at: &str, agent_done: Option<bool>) {
        let mut cache = self.features.lock().unwrap();
        let entry = cache
            .entry((features.posting_id.clone(), features.profile_version))
            .or_insert_with(|| CacheEntry {
                features: PostingFeatures::new(
                    features.posting_id.clone(),
                    features.profile_version,
                ),
                agent_done: false,
            });
        // The store upserts claims by identity; so does this.
        for claim in &features.claims {
            entry
                .features
                .claims
                .retain(|stored| !stored.same_as(claim));
            entry.features.claims.push(claim.clone());
        }
        entry.features.sort();
        let _ = extracted_at;
        if let Some(done) = agent_done {
            entry.agent_done = entry.agent_done || done;
        }
    }
}

impl IngestionRepo for FakeIngestionRepo {
    fn insert_offer(
        &self,
        offer: &Offer,
        provenance: &Provenance,
    ) -> Result<(), IngestionRepoError> {
        self.offers
            .lock()
            .unwrap()
            .insert(offer.id.clone(), offer.clone());
        self.provenance.lock().unwrap().push(provenance.clone());
        // The store opens a posting's lifecycle row in the same transaction
        // (S4.4); the fake mirrors that, staleness check included.
        self.lifecycle
            .lock()
            .unwrap()
            .entry(offer.id.clone())
            .or_insert_with(|| PostingLifecycle::new(&offer.id, offer, &offer.first_seen));
        Ok(())
    }

    fn get_offer(&self, id: &str) -> Result<Option<Offer>, IngestionRepoError> {
        Ok(self.offers.lock().unwrap().get(id).cloned())
    }

    /// The fake holds the whole table, so a page is the same sort the store's
    /// `ORDER BY first_seen DESC, id ASC` gives, plus a window.
    fn offers(&self, limit: usize, offset: usize) -> Result<Vec<Offer>, IngestionRepoError> {
        let mut all: Vec<Offer> = self.offers.lock().unwrap().values().cloned().collect();
        all.sort_by(|a, b| b.first_seen.cmp(&a.first_seen).then(a.id.cmp(&b.id)));
        Ok(all.into_iter().skip(offset).take(limit).collect())
    }

    /// One entry per Source, in source-id order (as the store's
    /// `GROUP BY source_id ORDER BY source_id` gives them).
    fn offer_sources(&self, posting_id: &str) -> Result<Vec<String>, IngestionRepoError> {
        let mut sources: Vec<String> = self
            .provenance
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.posting_id == posting_id)
            .map(|p| p.source_id.clone())
            .collect();
        sources.sort();
        sources.dedup();
        Ok(sources)
    }

    fn provenance(&self, posting_id: &str) -> Result<Vec<Provenance>, IngestionRepoError> {
        Ok(self
            .provenance
            .lock()
            .unwrap()
            .iter()
            .filter(|p| p.posting_id == posting_id)
            .cloned()
            .collect())
    }

    /// The fake is the whole table, so being a candidate is decided by the same
    /// domain rule the real store applies row by row.
    fn dedup_candidates(&self, probe: &DedupProbe) -> Result<Vec<Offer>, IngestionRepoError> {
        Ok(self
            .offers
            .lock()
            .unwrap()
            .values()
            .filter(|offer| probe_matches(probe, offer))
            .cloned()
            .collect())
    }

    fn merge_provenance(
        &self,
        posting_id: &str,
        provenance: &Provenance,
        now: &str,
    ) -> Result<(), IngestionRepoError> {
        let posting = {
            let mut offers = self.offers.lock().unwrap();
            offers.get_mut(posting_id).map(|posting| {
                posting.last_seen = now.to_string();
                posting.clone()
            })
        };
        self.provenance.lock().unwrap().push(provenance.clone());
        // The store does the same two things on a re-sight (S4.4): the posting
        // is present again, and coming back inside the window is a repost.
        if let Some(posting) = posting {
            let mut rows = self.lifecycle.lock().unwrap();
            if let Some(row) = rows.get_mut(posting_id) {
                if let Some(disappeared_at) = row.disappeared_at.clone()
                    && is_repost(&disappeared_at, now)
                {
                    row.repost_at = Some(now.to_string());
                }
                row.missing_runs = 0;
                row.disappeared_at = None;
                // Deadline closures stay closed; the store does the same.
                if !is_past_deadline(&posting, now) {
                    row.possibly_closed_at = None;
                }
                row.updated_at = now.to_string();
            }
        }
        Ok(())
    }

    fn features(
        &self,
        posting_id: &str,
        profile_version: u32,
    ) -> Result<Option<PostingFeatures>, IngestionRepoError> {
        Ok(self.entry(posting_id, profile_version).map(|e| e.features))
    }

    fn features_agent_done(
        &self,
        posting_id: &str,
        profile_version: u32,
    ) -> Result<bool, IngestionRepoError> {
        Ok(self
            .entry(posting_id, profile_version)
            .is_some_and(|entry| entry.agent_done))
    }

    fn save_deterministic_features(
        &self,
        features: &PostingFeatures,
        extracted_at: &str,
    ) -> Result<(), IngestionRepoError> {
        self.deterministic_saves.fetch_add(1, Ordering::Relaxed);
        self.put(features, extracted_at, None);
        // S4.2's `requirements_json` is this pass's to fill.
        let mut offers = self.offers.lock().unwrap();
        if let Some(offer) = offers.get_mut(&features.posting_id) {
            offer.requirements = features.skill_values();
        }
        Ok(())
    }

    fn save_agent_features(
        &self,
        features: &PostingFeatures,
        extracted_at: &str,
    ) -> Result<(), IngestionRepoError> {
        self.put(features, extracted_at, Some(true));
        Ok(())
    }

    fn features_pending(
        &self,
        profile_version: u32,
        limit: usize,
    ) -> Result<Vec<PendingPosting>, IngestionRepoError> {
        let mut pending: Vec<PendingPosting> = self
            .offers
            .lock()
            .unwrap()
            .values()
            .filter(|offer| {
                !self
                    .entry(&offer.id, profile_version)
                    .is_some_and(|entry| entry.agent_done)
            })
            .map(|offer| PendingPosting {
                id: offer.id.clone(),
                title: offer.title.clone(),
                company: offer.company.clone(),
                lang: offer.lang.clone(),
                text: offer.description_text.clone(),
            })
            .collect();
        // Newest first, as the store's `ORDER BY first_seen DESC` gives them.
        let seen: BTreeMap<String, String> = self
            .offers
            .lock()
            .unwrap()
            .iter()
            .map(|(id, offer)| (id.clone(), offer.first_seen.clone()))
            .collect();
        pending.sort_by(|a, b| seen[&b.id].cmp(&seen[&a.id]).then(a.id.cmp(&b.id)));
        pending.truncate(limit);
        Ok(pending)
    }

    fn lifecycle(&self, posting_id: &str) -> Result<Option<PostingLifecycle>, IngestionRepoError> {
        Ok(self.lifecycle.lock().unwrap().get(posting_id).cloned())
    }

    fn lifecycle_many(
        &self,
        posting_ids: &[String],
    ) -> Result<HashMap<String, PostingLifecycle>, IngestionRepoError> {
        let rows = self.lifecycle.lock().unwrap();
        Ok(posting_ids
            .iter()
            .filter_map(|id| rows.get(id).map(|row| (id.clone(), row.clone())))
            .collect())
    }

    fn save_lifecycle(&self, lifecycle: &PostingLifecycle) -> Result<(), IngestionRepoError> {
        self.lifecycle
            .lock()
            .unwrap()
            .insert(lifecycle.posting_id.clone(), lifecycle.clone());
        Ok(())
    }

    /// The provenance rows decide it, as the store's query does: a posting is
    /// seen by a run when one of the Sources the run covered observed it at or
    /// after the run started (S4.4, FR-52).
    fn postings_missing_from_run(
        &self,
        source_ids: &[String],
        started_at: &str,
    ) -> Result<Vec<Offer>, IngestionRepoError> {
        let offers = self.offers.lock().unwrap();
        let provenance = self.provenance.lock().unwrap();
        Ok(offers
            .values()
            .filter(|offer| {
                let covered =
                    |p: &&Provenance| p.posting_id == offer.id && source_ids.contains(&p.source_id);
                provenance.iter().filter(covered).count() > 0
                    && !provenance
                        .iter()
                        .filter(covered)
                        .any(|p| p.fetched_at.as_str() >= started_at)
            })
            .cloned()
            .collect())
    }

    fn postings_seen_by_run(
        &self,
        source_ids: &[String],
        since: &str,
    ) -> Result<Vec<Offer>, IngestionRepoError> {
        let offers = self.offers.lock().unwrap();
        let provenance = self.provenance.lock().unwrap();
        Ok(offers
            .values()
            .filter(|offer| {
                provenance.iter().any(|p| {
                    p.posting_id == offer.id
                        && source_ids.contains(&p.source_id)
                        && p.fetched_at.as_str() >= since
                })
            })
            .cloned()
            .collect())
    }

    fn possibly_closed_with_applications(&self) -> Result<Vec<ClosedWarning>, IngestionRepoError> {
        let lifecycle = self.lifecycle.lock().unwrap();
        let applications = self.saved_applications.lock().unwrap();
        Ok(lifecycle
            .values()
            .filter(|row| row.is_possibly_closed())
            .filter_map(|row| {
                let application_ids: Vec<String> = applications
                    .iter()
                    .filter(|(posting_id, _)| posting_id == &row.posting_id)
                    .map(|(_, application_id)| application_id.clone())
                    .collect();
                (!application_ids.is_empty()).then(|| ClosedWarning {
                    posting_id: row.posting_id.clone(),
                    application_ids,
                })
            })
            .collect())
    }
}

impl FakeIngestionRepo {
    /// Every posting the fake holds — dedup's whole point is that three sources
    /// make one.
    pub fn postings(&self) -> Vec<Offer> {
        self.offers.lock().unwrap().values().cloned().collect()
    }
}
