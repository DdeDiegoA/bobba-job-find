//! `StatusProposalRepo` — owns `email_classification` and `status_proposal`
//! (S8.4, FR-100..FR-103, AD-16). Adapter: `boba-store`.
//!
//! These rows are the status-proposal area's: the classification is what one
//! stored message was read as, and a proposal is the offer to move one
//! Application because of it. The `email_message` rows themselves belong to the
//! email area (S8.3); this port only reads them.

use boba_domain::mail::{ClassificationMethod, ProposalState, ReplyClass, StatusProposal};

/// One stored message, as classification reads it. The body is plain text
/// (AD-7): the `.eml` importer and the sync both stripped the markup already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredMessage {
    pub id: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub subject: String,
    pub body_text: String,
    /// UTC RFC 3339 (AD-17).
    pub received_at: String,
}

/// The verdict on one message (AC1).
#[derive(Debug, Clone, PartialEq)]
pub struct Classification {
    pub message_id: String,
    pub class: ReplyClass,
    pub confidence: f64,
    /// The message line the class was read from, verbatim.
    pub quoted_line: String,
    pub method: ClassificationMethod,
    /// ADR-010: `true` when the class or the quote came from the LLM.
    pub untrusted_origin: bool,
    pub classified_at: String,
}

/// A repo-level failure, stringified so the port stays free of the adapter's
/// error types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("status proposal store error: {0}")]
pub struct StatusProposalRepoError(pub String);

/// The status-proposal owner area (AD-16).
pub trait StatusProposalRepo: Send + Sync {
    /// Every stored message no classifier has read yet, oldest first. A message
    /// classified once is never handed out again: re-classifying it would ask
    /// the user the same question twice.
    fn messages_unclassified(&self) -> Result<Vec<StoredMessage>, StatusProposalRepoError>;

    /// One stored message by id.
    fn message_get(&self, id: &str) -> Result<Option<StoredMessage>, StatusProposalRepoError>;

    /// Store (or replace) one classification. Keyed by message: one message has
    /// one verdict.
    fn classification_put(
        &self,
        classification: &Classification,
    ) -> Result<(), StatusProposalRepoError>;

    /// Insert one proposal. A message that already has a proposal (pending,
    /// approved, dismissed) is left as it is; the returned bool says whether a
    /// row was written.
    fn proposal_insert(&self, proposal: &StatusProposal) -> Result<bool, StatusProposalRepoError>;

    /// The proposals in one state, newest first for the queue.
    fn proposals(
        &self,
        state: ProposalState,
    ) -> Result<Vec<StatusProposal>, StatusProposalRepoError>;

    /// One proposal by id, `None` when unknown.
    fn proposal_get(&self, id: &str) -> Result<Option<StatusProposal>, StatusProposalRepoError>;

    /// Record a decision. `application_id` and `from_status` are written when
    /// they were unknown at proposal time and the user or the use case resolved
    /// them at approval (AC2). Returns whether a row changed.
    fn proposal_decide(
        &self,
        id: &str,
        application_id: Option<&str>,
        from_status: Option<&str>,
        state: ProposalState,
        decided_at: &str,
    ) -> Result<bool, StatusProposalRepoError>;
}
