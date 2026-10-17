//! `OutboxRepo` — owns `apply_attempt` (S9.3, AC2, AD-12).
//!
//! The engine is its only caller and the adapter is
//! `boba-store::repos::outbox`. The row is the evidence a retry reads: it is
//! written before the message leaves, so a lost reply never becomes a second
//! send. The row keeps the payload's blake3, never the payload (AD-12).

use boba_domain::apply::attempt::{ApplyAttempt, AttemptState, NewAttempt};

/// A repo-level failure, stringified so the port stays free of the adapter's
/// error types.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("apply attempt store error: {0}")]
pub struct OutboxRepoError(pub String);

impl OutboxRepoError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// The apply outbox's storage surface (AD-16: apply owns `apply_attempt`).
pub trait OutboxRepo: Send + Sync {
    /// Record an attempt before its message is sent and assign its id, message
    /// id and payload hash. Returns the stored row.
    fn attempt_insert(
        &self,
        attempt: &NewAttempt,
        now: &str,
    ) -> Result<ApplyAttempt, OutboxRepoError>;

    /// Write the result of one attempt (AC2). `message_id` is kept from the
    /// insert; `error` is the provider class only.
    fn attempt_set_state(
        &self,
        id: &str,
        state: AttemptState,
        error: Option<&str>,
        now: &str,
    ) -> Result<(), OutboxRepoError>;

    /// Every attempt of one application, oldest first — the evidence a resend
    /// is checked against.
    fn attempts_for(&self, application_id: &str) -> Result<Vec<ApplyAttempt>, OutboxRepoError>;

    /// Turn every `pending` attempt into `uncertain` and return the rows it
    /// changed (startup recovery, AC2).
    ///
    /// Nothing is in flight when the app starts, so a `pending` row is a send
    /// whose result was lost to a crash. Treating it as failed would claim
    /// delivery did not happen; leaving it pending would refuse every future
    /// send with no way out — so it becomes `uncertain`, which the user's
    /// confirmation can clear.
    fn attempts_reconcile(&self, now: &str) -> Result<Vec<ApplyAttempt>, OutboxRepoError>;
}
