//! `OutboxRepo` — owns `apply_attempt` (S9.3, AC2; AD-12 evidence).
//!
//! An attempt is the record of one send: it is written **before** the message
//! leaves, so a crash or a lost reply leaves evidence behind instead of a silent
//! second send. The payload (recipient, subject, body, attachments) is hashed by
//! the adapter and never stored (AD-12): the row keeps the blake3, the state and
//! the message id.
//!
//! The trait lives with the engine because the engine is its only caller; the
//! adapter is `boba-store::repos::outbox`.

/// Where an attempt stands (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttemptState {
    /// The row exists and the message is on its way (or the process died
    /// between writing the row and recording the result).
    Pending,
    /// The provider accepted the message: the one state that proves a send.
    Sent,
    /// The provider definitely refused it; nothing left the account.
    Failed,
    /// The request ended without an answer (transport, timeout). It may or may
    /// not have been delivered, so a resend needs the user's word.
    Uncertain,
}

impl AttemptState {
    /// The stored text form (mirrors the `apply_attempt.state` CHECK).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Uncertain => "uncertain",
        }
    }

    /// Parse the stored form, `None` for a value this build does not know.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pending" => Some(Self::Pending),
            "sent" => Some(Self::Sent),
            "failed" => Some(Self::Failed),
            "uncertain" => Some(Self::Uncertain),
            _ => None,
        }
    }
}

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

/// One attempt to record before its message leaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewAttempt {
    pub application_id: String,
    /// `email` today; the ATS path (S9.4) adds its own.
    pub channel: String,
    /// The connected mailbox the message is sent from.
    pub account_id: String,
    pub recipient: String,
    pub subject: String,
    /// The exact bytes that are sent. The adapter hashes them and drops them.
    pub payload: Vec<u8>,
}

/// One stored attempt, with the evidence a retry reads (AC2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyAttempt {
    pub id: String,
    pub application_id: String,
    pub channel: String,
    pub account_id: String,
    pub recipient: String,
    pub subject: String,
    /// blake3 of the payload — the body itself is never stored (AD-12).
    pub payload_blake3: String,
    /// Boba's id for this message, set once the attempt is recorded; the audit
    /// outcome names it. `None` only for a row a crash left half-written.
    pub message_id: Option<String>,
    pub state: AttemptState,
    /// Provider class, never a body or a token.
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub sent_at: Option<String>,
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
}
