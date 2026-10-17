//! Auto Rules: the rule model, its safe defaults and the bounds core enforces
//! (FR-83, FR-87, AD-11).
//!
//! A rule is scoped to Sources × Apply Channels and carries the thresholds a
//! candidate has to clear, the FLAG policy, the caps and the active hours. Every
//! bound in `docs/product/PRD.md` FR-83 is a constant here, and [`AutoRule::validate`]
//! is the one place that rejects a value outside it: the UI is never the gate.

use serde::{Deserialize, Serialize};

use crate::scoring::gates::Confidence;
use crate::scoring::rank::Verdict;

/// A fresh ULID for an Auto Rule, prefixed by type (AD-17: `rule_…`).
pub fn new_rule_id() -> String {
    format!("rule_{}", ulid::Ulid::generate())
}

/// The channel an Auto Rule may target (FR-83, AC1).
///
/// `Manual` is deliberately absent: applying by hand is not something Auto can
/// do. `SessionBoard` is present but is never selectable by default (AC2): it
/// also needs that Source's own Auto opt-in (FR-91).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleChannel {
    /// Email Apply.
    Email,
    /// A hosted ATS form Boba fills.
    AtsForm,
    /// Sanctioned API (v1.1).
    SanctionedApi,
    /// The user's own logged-in session on a Session Board.
    SessionBoard,
}

impl RuleChannel {
    /// Every rule channel, in the order the UI lists them.
    pub const ALL: [RuleChannel; 4] = [
        RuleChannel::Email,
        RuleChannel::AtsForm,
        RuleChannel::SanctionedApi,
        RuleChannel::SessionBoard,
    ];

    /// The stable stored / wire form.
    pub fn as_str(self) -> &'static str {
        match self {
            RuleChannel::Email => "email",
            RuleChannel::AtsForm => "ats_form",
            RuleChannel::SanctionedApi => "sanctioned_api",
            RuleChannel::SessionBoard => "session_board",
        }
    }

    /// Parse the stored form, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.as_str() == value)
    }

    /// Whether Auto may target this channel without an extra per-Source opt-in
    /// (AC2: `ats_form`, `sanctioned_api` and `email`).
    pub fn auto_selectable_by_default(self) -> bool {
        !matches!(self, RuleChannel::SessionBoard)
    }
}

/// The daily window Auto may send in, as minutes of the local day (FR-83).
///
/// `start > end` is a window that wraps midnight (22:00–06:00). It is measured
/// in the user's local time; the caller passes the local minute of day.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveHours {
    pub start_minute: u16,
    pub end_minute: u16,
}

impl ActiveHours {
    pub const DEFAULT_START_MINUTE: u16 = 9 * 60;
    pub const DEFAULT_END_MINUTE: u16 = 18 * 60;

    /// The compiled default: 09:00–18:00 local.
    pub fn default_window() -> Self {
        Self {
            start_minute: Self::DEFAULT_START_MINUTE,
            end_minute: Self::DEFAULT_END_MINUTE,
        }
    }

    /// Whether a minute of the local day falls inside the window.
    pub fn contains(&self, minute: u16) -> bool {
        if self.start_minute <= self.end_minute {
            (self.start_minute..self.end_minute).contains(&minute)
        } else {
            minute >= self.start_minute || minute < self.end_minute
        }
    }

    /// Whether the two ends name a real window.
    pub fn is_valid(&self) -> bool {
        self.start_minute != self.end_minute
            && self.start_minute < 24 * 60
            && self.end_minute < 24 * 60
    }
}

/// The default caps FR-83 fixes.
pub const DEFAULT_DAILY_CAP: u8 = 5;
/// The hard maximum of a rule's daily cap (FR-83).
pub const MAX_DAILY_CAP: u8 = 25;
/// The default per-company cap: one application per company…
pub const DEFAULT_PER_COMPANY_CAP: u8 = 1;
/// …per this many days.
pub const DEFAULT_PER_COMPANY_DAYS: u16 = 30;
/// The widest window the per-company cap may look back.
pub const MAX_PER_COMPANY_DAYS: u16 = 365;
/// The most Sources one rule may name (a list, not a whole catalogue).
pub const MAX_SOURCES: usize = 50;

