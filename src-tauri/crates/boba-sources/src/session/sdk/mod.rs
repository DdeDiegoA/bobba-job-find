//! Session-board Source SDK (S11.1a, FR-30, AD-9, AD-10).
//!
//! The policy every Session Board Source shares, and the use case that gates
//! one on the user's per-Source opt-in:
//!
//! * a **versioned** warning (account-ban risk + the platform-specific cookie
//!   sentence) accepted per Source, recorded with its timestamp and version and
//!   audited (AD-9, AD-12);
//! * low-volume, human-like pacing ([`DEFAULT_PACING`]) and never a fixed CDP
//!   port (the browser profile surface owns the launch plan);
//! * no account creation and no CAPTCHA solving: a challenge classifies into a
//!   [`ChallengeKind`] and pauses the Source, which is notified and resumed by
//!   the user, never retried around (AD-10);
//! * "Sign out / clear session" deletes that Source's profile and cookies, and
//!   is audited; cookies live until that action (or Reset everything).

pub mod run;
pub mod service;

use serde::{Deserialize, Serialize};

pub use run::{LoadSpend, RunControl, RunError, SessionRun, check};
pub use service::{BoardView, PageLoad, SessionBoardError, SessionBoardService, SessionSnapshot};

/// The warning copy version the UI shows and the opt-in records (FR-30).
/// Bumping it forces every Session Board Source to be re-confirmed.
pub const WARNING_VERSION: &str = "v1";

/// Pacing between two automated page loads on one Session Board Source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pacing {
    /// The lowest delay between two page loads, in milliseconds.
    pub min_interval_ms: u64,
    /// The jitter applied around `min_interval_ms`, in percent of it.
    pub jitter_pct: u64,
}

impl Pacing {
    /// The delay before the `n`-th page load, drawn from `sample`.
    ///
    /// `sample` is any changing number (a counter, a timer-derived value); the
    /// result stays inside `min_interval_ms ± jitter_pct`, so the pacing is
    /// human-like without ever being faster than the floor.
    pub fn delay_ms(&self, sample: u64) -> u64 {
        let span = self.min_interval_ms.saturating_mul(self.jitter_pct) / 100;
        let low = self.min_interval_ms.saturating_sub(span);
        let spread = span.saturating_mul(2) + 1;
        low + sample % spread
    }
}

/// The conservative defaults every Session Board Source starts from: a mean of
/// one page every 30 s, ±40 % — so the *floor* is 18 s and the ceiling 42 s.
/// Low volume and human-like (FR-30), and never faster than the floor.
pub const DEFAULT_PACING: Pacing = Pacing {
    min_interval_ms: 30_000,
    jitter_pct: 40,
};

/// What a Session Board page asked for that Boba must never answer itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeKind {
    /// A CAPTCHA or human check.
    Captcha,
    /// A sign-in wall or an expired session.
    LoginWall,
    /// An "unusual activity" warning from the board.
    UnusualActivity,
    /// A rate-limit response.
    RateLimited,
}

impl ChallengeKind {
    /// Stable wire name (the pause record and the UI share it).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Captcha => "captcha",
            Self::LoginWall => "login_wall",
            Self::UnusualActivity => "unusual_activity",
            Self::RateLimited => "rate_limited",
        }
    }

    /// Parse the stored form, `None` for an unknown kind.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "captcha" => Some(Self::Captcha),
            "login_wall" => Some(Self::LoginWall),
            "unusual_activity" => Some(Self::UnusualActivity),
            "rate_limited" => Some(Self::RateLimited),
            _ => None,
        }
    }
}

