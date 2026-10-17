//! The Outbox hold model (S9.7, FR-84/FR-85, AD-14).
//!
//! These types cross the `OutboxHoldRepo` port, so they live in the domain
//! (AD-14). Two ideas live here, and both are policy the core owns:
//!
//! * **the hold** — an automatic send waits before it leaves, and the user can
//!   still cancel or edit it during that wait (FR-85);
//! * **the dry run** — a new Auto Rule's first matches are only reported, never
//!   sent, until the user activates the rule (FR-84).

/// The hold FR-85 fixes for every channel.
pub const DEFAULT_HOLD_MINUTES: u16 = 30;
/// The shortest hold the PRD allows (FR-85: minimum 5 minutes).
pub const MIN_HOLD_MINUTES: u16 = 5;
/// The longest hold this build accepts: a day is already "when you come back".
pub const MAX_HOLD_MINUTES: u16 = 24 * 60;
/// A Session Board send never waits less than this (FR-91).
pub const SESSION_BOARD_MIN_HOLD_MINUTES: u16 = 60;
/// The dry run FR-84 fixes: the first 3 matches.
pub const DEFAULT_DRY_RUN_MATCHES: u8 = 3;
/// The shortest dry run (FR-84: minimum 1).
pub const MIN_DRY_RUN_MATCHES: u8 = 1;
/// The longest dry run this build accepts.
pub const MAX_DRY_RUN_MATCHES: u8 = 10;
/// The shortest gap between two automatic sends to the same Source — the
/// anti-ban spacing (FR-91). S9.6's 5–20 minute gap is cross-rule; this one is
/// per Source and is the floor the Outbox release enforces.
pub const MIN_SOURCE_GAP_MINUTES: u32 = 5;
/// The per-Source gap a Session Board send keeps (FR-91: at least 10 minutes).
pub const SESSION_BOARD_MIN_SOURCE_GAP_MINUTES: u32 = 10;

/// Where one Outbox item stands (FR-84, FR-85).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxState {
    /// The rule is still in its dry run: shown as "would send", never sent.
    WouldSend,
    /// Waiting out the hold; the user can edit or cancel it.
    Held,
    /// The hold ended and the policy said Auto: the channel is about to send.
    Releasing,
    /// The channel confirmed the send.
    Sent,
    /// The user cancelled it during the hold, or the dry run ended.
    Cancelled,
    /// It left the automatic path: the user reviews and sends it by hand.
    NeedsReview,
}

impl OutboxState {
    pub const ALL: [OutboxState; 6] = [
        OutboxState::WouldSend,
        OutboxState::Held,
        OutboxState::Releasing,
        OutboxState::Sent,
        OutboxState::Cancelled,
        OutboxState::NeedsReview,
    ];

    /// The stored text form (mirrors the `apply_outbox.state` CHECK).
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WouldSend => "would_send",
            Self::Held => "held",
            Self::Releasing => "releasing",
            Self::Sent => "sent",
            Self::Cancelled => "cancelled",
            Self::NeedsReview => "needs_review",
        }
    }

    /// Parse the stored form, `None` for a value this build does not know.
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|state| state.as_str() == text)
    }

    /// Whether the item still occupies the application's one live slot.
    pub fn is_live(self) -> bool {
        matches!(self, Self::WouldSend | Self::Held | Self::Releasing)
    }
}

/// The minimum gap before another automatic send to `channel` may leave (FR-91).
pub fn min_source_gap_minutes(channel: &str) -> u32 {
    if channel == "session_board" {
        SESSION_BOARD_MIN_SOURCE_GAP_MINUTES
    } else {
        MIN_SOURCE_GAP_MINUTES
    }
}

/// The effective wait before a send of `channel` leaves (FR-85, FR-91).
///
/// A Session Board send is the one channel the PRD protects harder: it never
/// waits less than an hour, whatever the configured hold says.
pub fn effective_hold_minutes(channel: &str, configured: u16) -> u16 {
    if channel == "session_board" {
        configured.max(SESSION_BOARD_MIN_HOLD_MINUTES)
    } else {
        configured
    }
}

/// Why a hold this build refuses (FR-85).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HoldViolation {
    #[error("the hold must be at least {MIN_HOLD_MINUTES} minutes")]
    TooShort,
    #[error("the hold must be at most {MAX_HOLD_MINUTES} minutes")]
    TooLong,
}

/// Validate a configured hold; core rejects it, the UI never is the gate.
pub fn validate_hold_minutes(minutes: u16) -> Result<u16, HoldViolation> {
    if minutes < MIN_HOLD_MINUTES {
        return Err(HoldViolation::TooShort);
    }
    if minutes > MAX_HOLD_MINUTES {
        return Err(HoldViolation::TooLong);
    }
    Ok(minutes)
}

/// Why a dry-run length this build refuses (FR-84).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DryRunViolation {
    #[error("the dry run must be at least {MIN_DRY_RUN_MATCHES} match")]
    TooShort,
    #[error("the dry run must be at most {MAX_DRY_RUN_MATCHES} matches")]
    TooLong,
}

/// Validate a dry-run length; core rejects it, the UI never is the gate.
pub fn validate_dry_run_matches(matches: u8) -> Result<u8, DryRunViolation> {
    if matches < MIN_DRY_RUN_MATCHES {
        return Err(DryRunViolation::TooShort);
    }
    if matches > MAX_DRY_RUN_MATCHES {
        return Err(DryRunViolation::TooLong);
    }
    Ok(matches)
}

