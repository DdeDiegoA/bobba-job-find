//! Session Board Auto policy (S11.3, FR-91, AD-11).
//!
//! Auto on a Session Board Source is the one channel the PRD protects hardest:
//! it needs a **second, separate opt-in** per Source (typing the Source name
//! after a versioned ToS/ban warning), it runs under caps stricter than any
//! other rule, it lapses after 30 days without re-confirmation, and a challenge
//! pauses it for a week. This module is the pure half of that policy: the
//! numbers, the records and the state a send may run in. The store, the clock
//! and the audit log live outside (AD-1); the core only ever decides here.
//!
//! The opt-in row and the pause deadline are stored on the `source` row
//! (`source_auto_optin`, `source.paused_until`, migration 0027), so a closed
//! screen never re-opens Auto: every send re-reads them through the core.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The daily cap Auto starts with on a Session Board Source (FR-91).
pub const DEFAULT_SESSION_DAILY_CAP: u8 = 3;
/// The hard maximum a settings command accepts (FR-91: stricter than FR-83).
pub const MAX_SESSION_DAILY_CAP: u8 = 10;
/// The shortest gap between two automatic submissions on the same Source
/// (FR-91: at least 10 minutes apart).
pub const MIN_SUBMISSION_GAP_MINUTES: u32 = 10;
/// The shortest Outbox hold for a Session Board send (FR-91: at least 60 min).
pub const MIN_HOLD_MINUTES: u16 = 60;
/// Days Auto stays paused after a challenge (FR-91).
pub const CHALLENGE_PAUSE_DAYS: i64 = 7;
/// Days an opt-in stays valid before it must be re-confirmed (FR-91).
pub const RECONFIRM_DAYS: i64 = 30;
/// The Auto warning copy this build accepts. The Auto warning is different
/// copy from the Session Board reading one (S11.1a), so it carries its own
/// version and a change invalidates every stored acceptance.
pub const AUTO_WARNING_VERSION: &str = "v1";

/// The `setting` key holding one Source's Auto daily cap (`3` by default).
///
/// The cap is a preference, not an acceptance: it lives beside the other
/// `apply.auto.*` settings, while the opt-in and the pause sit on the Source
/// row they belong to.
pub fn cap_key(source_id: &str) -> String {
    format!("session.auto.cap.{source_id}")
}

/// Why a daily cap this build refuses (FR-91).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CapViolation {
    #[error("the session daily cap must be at least 1")]
    Zero,
    #[error("the session daily cap must be at most {MAX_SESSION_DAILY_CAP}")]
    TooHigh,
}

/// Validate a configured Session Board daily cap; the core rejects it, the UI
/// never is the gate (FR-91).
pub fn validate_daily_cap(cap: u8) -> Result<u8, CapViolation> {
    if cap == 0 {
        return Err(CapViolation::Zero);
    }
    if cap > MAX_SESSION_DAILY_CAP {
        return Err(CapViolation::TooHigh);
    }
    Ok(cap)
}

/// The cap a stored value resolves to: a malformed or out-of-range row falls
/// back to the conservative default rather than widening Auto (AD-1).
pub fn stored_daily_cap(value: Option<&str>) -> u8 {
    value
        .and_then(|text| text.trim().parse::<u8>().ok())
        .and_then(|cap| validate_daily_cap(cap).ok())
        .unwrap_or(DEFAULT_SESSION_DAILY_CAP)
}

/// Why accepting an Auto opt-in was refused (FR-91).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionAutoRefusal {
    #[error("the typed source name does not match")]
    NameMismatch,
    #[error("the warning changed; re-read version {current}")]
    WarningChanged { current: String },
}

/// One accepted Auto opt-in (FR-91).
///
/// It crosses the `SourcesRepo` port, so it lives in the domain (AD-14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionAutoOptIn {
    pub source_id: String,
    /// The name the user typed: it must equal the Source's own name.
    pub confirmed_name: String,
    /// The Auto warning copy the user read.
    pub warning_version: String,
    /// When it was accepted (UTC RFC 3339).
    pub accepted_at: String,
}

impl SessionAutoOptIn {
    /// Accept the opt-in after the user typed the Source name and read the
    /// current warning (FR-91 (a)+(b)). The core, never the UI, compares both.
    pub fn accept(
        source_id: &str,
        source_name: &str,
        typed_name: &str,
        warning_version: &str,
        now: OffsetDateTime,
    ) -> Result<Self, SessionAutoRefusal> {
        // The copy the user read must be the one this build ships.
        if warning_version != AUTO_WARNING_VERSION {
            return Err(SessionAutoRefusal::WarningChanged {
                current: AUTO_WARNING_VERSION.to_string(),
            });
        }
        if typed_name.trim() != source_name {
            return Err(SessionAutoRefusal::NameMismatch);
        }
        Ok(Self {
            source_id: source_id.to_string(),
            confirmed_name: source_name.to_string(),
            warning_version: warning_version.to_string(),
            accepted_at: format_time(now),
        })
    }

