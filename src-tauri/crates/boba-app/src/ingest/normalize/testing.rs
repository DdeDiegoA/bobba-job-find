//! In-memory `IngestionRepo` for use-case tests (test-only).
//!
//! It mirrors the store's contract — provenance rows accumulate, Offers are
//! fetched whole — without SQLite. The real SQL lives in `boba-store`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use boba_domain::dedup::{DedupProbe, probe_matches};
use boba_domain::posting::{Offer, Provenance};

use crate::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};

#[derive(Default)]
pub struct FakeIngestionRepo {
    offers: Mutex<BTreeMap<String, Offer>>,
    provenance: Mutex<Vec<Provenance>>,
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
}

impl FakeIngestionRepo {
    /// Every posting the fake holds — dedup's whole point is that three sources
    /// make one.
    pub fn postings(&self) -> Vec<Offer> {
        self.offers.lock().unwrap().values().cloned().collect()
    }
}
