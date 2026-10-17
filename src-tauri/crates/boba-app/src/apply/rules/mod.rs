//! Auto Rules use cases (S9.6, FR-83/FR-87/FR-108, AD-11).
//!
//! This is the only writer of the rule rows and of the five `apply.auto.*`
//! settings. Two rules shape it:
//!
//! * **Core validates, never the UI** (AD-11): [`AutoRulesService::save_rule`]
//!   parses the submitted channels/confidence/verdict and runs
//!   [`AutoRule::validate`], so a cap of 99 or an invented channel never reaches
//!   the store. The database checks the bounds again (migration 0022).
//! * **Enforcement is core-only** (AC2): [`AutoRulesService::decide`] assembles
//!   the caps from the ledger and calls the pure [`boba_domain::apply::policy::decide`].
//!   The callers that can start a send — attempt creation, Outbox release and
//!   just before submit — call this, and act on `Auto`, `Review` or `Block`.
//!   A screen is never the gate.
//!
//! The global settings live in `setting` rows (Consistency Conventions: config);
//! a malformed stored value falls back to the compiled default so the app never
//! loses a defined value (AD-1).

use std::collections::BTreeMap;
use std::sync::Arc;

use boba_domain::apply::policy::{
    self, ActiveHours, AutoAttemptEntry, AutoRule, AutoSettings, Decision, EnableRefused,
    PolicyCandidate, PolicyCounts, Reason, RuleChannel, RuleViolation, SettingsViolation,
};
use boba_domain::scoring::gates::Confidence;
use boba_domain::scoring::rank::Verdict;
use serde::{Deserialize, Serialize};
use specta::Type;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::ports::clock::ClockPort;
use crate::ports::repo::auto_rules::{AutoRulesRepo, AutoRulesRepoError};
use crate::ports::repo::system::{SystemRepo, SystemRepoError};

/// `setting` keys this area owns.
pub const AUTO_ENABLED_KEY: &str = "apply.auto.enabled";
/// When the consequences dialog was accepted (AC3).
pub const AUTO_CONFIRMED_AT_KEY: &str = "apply.auto.confirmed_at";
/// The cap across every rule (FR-83).
pub const GLOBAL_DAILY_CAP_KEY: &str = "apply.auto.global_daily_cap";

/// Everything Settings → Applying shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RulesSnapshot {
    pub auto_enabled: bool,
    /// Whether the consequences dialog was ever accepted (AC3).
    pub auto_confirmed: bool,
    pub global_daily_cap: u8,
    pub rules: Vec<RuleView>,
}

/// One Auto Rule as the screen shows it (AC1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RuleView {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub source_ids: Vec<String>,
    /// Stored channel ids: `email`, `ats_form`, `sanctioned_api`, `session_board`.
    pub channels: Vec<String>,
    pub min_rank: f32,
    pub min_fit: f32,
    /// `low`, `medium` or `high`.
    pub min_confidence: String,
    pub allow_flags: bool,
    /// `strong`, `good`, `moderate`, `weak` or `poor`.
    pub min_verdict: String,
    pub daily_cap: u8,
    pub per_company_cap: u8,
    pub per_company_days: u16,
    pub active_start_minute: u16,
    pub active_end_minute: u16,
    pub created_at: String,
    pub updated_at: String,
}

impl From<&AutoRule> for RuleView {
    fn from(rule: &AutoRule) -> Self {
        Self {
            id: rule.id.clone(),
            name: rule.name.clone(),
            enabled: rule.enabled,
            source_ids: rule.source_ids.clone(),
            channels: rule
                .channels
                .iter()
                .map(|c| c.as_str().to_string())
                .collect(),
            min_rank: rule.min_rank,
            min_fit: rule.min_fit,
            min_confidence: confidence_str(rule.min_confidence).to_string(),
            allow_flags: rule.allow_flags,
            min_verdict: rule.min_verdict.as_str().to_string(),
            daily_cap: rule.daily_cap,
            per_company_cap: rule.per_company_cap,
            per_company_days: rule.per_company_days,
            active_start_minute: rule.active_hours.start_minute,
            active_end_minute: rule.active_hours.end_minute,
            created_at: rule.created_at.clone(),
            updated_at: rule.updated_at.clone(),
        }
    }
}

/// What the screen submits to create or edit a rule (AC1). `id` is `None` for a
/// new rule; an unknown id is refused rather than silently created.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct RuleInput {
    pub id: Option<String>,
    pub name: String,
    pub enabled: bool,
    pub source_ids: Vec<String>,
    pub channels: Vec<String>,
    pub min_rank: f32,
    pub min_fit: f32,
    pub min_confidence: String,
    pub allow_flags: bool,
    pub min_verdict: String,
    pub daily_cap: u8,
    pub per_company_cap: u8,
    pub per_company_days: u16,
    pub active_start_minute: u16,
    pub active_end_minute: u16,
}