    /// When the opt-in must be re-confirmed (30 days after acceptance).
    pub fn expires_at(&self) -> Option<OffsetDateTime> {
        parse_time(&self.accepted_at)
            .map(|accepted| accepted + time::Duration::days(RECONFIRM_DAYS))
    }

    /// Whether the opt-in still covers `now`; a lapsed one falls back to
    /// Review mode (FR-91).
    pub fn is_fresh(&self, now: OffsetDateTime) -> bool {
        self.expires_at().is_some_and(|expires| now < expires)
    }
}

/// The pause deadline a challenge starts: seven full days after it was seen
/// (FR-91). The value is stored on the Source row (`source.paused_until`).
pub fn pause_until_after_challenge(seen_at: OffsetDateTime) -> String {
    format_time(seen_at + time::Duration::days(CHALLENGE_PAUSE_DAYS))
}

/// Whether a stored `paused_until` still keeps Auto off. A row this build
/// cannot read fails closed: an unreadable deadline is not a permission.
pub fn pause_is_active(paused_until: Option<&str>, now: OffsetDateTime) -> bool {
    match paused_until {
        None => false,
        Some(value) => parse_time(value).is_none_or(|until| now < until),
    }
}

/// Where Auto stands for one Source (FR-91).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionAutoStatus {
    /// Never opted in, or revoked.
    Off,
    /// The opt-in lapsed: Auto falls back to Review until re-confirmed.
    Expired { expires_at: String },
    /// A challenge paused Auto.
    Paused { until: String },
    /// Auto may run, under the caps.
    Active,
}

impl SessionAutoStatus {
    /// Whether Auto may send on this Source right now.
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    /// The stable wire name the UI maps to its copy.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Expired { .. } => "expired",
            Self::Paused { .. } => "paused",
            Self::Active => "active",
        }
    }
}

/// The state Auto is in, in the order that matters: no opt-in, then a lapsed
/// one, then a challenge pause, then active (FR-91).
pub fn status(
    opt_in: Option<&SessionAutoOptIn>,
    paused_until: Option<&str>,
    now: OffsetDateTime,
) -> SessionAutoStatus {
    let Some(opt_in) = opt_in else {
        return SessionAutoStatus::Off;
    };
    if !opt_in.is_fresh(now) {
        return SessionAutoStatus::Expired {
            expires_at: opt_in.expires_at().map(format_time).unwrap_or_default(),
        };
    }
    if pause_is_active(paused_until, now) {
        return SessionAutoStatus::Paused {
            until: paused_until.unwrap_or_default().to_string(),
        };
    }
    SessionAutoStatus::Active
}

/// Read a stored RFC 3339 timestamp.
pub fn parse_time(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339).ok()
}