/// Classify a page's visible text (AD-10). Anything that reads like a challenge
/// stops the Source: Boba never solves a CAPTCHA, creates an account or retries
/// around a wall.
///
/// The most severe signal wins, so a rate-limited page that also asks to sign
/// in pauses as `RateLimited`, and an "unusual activity" page that also shows a
/// human check pauses as `UnusualActivity`.
pub fn classify_challenge(text: &str) -> Option<ChallengeKind> {
    let text = text.to_lowercase();
    let has = |needle: &str| text.contains(needle);
    if ["too many requests", "rate limit", "rate-limit", "429"]
        .iter()
        .any(|needle| has(needle))
    {
        return Some(ChallengeKind::RateLimited);
    }
    if [
        "unusual activity",
        "temporarily restricted",
        "suspicious activity",
    ]
    .iter()
    .any(|needle| has(needle))
    {
        return Some(ChallengeKind::UnusualActivity);
    }
    if [
        "captcha",
        "verify you are human",
        "security check",
        "are you a robot",
    ]
    .iter()
    .any(|needle| has(needle))
    {
        return Some(ChallengeKind::Captcha);
    }
    if [
        "sign in",
        "log in",
        "join linkedin",
        "join now",
        "session expired",
        "authwall",
    ]
    .iter()
    .any(|needle| has(needle))
    {
        return Some(ChallengeKind::LoginWall);
    }
    None
}

/// A paused Session Board Source, as stored (FR-30, AD-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseRecord {
    /// What the board asked for.
    pub kind: ChallengeKind,
    /// When the challenge was seen (UTC RFC 3339).
    pub since: String,
}

impl PauseRecord {
    /// A pause that starts now.
    pub fn new(kind: ChallengeKind, since: impl Into<String>) -> Self {
        Self {
            kind,
            since: since.into(),
        }
    }

    /// The stored JSON form (a `setting` row's value).
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    /// Parse a stored value; `None` when it is malformed or of an unknown kind.
    pub fn parse(value: &str) -> Option<Self> {
        serde_json::from_str(value).ok()
    }
}

/// The `setting` key holding a Source's pause record.
pub fn pause_key(source_id: &str) -> String {
    format!("session.pause.{source_id}")
}

/// The `setting` key holding the weak-cookie acknowledgement (Linux without a
/// keyring, AC2); the value is the warning version acknowledged.
pub fn weak_ack_key(source_id: &str) -> String {
    format!("session.weak_ack.{source_id}")
}

/// The `setting` key holding a Session Board Source's page loads for the local
/// day `now` falls in (S11.1b: the daily cap is a local-day total).
pub fn loads_key(source_id: &str, now: &str) -> String {
    format!("session.loads.{source_id}.{}", local_day(now))
}

/// `YYYY-MM-DD` for `now` in the user's local day; UTC when `now` is unreadable.
fn local_day(now: &str) -> String {
    use time::format_description::well_known::Rfc3339;
    let offset = time::Duration::minutes(i64::from(local_offset_minutes()));
    match time::OffsetDateTime::parse(now, &Rfc3339) {
        Ok(now) => {
            let local = now + offset;
            format!(
                "{:04}-{:02}-{:02}",
                local.year(),
                u8::from(local.month()),
                local.day()
            )
        }
        Err(_) => "0000-00-00".to_string(),
    }
}