/// One decision, in the form the UI renders verbatim (AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DecisionView {
    /// `auto`, `review` or `block`.
    pub kind: String,
    /// The rule that would send, when `kind` is `auto`.
    pub rule_id: Option<String>,
    pub reasons: Vec<ReasonView>,
}

/// One reason the UI shows word for word (AC2): the i18n leaf is
/// `applying:reason.<code>` and `params` interpolate into it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ReasonView {
    pub code: String,
    pub params: BTreeMap<String, String>,
}

/// Why an Auto Rules use case failed.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RulesError {
    #[error(transparent)]
    Repo(#[from] AutoRulesRepoError),
    #[error(transparent)]
    System(#[from] SystemRepoError),
    /// A rule value the PRD bounds refuses (AD-11).
    #[error("invalid auto rule: {0}")]
    InvalidRule(RuleViolation),
    /// A global setting the PRD bounds refuses.
    #[error("invalid auto settings: {0}")]
    InvalidSettings(SettingsViolation),
    /// Turning Auto on without the dialog or without a rule (AC3).
    #[error("auto cannot be enabled: {0}")]
    CannotEnable(EnableRefused),
    /// A channel id this build does not know.
    #[error("unknown apply channel '{0}'")]
    UnknownChannel(String),
    /// A confidence id this build does not know.
    #[error("unknown confidence '{0}'")]
    UnknownConfidence(String),
    /// A verdict id this build does not know.
    #[error("unknown verdict '{0}'")]
    UnknownVerdict(String),
    /// An edit named a rule that is not stored.
    #[error("unknown auto rule '{0}'")]
    UnknownRule(String),
    /// The stored ledger timestamp cannot be read; the spacing is unknown.
    #[error("the auto ledger timestamp cannot be read")]
    Clock,
}

impl RulesError {
    /// The i18n key the command layer reports (CH7).
    pub fn message_key(&self) -> &'static str {
        match self {
            RulesError::Repo(_) | RulesError::System(_) => "errors.apply.store",
            RulesError::InvalidRule(_) => "errors.apply.rule_invalid",
            RulesError::InvalidSettings(_) => "errors.apply.settings_invalid",
            RulesError::CannotEnable(refused) => match refused {
                EnableRefused::NotConfirmed => "errors.apply.rule_unconfirmed",
                EnableRefused::NoRules => "errors.apply.rule_missing",
            },
            RulesError::UnknownChannel(_)
            | RulesError::UnknownConfidence(_)
            | RulesError::UnknownVerdict(_) => "errors.apply.rule_invalid",
            RulesError::UnknownRule(_) => "errors.apply.unknown_rule",
            RulesError::Clock => "errors.apply.rule_clock",
        }
    }
}

/// The Auto Rules use cases. The ports arrive as `Arc<dyn …>`, the shape every
/// use case in this crate uses, so one instance can sit behind a command.
pub struct AutoRulesService {
    rules: Arc<dyn AutoRulesRepo>,
    system: Arc<dyn SystemRepo>,
    clock: Arc<dyn ClockPort>,
}

