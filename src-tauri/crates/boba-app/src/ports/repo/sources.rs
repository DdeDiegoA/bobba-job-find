//! `SourcesRepo` — owns `source`, `source_run`, `source_optin` (AD-9, AD-16).
//! Adapter: `boba-store`.
//!
//! Bounds are intentionally absent for now: `boba_store::db::Store` is `!Sync`
//! (its writer holds an `mpsc::Sender`), so a `Send + Sync` supertrait would not
//! compile. Add the bound in the slice that shares the repo across async tasks.

use boba_domain::source::{OptInRecord, RunStatus, SourceRecord, SourceRunRecord};

/// Errors from the sources repository.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourcesRepoError {
    #[error("store error: {0}")]
    Store(String),
    #[error("invalid source record: {0}")]
    Invalid(String),
}

/// The sources owner's write and read surface (AD-16).
pub trait SourcesRepo {
    /// Insert or refresh one configured source.
    fn upsert_source(&self, record: &SourceRecord) -> Result<(), SourcesRepoError>;

    fn get_source(&self, id: &str) -> Result<Option<SourceRecord>, SourcesRepoError>;

    fn list_sources(&self) -> Result<Vec<SourceRecord>, SourcesRepoError>;

    /// Enable/disable a source without changing its manifest.
    fn set_source_enabled(
        &self,
        id: &str,
        enabled: bool,
        updated_at: &str,
    ) -> Result<(), SourcesRepoError>;

    /// Record (or refresh) the user's opt-in for one gated source (AD-9).
    fn record_optin(&self, optin: &OptInRecord) -> Result<(), SourcesRepoError>;

    fn get_optin(&self, source_id: &str) -> Result<Option<OptInRecord>, SourcesRepoError>;

    /// Ids of every source with a recorded opt-in; the registry gates on this.
    fn opted_in_ids(&self) -> Result<Vec<String>, SourcesRepoError>;

    fn start_run(&self, run: &SourceRunRecord) -> Result<(), SourcesRepoError>;

    fn finish_run(
        &self,
        id: &str,
        finished_at: &str,
        status: RunStatus,
        postings_found: i64,
        error: Option<&str>,
    ) -> Result<(), SourcesRepoError>;
}