/// The user's UTC offset in minutes, from the OS.
fn local_offset_minutes() -> i32 {
    i32::from(
        time::UtcOffset::current_local_offset()
            .unwrap_or(time::UtcOffset::UTC)
            .whole_minutes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_low_volume_and_human_like() {
        const {
            assert!(
                DEFAULT_PACING.min_interval_ms >= 30_000,
                "one page per half a minute at most"
            )
        };
        const { assert!(DEFAULT_PACING.jitter_pct > 0, "delays are not a metronome") };
        let delays: Vec<u64> = (0..64).map(|n| DEFAULT_PACING.delay_ms(n)).collect();
        let floor = DEFAULT_PACING.min_interval_ms
            - DEFAULT_PACING.min_interval_ms * DEFAULT_PACING.jitter_pct / 100;
        let ceiling = DEFAULT_PACING.min_interval_ms
            + DEFAULT_PACING.min_interval_ms * DEFAULT_PACING.jitter_pct / 100;
        assert!(delays.iter().all(|d| (floor..=ceiling).contains(d)));
        assert!(delays.iter().any(|d| *d != delays[0]), "the delay varies");
    }

    #[test]
    fn a_human_check_is_a_captcha() {
        for text in [
            "Please verify you are human",
            "Let's do a quick security check",
            "Complete the CAPTCHA to continue",
        ] {
            assert_eq!(
                classify_challenge(text),
                Some(ChallengeKind::Captcha),
                "{text}"
            );
        }
    }

    #[test]
    fn a_sign_in_wall_is_a_login_wall() {
        for text in [
            "Sign in to continue",
            "Your session expired, please log in",
            "Join LinkedIn",
        ] {
            assert_eq!(
                classify_challenge(text),
                Some(ChallengeKind::LoginWall),
                "{text}"
            );
        }
    }

    #[test]
    fn an_unusual_activity_warning_is_its_own_kind() {
        for text in [
            "We noticed unusual activity on your account",
            "Your account has been temporarily restricted",
        ] {
            assert_eq!(
                classify_challenge(text),
                Some(ChallengeKind::UnusualActivity),
                "{text}"
            );
        }
    }

    #[test]
    fn a_rate_limit_response_is_its_own_kind() {
        for text in ["Too many requests", "You are rate limited", "Error 429"] {
            assert_eq!(
                classify_challenge(text),
                Some(ChallengeKind::RateLimited),
                "{text}"
            );
        }
    }

    #[test]
    fn the_most_severe_signal_wins() {
        assert_eq!(
            classify_challenge("Too many requests. Please sign in later."),
            Some(ChallengeKind::RateLimited)
        );
        assert_eq!(
            classify_challenge("Unusual activity. Verify you are human."),
            Some(ChallengeKind::UnusualActivity)
        );
    }

    #[test]
    fn an_ordinary_page_has_no_challenge() {
        assert_eq!(
            classify_challenge("Senior Rust Engineer — Remote — Apply now"),
            None
        );
        assert_eq!(classify_challenge(""), None);
    }

    #[test]
    fn a_pause_record_round_trips_through_its_stored_json() {
        let pause = PauseRecord::new(ChallengeKind::Captcha, "2026-10-17T06:00:00.000Z");
        let parsed = PauseRecord::parse(&pause.to_json()).unwrap();
        assert_eq!(parsed, pause);
        assert!(PauseRecord::parse("not json").is_none());
        assert!(PauseRecord::parse(r#"{"kind":"telepathy","since":"x"}"#).is_none());
    }

    #[test]
    fn every_challenge_kind_has_a_stable_wire_name() {
        for kind in [
            ChallengeKind::Captcha,
            ChallengeKind::LoginWall,
            ChallengeKind::UnusualActivity,
            ChallengeKind::RateLimited,
        ] {
            assert_eq!(ChallengeKind::parse(kind.as_str()), Some(kind));
        }
        assert!(ChallengeKind::parse("telepathy").is_none());
    }

    #[test]
    fn setting_keys_are_namespaced_per_source() {
        assert_eq!(pause_key("linkedin"), "session.pause.linkedin");
        assert_eq!(weak_ack_key("linkedin"), "session.weak_ack.linkedin");
    }

    #[test]
    fn the_page_budget_is_a_local_day_total() {
        let key = loads_key("linkedin", "2026-10-17T06:00:00.000Z");
        assert!(key.starts_with("session.loads.linkedin."));
        assert_eq!(
            key.rsplit('.').next().map(str::len),
            Some(10),
            "the key carries a YYYY-MM-DD local day"
        );
        assert_ne!(
            key,
            loads_key("linkedin", "2026-10-25T06:00:00.000Z"),
            "another day starts a fresh budget"
        );
        assert_ne!(key, loads_key("indeed", "2026-10-17T06:00:00.000Z"));
    }
}