impl AutoRulesService {
    pub fn new(
        rules: Arc<dyn AutoRulesRepo>,
        system: Arc<dyn SystemRepo>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            rules,
            system,
            clock,
        }
    }

    /// The rules and the global settings, defaults resolved.
    pub fn snapshot(&self) -> Result<RulesSnapshot, RulesError> {
        let settings = self.load_settings()?;
        Ok(snapshot(&settings, &self.rules.rules()?))
    }

    /// Create or edit one rule (AC1). Core validates the whole rule before it is
    /// stored; `created_at` survives an edit and `updated_at` is stamped now.
    pub fn save_rule(&self, input: RuleInput) -> Result<RuleView, RulesError> {
        let now = self.clock.now_rfc3339();
        let rule = self.rule_from(input, &now)?;
        rule.validate().map_err(RulesError::InvalidRule)?;
        self.rules.rule_upsert(&rule)?;
        Ok(RuleView::from(&rule))
    }

    /// Remove one rule. Returns whether a row was deleted.
    pub fn delete_rule(&self, id: &str) -> Result<bool, RulesError> {
        Ok(self.rules.rule_delete(id)?)
    }

    /// Turn Auto on (after the consequences dialog, AC3) or off. Turning it off
    /// keeps the confirmation, so turning it back on does not ask again; a reset
    /// is what clears it.
    pub fn set_enabled(&self, enabled: bool, confirmed: bool) -> Result<RulesSnapshot, RulesError> {
        let current = self.load_settings()?;
        let now = self.clock.now_rfc3339();
        let next = if enabled {
            let rules = self.rules.rules()?;
            current
                .enable(&rules, confirmed, &now)
                .map_err(RulesError::CannotEnable)?
        } else {
            AutoSettings {
                auto_enabled: false,
                ..current
            }
        };
        self.persist_settings(&next, &now)?;
        Ok(snapshot(&self.load_settings()?, &self.rules.rules()?))
    }

    /// Set the cap across every rule (FR-83). Validated here, never by the UI.
    pub fn set_global_cap(&self, cap: u8) -> Result<RulesSnapshot, RulesError> {
        let mut settings = self.load_settings()?;
        settings.global_daily_cap = cap;
        settings.validate().map_err(RulesError::InvalidSettings)?;
        let now = self.clock.now_rfc3339();
        self.persist_settings(&settings, &now)?;
        Ok(snapshot(&self.load_settings()?, &self.rules.rules()?))
    }

    /// FR-108: turn Auto off and restore the default caps (AC4).
    pub fn reset_to_defaults(&self) -> Result<RulesSnapshot, RulesError> {
        let now = self.clock.now_rfc3339();
        let mut rules = self.rules.rules()?;
        let reset = policy::reset_to_defaults(&mut rules);
        for rule in &rules {
            self.rules.rule_upsert(rule)?;
        }
        self.persist_settings(&reset, &now)?;
        Ok(snapshot(&reset, &self.rules.rules()?))
    }

    /// Run the policy for one candidate (AC2). The caps and the spacing are read
    /// from the ledger here, so every caller gets the same answer.
    ///
    /// `spacing_sample` is the caller's random sample: the 5–20 minute gap is
    /// derived from it, which keeps this use case deterministic under test.
    pub fn decide(
        &self,
        candidate: &PolicyCandidate,
        spacing_sample: u64,
    ) -> Result<DecisionView, RulesError> {
        let settings = self.load_settings()?;
        let rules = self.rules.rules()?;
        let now = self.now()?;
        let counts = self.counts(
            &rules,
            &candidate.company_key,
            &now,
            candidate.utc_offset_minutes,
            spacing_sample,
        )?;
        Ok(decision_view(&policy::decide(
            &settings, &rules, candidate, &counts,
        )))
    }

    /// Record one automatic submission before its channel starts (FR-83). The
    /// row is what the three caps count on the next call.
    pub fn record_auto_send(
        &self,
        rule_id: &str,
        application_id: &str,
        source_id: &str,
        channel: RuleChannel,
        company_key: &str,
    ) -> Result<AutoAttemptEntry, RulesError> {
        let entry = AutoAttemptEntry {
            id: policy::new_auto_attempt_id(),
            rule_id: rule_id.to_string(),
            application_id: application_id.to_string(),
            source_id: source_id.to_string(),
            channel,
            company_key: company_key.to_string(),
            created_at: self.clock.now_rfc3339(),
        };
        self.rules.ledger_insert(&entry)?;
        Ok(entry)
    }

    // ---- internals ---------------------------------------------------------

    fn rule_from(&self, input: RuleInput, now: &str) -> Result<AutoRule, RulesError> {
        let channels = input
            .channels
            .iter()
            .map(|channel| {
                RuleChannel::parse(channel)
                    .ok_or_else(|| RulesError::UnknownChannel(channel.clone()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let min_confidence = parse_confidence(&input.min_confidence)
            .ok_or_else(|| RulesError::UnknownConfidence(input.min_confidence.clone()))?;
        let min_verdict = Verdict::parse(&input.min_verdict)
            .ok_or_else(|| RulesError::UnknownVerdict(input.min_verdict.clone()))?;

        let created_at = match &input.id {
            None => now.to_string(),
            Some(id) => {
                let existing = self
                    .rules
                    .rules()?
                    .into_iter()
                    .find(|rule| &rule.id == id)
                    .ok_or_else(|| RulesError::UnknownRule(id.clone()))?;
                existing.created_at
            }
        };

        Ok(AutoRule {
            id: input
                .id
                .unwrap_or_else(boba_domain::apply::policy::rule::new_rule_id),
            name: input.name,
            enabled: input.enabled,
            source_ids: input.source_ids,
            channels,
            min_rank: input.min_rank,
            min_fit: input.min_fit,
            min_confidence,
            allow_flags: input.allow_flags,
            min_verdict,
            daily_cap: input.daily_cap,
            per_company_cap: input.per_company_cap,
            per_company_days: input.per_company_days,
            active_hours: ActiveHours {
                start_minute: input.active_start_minute,
                end_minute: input.active_end_minute,
            },
            created_at,
            updated_at: now.to_string(),
        })
    }

    fn load_settings(&self) -> Result<AutoSettings, RulesError> {
        let defaults = AutoSettings::default();
        let enabled = self
            .system
            .setting_get(AUTO_ENABLED_KEY)?
            .map(|v| v == "true")
            .unwrap_or(defaults.auto_enabled);
        let confirmed_at = self
            .system
            .setting_get(AUTO_CONFIRMED_AT_KEY)?
            .filter(|value| !value.trim().is_empty());
        let global_daily_cap = self
            .system
            .setting_get(GLOBAL_DAILY_CAP_KEY)?
            .and_then(|v| v.parse::<u8>().ok())
            .unwrap_or(defaults.global_daily_cap);

        let settings = AutoSettings {
            // An enabled switch without a confirmation is not enabled: the
            // dialog is the one thing the user cannot skip (AC3).
            auto_enabled: enabled && confirmed_at.is_some(),
            confirmed_at,
            global_daily_cap,
        };
        if settings.validate().is_err() {
            // A stored row this build cannot honour never leaves Auto running
            // on a value the PRD forbids: it falls back to the defaults.
            return Ok(defaults);
        }
        Ok(settings)
    }

    fn persist_settings(&self, settings: &AutoSettings, now: &str) -> Result<(), RulesError> {
        settings.validate().map_err(RulesError::InvalidSettings)?;
        self.system
            .setting_put(AUTO_ENABLED_KEY, truth(settings.auto_enabled), now)?;
        self.system.setting_put(
            AUTO_CONFIRMED_AT_KEY,
            settings.confirmed_at.as_deref().unwrap_or_default(),
            now,
        )?;
        self.system.setting_put(
            GLOBAL_DAILY_CAP_KEY,
            &settings.global_daily_cap.to_string(),
            now,
        )?;
        Ok(())
    }

    fn now(&self) -> Result<OffsetDateTime, RulesError> {
        OffsetDateTime::parse(&self.clock.now_rfc3339(), &Rfc3339).map_err(|_| RulesError::Clock)
    }

    /// The caps and the spacing for one decision (AC2).
    fn counts(
        &self,
        rules: &[AutoRule],
        company_key: &str,
        now: &OffsetDateTime,
        utc_offset_minutes: i32,
        spacing_sample: u64,
    ) -> Result<PolicyCounts, RulesError> {
        // The daily caps reset at the user's local midnight, the same clock the
        // active hours use (S9.6 review F1): shift into local time, take the
        // day's start there, and shift back for the UTC ledger.
        let offset = time::Duration::minutes(i64::from(utc_offset_minutes));
        let day_start = (*now + offset).replace_time(time::Time::MIDNIGHT) - offset;
        let global_today = self.rules.ledger_global_since(&format_time(day_start))?;
        let mut per_rule_today = Vec::with_capacity(rules.len());
        let mut per_company = Vec::with_capacity(rules.len());
        for rule in rules {
            per_rule_today.push((
                rule.id.clone(),
                self.rules
                    .ledger_rule_since(&rule.id, &format_time(day_start))?,
            ));
            let window = *now - time::Duration::days(i64::from(rule.per_company_days));
            per_company.push((
                rule.id.clone(),
                self.rules
                    .ledger_company_since(company_key, &format_time(window))?,
            ));
        }
        let minutes_since_last_auto = match self.rules.ledger_last_at()? {
            None => None,
            Some(text) => {
                let last = OffsetDateTime::parse(&text, &Rfc3339).map_err(|_| RulesError::Clock)?;
                Some((*now - last).whole_minutes().max(0) as u32)
            }
        };
        Ok(PolicyCounts {
            global_today,
            per_rule_today,
            per_company,
            minutes_since_last_auto,
            spacing_sample,
        })
    }
}

fn snapshot(settings: &AutoSettings, rules: &[AutoRule]) -> RulesSnapshot {
    RulesSnapshot {
        auto_enabled: settings.auto_enabled,
        auto_confirmed: settings.confirmed_at.is_some(),
        global_daily_cap: settings.global_daily_cap,
        rules: rules.iter().map(RuleView::from).collect(),
    }
}

fn decision_view(decision: &Decision) -> DecisionView {
    match decision {
        Decision::Auto { rule_id } => DecisionView {
            kind: "auto".to_string(),
            rule_id: Some(rule_id.clone()),
            reasons: Vec::new(),
        },
        Decision::Review { reasons } | Decision::Block { reasons } => DecisionView {
            kind: if matches!(decision, Decision::Review { .. }) {
                "review"
            } else {
                "block"
            }
            .to_string(),
            rule_id: None,
            reasons: reasons.iter().map(ReasonView::from).collect(),
        },
    }
}

impl From<&Reason> for ReasonView {
    fn from(reason: &Reason) -> Self {
        Self {
            code: reason.code().to_string(),
            params: reason
                .params()
                .into_iter()
                .map(|(key, value)| (key.to_string(), value))
                .collect(),
        }
    }
}

/// The user's offset from UTC in minutes, east positive (FR-83).
///
/// The offset comes from the OS, not from a timezone database, so DST is honest
/// without a dependency; a process the platform will not report it to falls back
/// to UTC rather than guessing a window. The UI knows the offset first-hand and
/// sends it, so this is only the fallback.
pub fn local_offset_minutes() -> i32 {
    i32::from(
        time::UtcOffset::current_local_offset()
            .unwrap_or(time::UtcOffset::UTC)
            .whole_minutes(),
    )
}

/// The minute of the user's local day at `now`, at `offset_minutes` from UTC
/// (FR-83's active hours).
pub fn local_minute_at(now: &str, offset_minutes: i32) -> u16 {
    let Ok(now) = OffsetDateTime::parse(now, &Rfc3339) else {
        return 0;
    };
    let local = now + time::Duration::minutes(i64::from(offset_minutes));
    local.hour() as u16 * 60 + local.minute() as u16
}

/// The minute of the user's local day at `now`, with the OS offset (FR-83).
pub fn local_minute(now: &str) -> u16 {
    local_minute_at(now, local_offset_minutes())
}

fn truth(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn confidence_str(value: Confidence) -> &'static str {
    match value {
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}

fn parse_confidence(value: &str) -> Option<Confidence> {
    match value {
        "low" => Some(Confidence::Low),
        "medium" => Some(Confidence::Medium),
        "high" => Some(Confidence::High),
        _ => None,
    }
}

fn format_time(value: OffsetDateTime) -> String {
    value
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::repo::auto_rules::AutoRulesRepoError;
    use crate::system::{FixedClock, testing::FakeSystemRepo};
    use std::sync::Mutex;

    const NOW: &str = "2026-10-17T09:00:00Z";

    #[derive(Default)]
    struct FakeAutoRules {
        rules: Mutex<Vec<AutoRule>>,
        ledger: Mutex<Vec<AutoAttemptEntry>>,
    }

    impl FakeAutoRules {
        fn with_rule(rule: AutoRule) -> Self {
            Self {
                rules: Mutex::new(vec![rule]),
                ledger: Mutex::new(Vec::new()),
            }
        }

        /// A rule plus the ledger rows already recorded (their `created_at`
        /// decides which local day they fall in).
        fn with_ledger(rule: AutoRule, created_at: &[&str]) -> Self {
            Self {
                rules: Mutex::new(vec![rule]),
                ledger: Mutex::new(
                    created_at
                        .iter()
                        .enumerate()
                        .map(|(index, at)| AutoAttemptEntry {
                            id: format!("aut_{index}"),
                            rule_id: rule_for_ledger(),
                            application_id: format!("app_{index}"),
                            source_id: "remotive".into(),
                            channel: RuleChannel::Email,
                            company_key: "other".into(),
                            created_at: (*at).into(),
                        })
                        .collect(),
                ),
            }
        }
    }

    fn rule_for_ledger() -> String {
        "rule_1".to_string()
    }

    impl AutoRulesRepo for FakeAutoRules {
        fn rules(&self) -> Result<Vec<AutoRule>, AutoRulesRepoError> {
            Ok(self.rules.lock().unwrap().clone())
        }

        fn rule_upsert(&self, rule: &AutoRule) -> Result<(), AutoRulesRepoError> {
            let mut rules = self.rules.lock().unwrap();
            match rules.iter_mut().find(|stored| stored.id == rule.id) {
                Some(stored) => *stored = rule.clone(),
                None => rules.push(rule.clone()),
            }
            Ok(())
        }

        fn rule_delete(&self, id: &str) -> Result<bool, AutoRulesRepoError> {
            let mut rules = self.rules.lock().unwrap();
            let before = rules.len();
            rules.retain(|rule| rule.id != id);
            Ok(rules.len() != before)
        }

        fn ledger_insert(&self, entry: &AutoAttemptEntry) -> Result<(), AutoRulesRepoError> {
            self.ledger.lock().unwrap().push(entry.clone());
            Ok(())
        }

        fn ledger_global_since(&self, since: &str) -> Result<u32, AutoRulesRepoError> {
            Ok(self
                .ledger
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| entry.created_at.as_str() >= since)
                .count() as u32)
        }

        fn ledger_rule_since(&self, rule_id: &str, since: &str) -> Result<u32, AutoRulesRepoError> {
            Ok(self
                .ledger
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| entry.rule_id == rule_id && entry.created_at.as_str() >= since)
                .count() as u32)
        }

        fn ledger_company_since(
            &self,
            company_key: &str,
            since: &str,
        ) -> Result<u32, AutoRulesRepoError> {
            Ok(self
                .ledger
                .lock()
                .unwrap()
                .iter()
                .filter(|entry| {
                    entry.company_key == company_key && entry.created_at.as_str() >= since
                })
                .count() as u32)
        }

        fn ledger_last_at(&self) -> Result<Option<String>, AutoRulesRepoError> {
            Ok(self
                .ledger
                .lock()
                .unwrap()
                .iter()
                .map(|entry| entry.created_at.clone())
                .max())
        }
    }

    fn rule() -> AutoRule {
        AutoRule::defaults("rule_1", "Email × remote", NOW)
    }

    fn service(rules: FakeAutoRules, system: FakeSystemRepo) -> AutoRulesService {
        AutoRulesService::new(
            Arc::new(rules),
            Arc::new(system),
            Arc::new(FixedClock::new(NOW)),
        )
    }

    fn input() -> RuleInput {
        RuleInput {
            id: None,
            name: "Email × remote".into(),
            enabled: true,
            source_ids: vec!["remotive".into()],
            channels: vec!["email".into()],
            min_rank: 80.0,
            min_fit: 80.0,
            min_confidence: "medium".into(),
            allow_flags: false,
            min_verdict: "good".into(),
            daily_cap: 5,
            per_company_cap: 1,
            per_company_days: 30,
            active_start_minute: 540,
            active_end_minute: 1080,
        }
    }

    fn candidate() -> PolicyCandidate {
        PolicyCandidate {
            source_id: "remotive".into(),
            channel: RuleChannel::Email,
            company_key: "acme".into(),
            rank: Some(85.0),
            fit: Some(82.0),
            verdict: Verdict::Good,
            confidence: Confidence::Medium,
            gates: boba_domain::scoring::gates::Verdict::Pass,
            fraud_badge: false,
            documents_verified: true,
            language_variant_confirmed: true,
            duplicate: false,
            agent_used: false,
            cli_tool_lockdown: true,
            source_auto_opt_in: true,
            local_minute: 10 * 60,
            utc_offset_minutes: 0,
        }
    }

    #[test]
    fn a_fresh_install_reports_the_compiled_defaults() {
        let snapshot = service(FakeAutoRules::default(), FakeSystemRepo::default())
            .snapshot()
            .unwrap();
        assert!(!snapshot.auto_enabled);
        assert!(!snapshot.auto_confirmed);
        assert_eq!(snapshot.global_daily_cap, 10);
        assert!(snapshot.rules.is_empty());
    }

    #[test]
    fn a_rule_is_validated_by_core_before_it_is_stored() {
        let svc = service(FakeAutoRules::default(), FakeSystemRepo::default());

        let mut bad_cap = input();
        bad_cap.daily_cap = 26;
        assert!(matches!(
            svc.save_rule(bad_cap),
            Err(RulesError::InvalidRule(RuleViolation::DailyCapOutOfRange))
        ));

        let mut bad_channel = input();
        bad_channel.channels = vec!["carrier_pigeon".into()];
        assert!(matches!(
            svc.save_rule(bad_channel),
            Err(RulesError::UnknownChannel(_))
        ));

        let mut session = input();
        session.channels = vec!["session_board".into()];
        session.source_ids.clear();
        assert!(matches!(
            svc.save_rule(session),
            Err(RulesError::InvalidRule(
                RuleViolation::SessionBoardNeedsSource
            ))
        ));

        let mut bad_confidence = input();
        bad_confidence.min_confidence = "certain".into();
        assert!(matches!(
            svc.save_rule(bad_confidence),
            Err(RulesError::UnknownConfidence(_))
        ));

        let mut bad_verdict = input();
        bad_verdict.min_verdict = "great".into();
        assert!(matches!(
            svc.save_rule(bad_verdict),
            Err(RulesError::UnknownVerdict(_))
        ));

        assert!(
            svc.snapshot().unwrap().rules.is_empty(),
            "nothing was stored"
        );
    }

    #[test]
    fn a_saved_rule_round_trips_and_an_edit_keeps_its_birth() {
        let svc = service(FakeAutoRules::default(), FakeSystemRepo::default());
        let saved = svc.save_rule(input()).unwrap();
        assert!(saved.id.starts_with("rule_"));
        assert_eq!(saved.channels, vec!["email".to_string()]);
        assert_eq!(svc.snapshot().unwrap().rules.len(), 1);

        let mut edit = input();
        edit.id = Some(saved.id.clone());
        edit.name = "Edited".into();
        edit.daily_cap = 9;
        let edited = svc.save_rule(edit).unwrap();
        assert_eq!(edited.id, saved.id);
        assert_eq!(edited.name, "Edited");
        assert_eq!(edited.daily_cap, 9);
        assert_eq!(
            edited.created_at, saved.created_at,
            "an edit is not a new rule"
        );

        let mut missing = input();
        missing.id = Some("rule_ghost".into());
        assert!(matches!(
            svc.save_rule(missing),
            Err(RulesError::UnknownRule(_))
        ));
    }

    #[test]
    fn enabling_needs_the_dialog_and_a_rule() {
        let svc = service(FakeAutoRules::with_rule(rule()), FakeSystemRepo::default());

        assert!(matches!(
            svc.set_enabled(true, false),
            Err(RulesError::CannotEnable(EnableRefused::NotConfirmed))
        ));
        assert!(!svc.snapshot().unwrap().auto_enabled);

        let empty = service(FakeAutoRules::default(), FakeSystemRepo::default());
        assert!(matches!(
            empty.set_enabled(true, true),
            Err(RulesError::CannotEnable(EnableRefused::NoRules))
        ));

        let enabled = svc.set_enabled(true, true).unwrap();
        assert!(enabled.auto_enabled);
        assert!(enabled.auto_confirmed);

        let off = svc.set_enabled(false, false).unwrap();
        assert!(!off.auto_enabled);
        assert!(
            off.auto_confirmed,
            "turning Auto off keeps the confirmation; a reset clears it"
        );
    }

    #[test]
    fn the_global_cap_is_validated_by_core() {
        let svc = service(FakeAutoRules::default(), FakeSystemRepo::default());
        assert!(matches!(
            svc.set_global_cap(26),
            Err(RulesError::InvalidSettings(
                SettingsViolation::GlobalCapOutOfRange
            ))
        ));
        assert_eq!(svc.snapshot().unwrap().global_daily_cap, 10);
        assert_eq!(svc.set_global_cap(12).unwrap().global_daily_cap, 12);
        assert_eq!(svc.snapshot().unwrap().global_daily_cap, 12);
    }

    #[test]
    fn a_reset_turns_auto_off_and_restores_the_caps() {
        let mut stored = rule();
        stored.daily_cap = 25;
        stored.per_company_cap = 4;
        stored.per_company_days = 90;
        let svc = service(FakeAutoRules::with_rule(stored), FakeSystemRepo::default());
        svc.set_enabled(true, true).unwrap();

        let reset = svc.reset_to_defaults().unwrap();
        assert!(!reset.auto_enabled);
        assert!(!reset.auto_confirmed);
        assert_eq!(reset.global_daily_cap, 10);
        assert_eq!(reset.rules[0].daily_cap, 5);
        assert_eq!(reset.rules[0].per_company_cap, 1);
        assert_eq!(reset.rules[0].per_company_days, 30);
    }

    #[test]
    fn a_stored_value_outside_the_bounds_falls_back_to_the_defaults() {
        let mut system = FakeSystemRepo::default();
        system.seed_setting(GLOBAL_DAILY_CAP_KEY, "99");
        system.seed_setting(AUTO_ENABLED_KEY, "true");
        system.seed_setting(AUTO_CONFIRMED_AT_KEY, "2026-10-17T08:00:00Z");
        let snapshot = service(FakeAutoRules::default(), system)
            .snapshot()
            .unwrap();
        assert_eq!(snapshot.global_daily_cap, 10);
        assert!(!snapshot.auto_enabled);
    }

    #[test]
    fn an_enabled_switch_without_a_confirmation_reads_as_off() {
        let mut system = FakeSystemRepo::default();
        system.seed_setting(AUTO_ENABLED_KEY, "true");
        let snapshot = service(FakeAutoRules::with_rule(rule()), system)
            .snapshot()
            .unwrap();
        assert!(!snapshot.auto_enabled, "AC3: the dialog cannot be skipped");
    }

    #[test]
    fn decide_reads_the_policy_and_reports_the_reasons_verbatim() {
        let svc = service(FakeAutoRules::with_rule(rule()), FakeSystemRepo::default());

        // Auto off: Review with the one reason.
        let off = svc.decide(&candidate(), 3).unwrap();
        assert_eq!(off.kind, "review");
        assert_eq!(off.reasons[0].code, "auto_off");
        assert!(off.reasons[0].params.is_empty());

        svc.set_enabled(true, true).unwrap();
        let auto = svc.decide(&candidate(), 3).unwrap();
        assert_eq!(auto.kind, "auto");
        assert_eq!(auto.rule_id.as_deref(), Some("rule_1"));
        assert!(auto.reasons.is_empty());
    }

    #[test]
    fn the_caps_and_the_spacing_come_from_the_ledger() {
        let svc = service(FakeAutoRules::with_rule(rule()), FakeSystemRepo::default());
        svc.set_enabled(true, true).unwrap();

        svc.record_auto_send("rule_1", "app_1", "remotive", RuleChannel::Email, "acme")
            .unwrap();

        // The clock is fixed, so zero minutes have passed: the 5-minute gap
        // (sample 0) has not elapsed.
        let decision = svc.decide(&candidate(), 0).unwrap();
        assert_eq!(decision.kind, "review");
        let spacing = decision
            .reasons
            .iter()
            .find(|reason| reason.code == "spacing_not_elapsed")
            .expect("the 5-minute gap has not elapsed");
        assert_eq!(spacing.params.get("wait_minutes"), Some(&"5".to_string()));

        // The company cap is 1 per 30 days, so this same company is done for
        // the window; the spacing is not the only reason.
        assert!(
            decision
                .reasons
                .iter()
                .any(|reason| reason.code == "company_cap_reached"),
            "reasons: {:?}",
            decision.reasons
        );
    }

    #[test]
    fn the_daily_caps_reset_at_the_users_local_midnight() {
        // 09:00Z at UTC+10:00 is 19:00 local on the 17th, so the local day
        // began at 14:00Z on the 16th — six hours after UTC midnight.
        let mut user = candidate();
        user.utc_offset_minutes = 10 * 60;

        // 20:00Z on the 16th (06:00 local on the 17th) is inside the local day
        // that is running now: it must count against the rule's daily cap.
        let mut capped = rule();
        capped.daily_cap = 1;
        let svc = service(
            FakeAutoRules::with_ledger(capped.clone(), &["2026-10-16T20:00:00Z"]),
            FakeSystemRepo::default(),
        );
        svc.set_enabled(true, true).unwrap();
        let decision = svc.decide(&user, 0).unwrap();
        assert!(
            decision
                .reasons
                .iter()
                .any(|reason| reason.code == "rule_daily_cap_reached"),
            "reasons: {:?}",
            decision.reasons
        );

        // 10:00Z on the 16th (20:00 local on the 16th) belongs to yesterday
        // local: it must not count, although a UTC day would have kept it.
        let svc = service(
            FakeAutoRules::with_ledger(capped, &["2026-10-16T10:00:00Z"]),
            FakeSystemRepo::default(),
        );
        svc.set_enabled(true, true).unwrap();
        let decision = svc.decide(&user, 0).unwrap();
        assert!(
            !decision
                .reasons
                .iter()
                .any(|reason| reason.code == "rule_daily_cap_reached"),
            "reasons: {:?}",
            decision.reasons
        );
    }

    #[test]
    fn the_local_minute_is_inside_the_day() {
        let minute = local_minute(NOW);
        assert!(minute < 24 * 60, "got {minute}");
        assert_eq!(
            local_minute("not a timestamp"),
            0,
            "an unreadable clock is minute 0"
        );
    }

    #[test]
    fn the_local_minute_follows_the_given_offset() {
        // East of UTC: 09:00Z is 19:00 local at +10:00.
        assert_eq!(local_minute_at(NOW, 10 * 60), 19 * 60);
        // West of UTC: 02:00Z is 21:00 local the day before at -05:00.
        assert_eq!(local_minute_at("2026-10-17T02:00:00Z", -5 * 60), 21 * 60);
    }

    #[test]
    fn a_decision_serialises_to_the_shape_the_ui_switches_on() {
        let view = DecisionView {
            kind: "review".into(),
            rule_id: None,
            reasons: vec![ReasonView {
                code: "rank_below".into(),
                params: [("found".to_string(), "61".to_string())].into(),
            }],
        };
        let json = serde_json::to_value(&view).unwrap();
        assert_eq!(json["kind"], "review");
        assert_eq!(json["reasons"][0]["code"], "rank_below");
        assert_eq!(json["reasons"][0]["params"]["found"], "61");
    }
}
