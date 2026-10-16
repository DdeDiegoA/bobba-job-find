//! `ProfileRepo` — owns `profile`, `profile_fact`, `profile_fact_proposal`
//! (AD-15, AD-16). Only this area writes those tables; other modules read
//! through its queries. The adapter is `boba-store`.
//!
//! A fact is immutable (AD-15): the port can **append** a revision, never update
//! or delete one. `proposal_delete` is the one removal, and it only drops a
//! pending proposal the user has confirmed or discarded (S3.5).

use boba_domain::profile::{FactProposal, ProfileFact};
use thiserror::Error;

/// A repo-level failure, stringified so the port stays free of the adapter's
/// error types.
#[derive(Debug, Clone, Error)]
#[error("profile store error: {0}")]
pub struct ProfileRepoError(pub String);

impl ProfileRepoError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The `profile` owner area (AD-16).
pub trait ProfileRepo: Send + Sync {
    /// The current `profile.version`; `0` before the first fact exists (AD-15).
    fn profile_version(&self) -> Result<u32, ProfileRepoError>;

    /// Store the profile version and its `updated_at` (UTC RFC 3339, AD-17).
    fn profile_set_version(&self, version: u32, updated_at: &str) -> Result<(), ProfileRepoError>;

    /// The latest revision of every item, ordered by kind then creation.
    fn facts_current(&self) -> Result<Vec<ProfileFact>, ProfileRepoError>;

    /// The latest revision of one item, `None` when the item is unknown.
    fn fact_latest(&self, item_id: &str) -> Result<Option<ProfileFact>, ProfileRepoError>;

    /// One revision by row id, `None` when unknown.
    fn fact_get(&self, id: &str) -> Result<Option<ProfileFact>, ProfileRepoError>;

    /// Every revision of one item, newest first (History, AC3).
    fn fact_history(&self, item_id: &str) -> Result<Vec<ProfileFact>, ProfileRepoError>;

    /// Append a revision. There is no update and no delete (AD-15).
    fn fact_append(&self, fact: &ProfileFact) -> Result<(), ProfileRepoError>;

    /// Proposals awaiting review, newest first (S3.5, S3.6).
    fn proposals_pending(&self) -> Result<Vec<FactProposal>, ProfileRepoError>;

    /// Store a proposal an agent run made (AC4).
    fn proposal_append(&self, proposal: &FactProposal) -> Result<(), ProfileRepoError>;

    /// Drop a pending proposal the user confirmed or discarded (S3.5). Returns
    /// whether a row was removed.
    fn proposal_delete(&self, id: &str) -> Result<bool, ProfileRepoError>;
}