/// One Auto Rule, as Settings → Applying stores it (FR-83).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutoRule {
    /// `rule_<ULID>` (AD-17).
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// The Sources this rule covers. Empty means every Source — the rule is not
    /// scoped to a hand-picked list.
    pub source_ids: Vec<String>,
    /// The channels this rule targets. Never empty.
    pub channels: Vec<RuleChannel>,
    /// Minimum Rank (0–100).
    pub min_rank: f32,
    /// Minimum Fit axis (0–100).
    pub min_fit: f32,
    /// Minimum overall card confidence.
    pub min_confidence: Confidence,
    /// Whether a gate FLAG may be auto-sent. Defaults to `false` (AC1).
    pub allow_flags: bool,
    /// The minimum Verdict band a candidate must reach.
    pub min_verdict: Verdict,
    /// This rule's daily cap (1–[`MAX_DAILY_CAP`], default [`DEFAULT_DAILY_CAP`]).
    pub daily_cap: u8,
    /// Applications allowed per company inside `per_company_days`.
    pub per_company_cap: u8,
    /// The look-back of `per_company_cap` (default 30 days).
    pub per_company_days: u16,
    /// When Auto may send for this rule.
    pub active_hours: ActiveHours,
    /// UTC RFC 3339 (AD-17).
    pub created_at: String,
    /// UTC RFC 3339 (AD-17).
    pub updated_at: String,
}

