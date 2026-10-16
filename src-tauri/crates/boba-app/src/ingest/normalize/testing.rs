//! In-memory `IngestionRepo` for use-case tests (test-only).
//!
//! It mirrors the store's contract — provenance rows accumulate, Offers are
//! fetched whole — without SQLite. The real SQL lives in `boba-store`.

use std::collections::BTreeMap;
use std::sync::Mutex;

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
}
