//! `TriageRepo` — owns `triage_dismissal`, `triage_override` and
//! `triage_suggestion` (S6.6, FR-54, FR-66, FR-67).
//!
//! The store keeps what the user decided in the Inbox: why they dismissed an
//! Offer, which VETO they overruled, and which Preferences change they accepted
//! or ignored. The use case decides what those rows mean; this port is only the
//! surface it reads and writes them through.

use boba_domain::triage::{
    DismissalRecord, SuggestionDecision, SuggestionDecisionStatus, VetoOverride,
};

/// A repo-level failure, stringified so the port stays free of the adapter's
/// error types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("triage store error: {0}")]
pub struct TriageRepoError(pub String);

impl TriageRepoError {
    /// Wrap one adapter error.
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The triage owner's read and write surface.
pub trait TriageRepo: Send + Sync {
    /// Record the reason an Offer was dismissed (FR-66). Dismissing the same
    /// Offer again replaces its reason.
    fn dismiss(&self, record: &DismissalRecord) -> Result<(), TriageRepoError>;

    /// Forget the reason for one Offer (Undo). Returns whether a row was
    /// removed: an id with no reason is `false`, not an error.
    fn undo_dismiss(&self, posting_id: &str) -> Result<bool, TriageRepoError>;

    /// Every reason on record, newest first — what the suggestion counts and
    /// the Excluded list read.
    fn dismissals(&self) -> Result<Vec<DismissalRecord>, TriageRepoError>;

    /// Record that the user overruled a VETO (FR-54). Recording the same Offer
    /// twice is the same decision, not an error.
    fn override_veto(&self, row: &VetoOverride) -> Result<(), TriageRepoError>;

    /// Every override on record, oldest first.
    fn overrides(&self) -> Result<Vec<VetoOverride>, TriageRepoError>;

    /// What the user did with a suggestion (FR-67), `None` while undecided.
    fn suggestion_decision(&self, id: &str) -> Result<Option<SuggestionDecision>, TriageRepoError>;

    /// Record the user's answer to a suggestion; a later answer replaces it.
    fn save_suggestion_decision(&self, row: &SuggestionDecision) -> Result<(), TriageRepoError>;

    /// The status of one suggestion, `None` while undecided. A convenience so a
    /// caller need not clone the whole row.
    fn suggestion_status(
        &self,
        id: &str,
    ) -> Result<Option<SuggestionDecisionStatus>, TriageRepoError> {
        Ok(self.suggestion_decision(id)?.map(|row| row.status))
    }
}
