//! In-memory `SourcesRepo` for manual-intake tests (test-only).
//!
//! The intake registers the `manual` source row before it stores an Offer (the
//! `posting.source_id` foreign key needs it), so the fakes here mirror just that
//! part of the store's contract. The real SQL is tested in `boba-store`.

use std::sync::{Arc, Mutex};

use boba_domain::source::{OptInRecord, RunStatus, SourceRecord, SourceRunRecord};

use crate::ports::repo::sources::{SourcesRepo, SourcesRepoError};

#[derive(Default, Clone)]
pub struct FakeSourcesRepo {
    /// Shared: the service takes an `Arc<dyn SourcesRepo>`, the test keeps this.
    state: Arc<Mutex<Vec<SourceRecord>>>,
}

impl FakeSourcesRepo {
    fn rows(&self) -> std::sync::MutexGuard<'_, Vec<SourceRecord>> {
        self.state.lock().unwrap()
    }

    /// Where the `manual` row was created, if it was.
    pub fn get(&self, id: &str) -> Option<SourceRecord> {
        self.rows().iter().find(|row| row.id == id).cloned()
    }

    /// How many rows exist, so a test can prove the intake registers one only.
    pub fn len(&self) -> usize {
        self.rows().len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows().is_empty()
    }
}

impl SourcesRepo for FakeSourcesRepo {
    fn upsert_source(&self, record: &SourceRecord) -> Result<(), SourcesRepoError> {
        let mut rows = self.rows();
        match rows.iter_mut().find(|row| row.id == record.id) {
            Some(row) => *row = record.clone(),
            None => rows.push(record.clone()),
        }
        Ok(())
    }

    fn get_source(&self, id: &str) -> Result<Option<SourceRecord>, SourcesRepoError> {
        Ok(self.get(id))
    }

    fn list_sources(&self) -> Result<Vec<SourceRecord>, SourcesRepoError> {
        Ok(self.rows().clone())
    }

    fn set_source_enabled(
        &self,
        id: &str,
        enabled: bool,
        updated_at: &str,
    ) -> Result<(), SourcesRepoError> {
        let mut rows = self.rows();
        let row = rows
            .iter_mut()
            .find(|row| row.id == id)
            .ok_or_else(|| SourcesRepoError::Invalid(id.to_string()))?;
        row.enabled = enabled;
        row.updated_at = updated_at.to_string();
        Ok(())
    }

    fn record_optin(&self, _optin: &OptInRecord) -> Result<(), SourcesRepoError> {
        Ok(())
    }

    fn get_optin(&self, _source_id: &str) -> Result<Option<OptInRecord>, SourcesRepoError> {
        Ok(None)
    }

    fn remove_optin(&self, _source_id: &str) -> Result<(), SourcesRepoError> {
        Ok(())
    }

    fn opted_in_ids(&self) -> Result<Vec<String>, SourcesRepoError> {
        Ok(Vec::new())
    }

    fn start_run(&self, _run: &SourceRunRecord) -> Result<(), SourcesRepoError> {
        Ok(())
    }

    fn finish_run(
        &self,
        _id: &str,
        _finished_at: &str,
        _status: RunStatus,
        _postings_found: i64,
        _error: Option<&str>,
    ) -> Result<(), SourcesRepoError> {
        Ok(())
    }
}
