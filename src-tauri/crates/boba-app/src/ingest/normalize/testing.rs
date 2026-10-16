//! In-memory `IngestionRepo` for use-case tests (test-only).
//!
//! It mirrors the store's contract — provenance rows accumulate, Offers are
//! fetched whole, features are cached per (posting, profile version) with the
//! agent-pass marker — without SQLite. The real SQL lives in `boba-store`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use boba_domain::dedup::{DedupProbe, probe_matches};
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
        Ok(())
    }

    fn get_offer(&self, id: &str) -> Result<Option<Offer>, IngestionRepoError> {
        Ok(self.offers.lock().unwrap().get(id).cloned())
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
        if let Some(posting) = self.offers.lock().unwrap().get_mut(posting_id) {
            posting.last_seen = now.to_string();
        }
        self.provenance.lock().unwrap().push(provenance.clone());
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
}

impl FakeIngestionRepo {
    /// Every posting the fake holds — dedup's whole point is that three sources
    /// make one.
    pub fn postings(&self) -> Vec<Offer> {
        self.offers.lock().unwrap().values().cloned().collect()
    }
}