/// Format a timestamp the way the core stores it (UTC RFC 3339).
pub fn format_time(value: OffsetDateTime) -> String {
    value
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-10-17T09:00:00Z";

    fn now() -> OffsetDateTime {
        parse_time(NOW).expect("a valid fixture")
    }

    fn at(text: &str) -> OffsetDateTime {
        parse_time(text).expect("a valid fixture")
    }

    fn accepted(at_text: &str) -> SessionAutoOptIn {
        SessionAutoOptIn::accept(
            "linkedin",
            "LinkedIn",
            "LinkedIn",
            AUTO_WARNING_VERSION,
            at(at_text),
        )
        .expect("a valid acceptance")
    }

    #[test]
    fn the_default_cap_is_three_and_ten_is_the_hard_maximum() {
        assert_eq!(DEFAULT_SESSION_DAILY_CAP, 3);
        assert_eq!(MAX_SESSION_DAILY_CAP, 10);
        assert_eq!(MIN_SUBMISSION_GAP_MINUTES, 10);
        assert_eq!(MIN_HOLD_MINUTES, 60);
        assert_eq!(CHALLENGE_PAUSE_DAYS, 7);
        assert_eq!(RECONFIRM_DAYS, 30);

        assert_eq!(validate_daily_cap(1), Ok(1));
        assert_eq!(validate_daily_cap(3), Ok(3));
        assert_eq!(validate_daily_cap(10), Ok(10));
        assert_eq!(validate_daily_cap(0), Err(CapViolation::Zero));
        assert_eq!(validate_daily_cap(11), Err(CapViolation::TooHigh));
    }

    #[test]
    fn a_stored_cap_outside_the_range_falls_back_to_three() {
        assert_eq!(stored_daily_cap(None), DEFAULT_SESSION_DAILY_CAP);
        assert_eq!(stored_daily_cap(Some("7")), 7);
        assert_eq!(stored_daily_cap(Some("3")), 3);
        assert_eq!(
            stored_daily_cap(Some("25")),
            DEFAULT_SESSION_DAILY_CAP,
            "a hand-edited row never widens Auto"
        );
        assert_eq!(stored_daily_cap(Some("0")), DEFAULT_SESSION_DAILY_CAP);
        assert_eq!(stored_daily_cap(Some("nope")), DEFAULT_SESSION_DAILY_CAP);
    }

    #[test]
    fn accepting_auto_needs_the_typed_name_and_the_current_warning() {
        let accepted = SessionAutoOptIn::accept(
            "linkedin",
            "LinkedIn",
            "  LinkedIn ",
            AUTO_WARNING_VERSION,
            now(),
        )
        .expect("surrounding whitespace is not a different name");
        assert_eq!(accepted.source_id, "linkedin");
        assert_eq!(accepted.confirmed_name, "LinkedIn");
        assert_eq!(accepted.accepted_at, NOW);

        assert_eq!(
            SessionAutoOptIn::accept(
                "linkedin",
                "LinkedIn",
                "Linkedin",
                AUTO_WARNING_VERSION,
                now()
            ),
            Err(SessionAutoRefusal::NameMismatch),
            "the typed name must be the Source's own"
        );
        assert_eq!(
            SessionAutoOptIn::accept("linkedin", "LinkedIn", "", AUTO_WARNING_VERSION, now()),
            Err(SessionAutoRefusal::NameMismatch)
        );
        assert_eq!(
            SessionAutoOptIn::accept("linkedin", "LinkedIn", "LinkedIn", "v0", now()),
            Err(SessionAutoRefusal::WarningChanged {
                current: AUTO_WARNING_VERSION.to_string()
            }),
            "a warning this build does not ship is never accepted"
        );
    }

    #[test]
    fn an_opt_in_lapses_thirty_days_after_it_was_accepted() {
        let opt_in = accepted("2026-09-17T09:00:00Z");
        assert_eq!(
            opt_in.expires_at().map(format_time).as_deref(),
            Some("2026-10-17T09:00:00Z")
        );

        assert!(opt_in.is_fresh(at("2026-10-16T09:00:00Z")));
        assert!(opt_in.is_fresh(at("2026-10-16T23:59:59Z")));
        assert!(
            !opt_in.is_fresh(at("2026-10-17T09:00:00Z")),
            "thirty days later it must be re-confirmed"
        );
        assert!(!opt_in.is_fresh(at("2026-11-20T00:00:00Z")));
    }

    #[test]
    fn a_challenge_pauses_auto_for_seven_days() {
        let until = pause_until_after_challenge(now());
        assert_eq!(until, "2026-10-24T09:00:00Z");

        assert!(pause_is_active(Some(&until), at("2026-10-18T09:00:00Z")));
        assert!(pause_is_active(Some(&until), at("2026-10-24T08:59:59Z")));
        assert!(
            !pause_is_active(Some(&until), at("2026-10-24T09:00:00Z")),
            "the pause ends seven full days after the challenge"
        );
        assert!(!pause_is_active(None, now()));
        assert!(
            pause_is_active(Some("not a date"), now()),
            "an unreadable deadline fails closed"
        );
    }

    #[test]
    fn the_status_order_is_off_then_expired_then_paused_then_active() {
        assert_eq!(status(None, None, now()), SessionAutoStatus::Off);

        let lapsed = accepted("2026-09-01T09:00:00Z");
        assert_eq!(
            status(Some(&lapsed), None, now()),
            SessionAutoStatus::Expired {
                expires_at: "2026-10-01T09:00:00Z".to_string()
            }
        );

        let fresh = accepted("2026-10-10T09:00:00Z");
        let until = pause_until_after_challenge(now());
        assert_eq!(
            status(Some(&fresh), Some(&until), now()),
            SessionAutoStatus::Paused {
                until: "2026-10-24T09:00:00Z".to_string()
            }
        );

        let over = pause_until_after_challenge(at("2026-10-01T09:00:00Z"));
        assert_eq!(
            status(Some(&fresh), Some(&over), now()),
            SessionAutoStatus::Active,
            "an ended pause does not keep Auto off"
        );
        assert_eq!(status(Some(&fresh), None, now()), SessionAutoStatus::Active);

        assert!(!SessionAutoStatus::Off.is_active());
        assert!(
            !SessionAutoStatus::Expired {
                expires_at: String::new()
            }
            .is_active()
        );
        assert!(
            !SessionAutoStatus::Paused {
                until: String::new()
            }
            .is_active()
        );
        assert!(SessionAutoStatus::Active.is_active());
        assert_eq!(SessionAutoStatus::Off.as_str(), "off");
        assert_eq!(SessionAutoStatus::Active.as_str(), "active");
    }

    #[test]
    fn the_cap_key_is_per_source() {
        assert_eq!(cap_key("linkedin"), "session.auto.cap.linkedin");
        assert_eq!(cap_key("indeed"), "session.auto.cap.indeed");
    }
}
