//! `ApplyRepo` — owns `application`, `apply_attempt`, `status_event`, `outbox`,
//! `application_answer_bank` (AD-12, AD-16). Status changes go only through the
//! apply `TransitionStatus` use case, which is the only writer of
//! `application.status` and `status_event`.
//!
//! `application.status` is moved and its `status_event` appended in **one**
//! transaction, and only when the stored status is still the one the caller saw:
//! the check is what makes an approved Status Proposal and a user drag of the
//! same card safe to race (AC4).

use boba_domain::application::{Application, BankAnswer, NewBankAnswer, PackageItem, StatusEvent};

/// A repo-level failure, stringified so the port stays free of the adapter's
/// error types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApplyRepoError {
    #[error("apply store error: {0}")]
    Store(String),
    /// The application does not exist (or was deleted with its Offer).
    #[error("unknown application: {0}")]
    UnknownApplication(String),
    /// The stored status is no longer `expected`: the move was already applied
    /// (or another actor moved the card), so the caller re-reads the board.
    #[error("status moved from {expected} to {found} in the meantime")]
    Conflict { expected: String, found: String },
}

/// The apply owner's read and write surface (AD-16).
pub trait ApplyRepo: Send + Sync {
    /// Every application on the board, oldest first.
    fn applications(&self) -> Result<Vec<Application>, ApplyRepoError>;

    /// One application by id, `None` when unknown.
    fn application_get(&self, id: &str) -> Result<Option<Application>, ApplyRepoError>;

    /// One application's history, oldest first (AC3 timeline).
    fn status_events(&self, application_id: &str) -> Result<Vec<StatusEvent>, ApplyRepoError>;

    /// Every Stage change across all Applications, oldest first (CH12). The
    /// Audit Log's read side needs the whole history at once; asking per
    /// Application made it one query per row the board has ever held.
    fn all_status_events(&self) -> Result<Vec<StatusEvent>, ApplyRepoError>;

    /// The documents this application holds, oldest version first (AC3: the
    /// Package the application carries, read through the documents owner).
    fn package(&self, application_id: &str) -> Result<Vec<PackageItem>, ApplyRepoError>;

    /// Insert one application and its birth event, in one transaction (AC4).
    ///
    /// The row is created where the user first pursues an Offer — Save in the
    /// Inbox (S6.6) — and the birth event is the same history every later move
    /// appends to, so the timeline never starts mid-story.
    fn application_create(
        &self,
        application: &Application,
        event: &StatusEvent,
    ) -> Result<(), ApplyRepoError>;

    /// Append one status event and move the row in one transaction (AC2/AC4).
    ///
    /// `event.from_status` is the status the caller saw: the write fails with
    /// [`ApplyRepoError::Conflict`] when the row already moved. `None` means
    /// "no expectation", which is how the row that records an application's
    /// birth is written.
    fn append_status_event(&self, event: &StatusEvent) -> Result<(), ApplyRepoError>;

    /// Store one drafted screening answer (S7.8, FR-80). The row lands
    /// `untrusted_origin = true` and unapproved: only [`Self::answer_bank_approve`]
    /// makes it reusable (AC2, AC3).
    fn answer_bank_insert(&self, answer: &NewBankAnswer) -> Result<(), ApplyRepoError>;

    /// One application's bank rows, oldest first — the drafts the user still has
    /// to look at and the approved answers it already owns.
    fn answer_bank_for_application(
        &self,
        application_id: &str,
    ) -> Result<Vec<BankAnswer>, ApplyRepoError>;

    /// The approved answers whose question key is one of `keys` (AC2): what the
    /// next drafting run is handed for reuse. Matching is on the whole key, so
    /// an unrelated question never inherits an answer.
    ///
    /// `needs_review` is not a filter here: it marks an answer the user must see
    /// when it is used (AC1), and approving it is the user having seen it.
    fn answer_bank_reusable(&self, keys: &[String]) -> Result<Vec<BankAnswer>, ApplyRepoError>;

    /// Approve one row: `answer` replaces the stored text when the user edited
    /// it, `None` confirms it as it stands. Either way the row is stamped
    /// approved and stops being agent text (AC3). Returns whether a row was
    /// written: an id nobody holds is `false`, not an error, so the caller says
    /// which of the two happened.
    fn answer_bank_approve(
        &self,
        id: &str,
        answer: Option<&str>,
        approved_at: &str,
    ) -> Result<bool, ApplyRepoError>;
}