/// One item one application queued for the Outbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewOutboxItem {
    pub application_id: String,
    /// The rule whose decision queued it.
    pub rule_id: String,
    pub source_id: String,
    /// The rule channel's stored form (`email`, `ats_form`, …).
    pub channel: String,
    /// The employer key the per-company cap groups on.
    pub company_key: String,
    /// The Offer's title and company, for the list and the tray.
    pub title: String,
    pub company: String,
    /// `would_send` while the rule is in its dry run, `held` afterwards.
    pub state: OutboxState,
    /// The configured hold (the channel's floor is applied before it lands).
    pub hold_minutes: u16,
    /// When the hold ends (RFC 3339, UTC).
    pub release_at: String,
}

/// One stored Outbox item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxItem {
    pub id: String,
    pub application_id: String,
    pub rule_id: String,
    pub source_id: String,
    pub channel: String,
    pub company_key: String,
    pub title: String,
    pub company: String,
    pub state: OutboxState,
    pub hold_minutes: u16,
    pub release_at: String,
    /// What the user changed during the hold (FR-85). The send uses them.
    pub subject_override: Option<String>,
    pub body_override: Option<String>,
    /// Why the item left the automatic path, when it did.
    pub note: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// How one rule's dry run stands (FR-84).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunState {
    pub rule_id: String,
    /// How many matches the dry run reports (default 3, minimum 1).
    pub required: u8,
    /// How many matches it has reported so far.
    pub seen: u8,
    /// When the user activated the rule; `None` while it is still a dry run.
    pub activated_at: Option<String>,
}

impl DryRunState {
    /// The compiled default for a rule that never ran.
    pub fn new(rule_id: &str) -> Self {
        Self {
            rule_id: rule_id.to_string(),
            required: DEFAULT_DRY_RUN_MATCHES,
            seen: 0,
            activated_at: None,
        }
    }

    /// Whether the user turned the dry run into a live rule.
    pub fn is_active(&self) -> bool {
        self.activated_at.is_some()
    }

    /// Whether the dry run has reported every match it promises.
    pub fn is_complete(&self) -> bool {
        self.seen >= self.required
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbox_states_round_trip_and_parse_nothing_else() {
        for state in OutboxState::ALL {
            assert_eq!(OutboxState::parse(state.as_str()), Some(state));
        }
        assert_eq!(OutboxState::parse("sending"), None);
        assert_eq!(
            OutboxState::parse("Held"),
            None,
            "the stored form is lower case"
        );
    }

    #[test]
    fn only_the_open_states_are_live() {
        assert!(OutboxState::WouldSend.is_live());
        assert!(OutboxState::Held.is_live());
        assert!(OutboxState::Releasing.is_live());
        assert!(!OutboxState::Sent.is_live());
        assert!(!OutboxState::Cancelled.is_live());
        assert!(!OutboxState::NeedsReview.is_live());
    }

    #[test]
    fn a_hold_is_never_shorter_than_five_minutes_nor_longer_than_a_day() {
        assert_eq!(
            validate_hold_minutes(MIN_HOLD_MINUTES),
            Ok(MIN_HOLD_MINUTES)
        );
        assert_eq!(validate_hold_minutes(DEFAULT_HOLD_MINUTES), Ok(30));
        assert_eq!(
            validate_hold_minutes(MAX_HOLD_MINUTES),
            Ok(MAX_HOLD_MINUTES)
        );
        assert_eq!(validate_hold_minutes(4), Err(HoldViolation::TooShort));
        assert_eq!(validate_hold_minutes(0), Err(HoldViolation::TooShort));
        assert_eq!(
            validate_hold_minutes(MAX_HOLD_MINUTES + 1),
            Err(HoldViolation::TooLong)
        );
    }

    #[test]
    fn a_source_never_takes_two_automatic_sends_back_to_back() {
        assert_eq!(min_source_gap_minutes("email"), 5);
        assert_eq!(min_source_gap_minutes("ats_form"), 5);
        assert_eq!(min_source_gap_minutes("session_board"), 10);
    }

    #[test]
    fn a_session_board_hold_never_drops_below_an_hour() {
        assert_eq!(effective_hold_minutes("session_board", 30), 60);
        assert_eq!(effective_hold_minutes("session_board", 90), 90);
        assert_eq!(effective_hold_minutes("email", 30), 30);
        assert_eq!(effective_hold_minutes("ats_form", 5), 5);
    }

    #[test]
    fn a_dry_run_is_at_least_one_match_and_at_most_ten() {
        assert_eq!(validate_dry_run_matches(MIN_DRY_RUN_MATCHES), Ok(1));
        assert_eq!(validate_dry_run_matches(DEFAULT_DRY_RUN_MATCHES), Ok(3));
        assert_eq!(validate_dry_run_matches(MAX_DRY_RUN_MATCHES), Ok(10));
        assert_eq!(validate_dry_run_matches(0), Err(DryRunViolation::TooShort));
        assert_eq!(
            validate_dry_run_matches(MAX_DRY_RUN_MATCHES + 1),
            Err(DryRunViolation::TooLong)
        );
    }

    #[test]
    fn a_fresh_dry_run_reports_three_matches_and_is_not_active() {
        let state = DryRunState::new("rule_1");
        assert_eq!(state.required, 3);
        assert_eq!(state.seen, 0);
        assert!(!state.is_active());
        assert!(!state.is_complete());
    }

    #[test]
    fn a_complete_dry_run_that_is_still_not_activated_keeps_reporting() {
        let mut state = DryRunState::new("rule_1");
        state.seen = 3;
        assert!(state.is_complete());
        assert!(!state.is_active(), "only the user activates a rule");
    }
}