impl AutoRule {
    /// The conservative defaults a new rule starts from (EXPERIENCE §9.12 shows
    /// the same shape: Rank≥80 · Fit≥80 · no FLAGs · 5/day · 1/company/30d).
    pub fn defaults(
        id: impl Into<String>,
        name: impl Into<String>,
        now: impl Into<String>,
    ) -> Self {
        let now = now.into();
        Self {
            id: id.into(),
            name: name.into(),
            enabled: true,
            source_ids: Vec::new(),
            channels: vec![RuleChannel::Email],
            min_rank: 80.0,
            min_fit: 80.0,
            min_confidence: Confidence::Medium,
            allow_flags: false,
            min_verdict: Verdict::Good,
            daily_cap: DEFAULT_DAILY_CAP,
            per_company_cap: DEFAULT_PER_COMPANY_CAP,
            per_company_days: DEFAULT_PER_COMPANY_DAYS,
            active_hours: ActiveHours::default_window(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// Reject a value the PRD bounds (AD-11: core validates, never the UI).
    pub fn validate(&self) -> Result<(), RuleViolation> {
        if self.name.trim().is_empty() {
            return Err(RuleViolation::NameEmpty);
        }
        if self.name.chars().count() > 80 {
            return Err(RuleViolation::NameTooLong);
        }
        if self.channels.is_empty() {
            return Err(RuleViolation::NoChannels);
        }
        if self.channels.contains(&RuleChannel::SessionBoard) && self.source_ids.is_empty() {
            return Err(RuleViolation::SessionBoardNeedsSource);
        }
        if self.source_ids.len() > MAX_SOURCES {
            return Err(RuleViolation::TooManySources);
        }
        if self.source_ids.iter().any(|id| id.trim().is_empty()) {
            return Err(RuleViolation::EmptySourceScope);
        }
        if !(0.0..=100.0).contains(&self.min_rank) {
            return Err(RuleViolation::MinRankOutOfRange);
        }
        if !(0.0..=100.0).contains(&self.min_fit) {
            return Err(RuleViolation::MinFitOutOfRange);
        }
        if !(1..=MAX_DAILY_CAP).contains(&self.daily_cap) {
            return Err(RuleViolation::DailyCapOutOfRange);
        }
        if self.per_company_cap == 0 {
            return Err(RuleViolation::PerCompanyCapZero);
        }
        if !(1..=MAX_PER_COMPANY_DAYS).contains(&self.per_company_days) {
            return Err(RuleViolation::PerCompanyDaysOutOfRange);
        }
        if !self.active_hours.is_valid() {
            return Err(RuleViolation::ActiveHoursInvalid);
        }
        Ok(())
    }

    /// Whether this rule is scoped to `source_id`: an explicit listing, or every
    /// Source when the list is empty.
    pub fn covers_source(&self, source_id: &str) -> bool {
        self.source_ids.is_empty() || self.source_ids.iter().any(|id| id == source_id)
    }

    /// Whether this rule targets `channel`.
    pub fn covers_channel(&self, channel: RuleChannel) -> bool {
        self.channels.contains(&channel)
    }
}

/// Why a rule is refused (AC1's ranges, AD-11). The app maps each to an i18n
/// key; the UI shows the reason verbatim.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuleViolation {
    #[error("a rule needs a name")]
    NameEmpty,
    #[error("the name is longer than 80 characters")]
    NameTooLong,
    #[error("a rule needs at least one apply channel")]
    NoChannels,
    #[error("the session board needs an explicit source scope")]
    SessionBoardNeedsSource,
    #[error("the source list cannot be empty when a session board channel is used")]
    EmptySourceScope,
    #[error("the source scope has more than 50 sources")]
    TooManySources,
    #[error("the minimum rank must be between 0 and 100")]
    MinRankOutOfRange,
    #[error("the minimum fit must be between 0 and 100")]
    MinFitOutOfRange,
    #[error("the daily cap must be between 1 and {MAX_DAILY_CAP}")]
    DailyCapOutOfRange,
    #[error("the per-company cap must be at least 1")]
    PerCompanyCapZero,
    #[error("the per-company window must be between 1 and {MAX_PER_COMPANY_DAYS} days")]
    PerCompanyDaysOutOfRange,
    #[error("the active hours are not a valid window")]
    ActiveHoursInvalid,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule() -> AutoRule {
        AutoRule::defaults("rule_1", "Email Apply × remote", "2026-10-17T00:00:00Z")
    }

    #[test]
    fn a_fresh_rule_carries_the_safe_defaults() {
        let rule = rule();
        assert_eq!(rule.channels, vec![RuleChannel::Email]);
        assert_eq!(rule.min_rank, 80.0);
        assert_eq!(rule.min_fit, 80.0);
        assert_eq!(rule.min_confidence, Confidence::Medium);
        assert_eq!(rule.min_verdict, Verdict::Good);
        assert!(!rule.allow_flags, "FR-83: FLAGs are off by default");
        assert_eq!(rule.daily_cap, DEFAULT_DAILY_CAP);
        assert_eq!(rule.per_company_cap, DEFAULT_PER_COMPANY_CAP);
        assert_eq!(rule.per_company_days, DEFAULT_PER_COMPANY_DAYS);
        assert_eq!(rule.active_hours, ActiveHours::default_window());
        assert!(
            rule.source_ids.is_empty(),
            "an empty scope means every source"
        );
        assert!(
            rule.enabled,
            "a saved rule is on unless the user turns it off"
        );
        assert!(rule.validate().is_ok());
    }

    #[test]
    fn channel_names_round_trip_and_manual_is_not_one() {
        for channel in RuleChannel::ALL {
            assert_eq!(RuleChannel::parse(channel.as_str()), Some(channel));
        }
        assert_eq!(RuleChannel::parse("manual"), None);
        assert!(RuleChannel::Email.auto_selectable_by_default());
        assert!(RuleChannel::AtsForm.auto_selectable_by_default());
        assert!(RuleChannel::SanctionedApi.auto_selectable_by_default());
        assert!(!RuleChannel::SessionBoard.auto_selectable_by_default());
    }

    #[test]
    fn the_caps_stop_at_the_prd_bounds() {
        let mut capped = rule();
        capped.daily_cap = 1;
        assert!(capped.validate().is_ok(), "the minimum daily cap");
        capped.daily_cap = MAX_DAILY_CAP;
        assert!(capped.validate().is_ok(), "the hard maximum");
        capped.daily_cap = MAX_DAILY_CAP + 1;
        assert_eq!(capped.validate(), Err(RuleViolation::DailyCapOutOfRange));
        capped.daily_cap = 0;
        assert_eq!(capped.validate(), Err(RuleViolation::DailyCapOutOfRange));

        let mut company = rule();
        company.per_company_cap = 0;
        assert_eq!(company.validate(), Err(RuleViolation::PerCompanyCapZero));
        company.per_company_cap = 1;
        company.per_company_days = 0;
        assert_eq!(
            company.validate(),
            Err(RuleViolation::PerCompanyDaysOutOfRange)
        );
        company.per_company_days = MAX_PER_COMPANY_DAYS + 1;
        assert_eq!(
            company.validate(),
            Err(RuleViolation::PerCompanyDaysOutOfRange)
        );
    }

    #[test]
    fn thresholds_stay_between_zero_and_one_hundred() {
        let mut rule = rule();
        rule.min_rank = 0.0;
        rule.min_fit = 100.0;
        assert!(rule.validate().is_ok());
        rule.min_rank = -1.0;
        assert_eq!(rule.validate(), Err(RuleViolation::MinRankOutOfRange));
        rule.min_rank = 100.5;
        assert_eq!(rule.validate(), Err(RuleViolation::MinRankOutOfRange));
        rule.min_rank = 50.0;
        rule.min_fit = f32::NAN;
        assert_eq!(rule.validate(), Err(RuleViolation::MinFitOutOfRange));
    }

    #[test]
    fn a_rule_needs_a_name_and_a_channel() {
        let mut rule = rule();
        rule.name = "  ".into();
        assert_eq!(rule.validate(), Err(RuleViolation::NameEmpty));
        rule.name = "x".repeat(81);
        assert_eq!(rule.validate(), Err(RuleViolation::NameTooLong));
        rule.name = "ok".into();
        rule.channels.clear();
        assert_eq!(rule.validate(), Err(RuleViolation::NoChannels));
    }

    #[test]
    fn the_session_board_needs_an_explicit_source_scope() {
        let mut rule = rule();
        rule.channels = vec![RuleChannel::SessionBoard];
        assert_eq!(rule.validate(), Err(RuleViolation::SessionBoardNeedsSource));
        rule.source_ids = vec!["linkedin".into()];
        assert!(
            rule.validate().is_ok(),
            "a named source is the opt-in surface"
        );
    }

    #[test]
    fn the_source_scope_is_bounded() {
        let mut rule = rule();
        rule.source_ids = (0..51).map(|i| format!("src_{i}")).collect();
        assert_eq!(rule.validate(), Err(RuleViolation::TooManySources));
    }

    #[test]
    fn active_hours_include_the_wrap_and_reject_an_empty_window() {
        let day = ActiveHours {
            start_minute: 9 * 60,
            end_minute: 18 * 60,
        };
        assert!(day.contains(9 * 60));
        assert!(day.contains(17 * 60 + 59));
        assert!(!day.contains(18 * 60), "the end is exclusive");
        assert!(!day.contains(8 * 60 + 59));

        let night = ActiveHours {
            start_minute: 22 * 60,
            end_minute: 6 * 60,
        };
        assert!(night.contains(23 * 60));
        assert!(night.contains(2 * 60));
        assert!(!night.contains(12 * 60));

        assert!(day.is_valid());
        assert!(
            !ActiveHours {
                start_minute: 10,
                end_minute: 10
            }
            .is_valid(),
            "an empty window would never send, and would be sold as '24/7'"
        );
        assert!(
            !ActiveHours {
                start_minute: 0,
                end_minute: 1440
            }
            .is_valid()
        );
    }

    #[test]
    fn an_explicit_source_scope_is_what_the_rule_covers() {
        let mut rule = rule();
        assert!(rule.covers_source("any"));
        rule.source_ids = vec!["linkedin".into()];
        assert!(rule.covers_source("linkedin"));
        assert!(!rule.covers_source("indeed"));
        assert!(rule.covers_channel(RuleChannel::Email));
        assert!(!rule.covers_channel(RuleChannel::AtsForm));
    }
}
