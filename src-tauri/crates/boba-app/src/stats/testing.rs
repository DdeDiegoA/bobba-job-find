//! In-memory `SourcesRepo` for the stats tests (test-only).
//!
//! The manual-intake fake does not keep opt-ins, and the Audit Log view reads
//! them (FR-91). This decorator delegates every source call to it and keeps the
//! `source_optin` rows itself, so a test can seed an acceptance with its warning
//! version.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use boba_domain::source::{OptInRecord, RunStatus, SourceRecord, SourceRunRecord};

use crate::ingest::manual::testing::FakeSourcesRepo;
use crate::ports::repo::sources::{SourcesRepo, SourcesRepoError};

#[derive(Default, Clone)]
pub struct OptInSources {
    base: FakeSourcesRepo,
    optins: Arc<Mutex<BTreeMap<String, OptInRecord>>>,
}

impl SourcesRepo for OptInSources {
    fn upsert_source(&self, record: &SourceRecord) -> Result<(), SourcesRepoError> {
        self.base.upsert_source(record)
    }

    fn get_source(&self, id: &str) -> Result<Option<SourceRecord>, SourcesRepoError> {
        self.base.get_source(id)
    }

    fn list_sources(&self) -> Result<Vec<SourceRecord>, SourcesRepoError> {
        self.base.list_sources()
    }

    fn set_source_enabled(
        &self,
        id: &str,
        enabled: bool,
        updated_at: &str,
    ) -> Result<(), SourcesRepoError> {
        self.base.set_source_enabled(id, enabled, updated_at)
    }

    fn record_optin(&self, optin: &OptInRecord) -> Result<(), SourcesRepoError> {
        self.optins
            .lock()
            .unwrap()
            .insert(optin.source_id.clone(), optin.clone());
        Ok(())
    }

    fn get_optin(&self, source_id: &str) -> Result<Option<OptInRecord>, SourcesRepoError> {
        Ok(self.optins.lock().unwrap().get(source_id).cloned())
    }

    fn remove_optin(&self, source_id: &str) -> Result<(), SourcesRepoError> {
        self.optins.lock().unwrap().remove(source_id);
        Ok(())
    }

    fn opted_in_ids(&self) -> Result<Vec<String>, SourcesRepoError> {
        Ok(self.optins.lock().unwrap().keys().cloned().collect())
    }

    fn start_run(&self, _: &SourceRunRecord) -> Result<(), SourcesRepoError> {
        Err(SourcesRepoError::Store("read-only fixture".to_string()))
    }

    fn recent_runs(&self, _: &str, _: usize) -> Result<Vec<SourceRunRecord>, SourcesRepoError> {
        Ok(Vec::new())
    }

    fn finish_run(
        &self,
        _: &str,
        _: &str,
        _: RunStatus,
        _: i64,
        _: Option<&str>,
    ) -> Result<(), SourcesRepoError> {
        Err(SourcesRepoError::Store("read-only fixture".to_string()))
    }
}
