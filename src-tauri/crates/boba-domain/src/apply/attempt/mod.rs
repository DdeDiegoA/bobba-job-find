//! The apply attempt model (S9.3, FR-82/FR-88, AD-12, AD-14).
//!
//! An attempt is the record of one send. It is written **before** the message
//! leaves, so a crash or a lost reply leaves evidence behind instead of a silent
//! second send (AC2). These types cross the `OutboxRepo` port, so they live in
//! the domain (AD-14); the hashing and the rows live in the `boba-store` adapter.

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
    /// The request ended without an answer, or the process died mid-send.
    /// Delivery is unproven, so a resend needs the user's word.
    Uncertain,
}

impl AttemptState {
    pub const ALL: [AttemptState; 4] = [
        AttemptState::Pending,
        AttemptState::Sent,
        AttemptState::Failed,
        AttemptState::Uncertain,
    ];

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
    /// The exact bytes that are sent. The adapter hashes them and drops them
    /// (AD-12: a row holds the blake3 of a payload, never the payload).
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
    /// Boba's id for this message, set when the attempt is recorded; the audit
    /// outcome names it.
    pub message_id: Option<String>,
    pub state: AttemptState,
    /// Provider class, never a body or a token.
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub sent_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_states_round_trip_and_parse_nothing_else() {
        for state in AttemptState::ALL {
            assert_eq!(AttemptState::parse(state.as_str()), Some(state));
        }
        assert_eq!(AttemptState::parse("sending"), None);
        assert_eq!(
            AttemptState::parse("Sent"),
            None,
            "the stored form is lower case"
        );
    }
}
