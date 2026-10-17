//! Global Auto settings and the first-enable confirmation (FR-83, AC3, FR-108).

use serde::{Deserialize, Serialize};

use super::rule::{
    AutoRule, DEFAULT_DAILY_CAP, DEFAULT_PER_COMPANY_CAP, DEFAULT_PER_COMPANY_DAYS, MAX_DAILY_CAP,
};

/// The compiled global daily cap (FR-83).
pub const DEFAULT_GLOBAL_DAILY_CAP: u8 = 10;

/// Everything about Auto that is not a rule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoSettings {
    /// Auto stays off until a rule exists and the user confirmed the dialog.
    pub auto_enabled: bool,
    /// When the consequences dialog was accepted, `None` until then (AC3).
    pub confirmed_at: Option<String>,
    /// The cap across every rule, per day (default 10).
    pub global_daily_cap: u8,
}

impl Default for AutoSettings {
    fn default() -> Self {
        Self {
            auto_enabled: false,
            confirmed_at: None,
            global_daily_cap: DEFAULT_GLOBAL_DAILY_CAP,
        }
    }
}

impl AutoSettings {
    /// Reject a range the PRD bounds (AD-11).
    pub fn validate(&self) -> Result<(), SettingsViolation> {
        if !(1..=MAX_DAILY_CAP).contains(&self.global_daily_cap) {
            return Err(SettingsViolation::GlobalCapOutOfRange);
        }
        Ok(())
    }

    /// Turn Auto on, only after the consequences dialog was confirmed and at
    /// least one enabled rule exists (AC3, FR-83).
    pub fn enable(
        &self,
        rules: &[AutoRule],
        confirmed: bool,
        now: &str,
    ) -> Result<Self, EnableRefused> {
        if !confirmed {
            return Err(EnableRefused::NotConfirmed);
        }
        if !rules.iter().any(|rule| rule.enabled) {
            return Err(EnableRefused::NoRules);
        }
        Ok(Self {
            auto_enabled: true,
            confirmed_at: Some(now.to_string()),
            ..self.clone()
        })
    }
}

/// Reset Apply to its defaults (FR-108, AC4).
///
/// Auto turns off and the confirmation is cleared, so enabling again asks
/// again; the global cap returns to 10 and every rule keeps its scope and
/// thresholds but goes back to the default caps. The rules are not deleted:
/// "reset the caps" is not "forget my rules".
pub fn reset_to_defaults(rules: &mut [AutoRule]) -> AutoSettings {
    for rule in rules.iter_mut() {
        rule.daily_cap = DEFAULT_DAILY_CAP;
        rule.per_company_cap = DEFAULT_PER_COMPANY_CAP;
        rule.per_company_days = DEFAULT_PER_COMPANY_DAYS;
    }
    AutoSettings::default()
}

/// Why a settings value is refused (AD-11).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingsViolation {
    #[error("the global daily cap must be between 1 and {MAX_DAILY_CAP}")]
    GlobalCapOutOfRange,
}

/// Why turning Auto on was refused (AC3).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnableRefused {
    #[error("the consequences dialog was not confirmed")]
    NotConfirmed,
    #[error("Auto needs at least one enabled rule")]
    NoRules,
}

#[cfg(test)]
mod tests {
    use super::super::rule::AutoRule;
    use super::*;

    fn rule() -> AutoRule {
        AutoRule::defaults("rule_1", "Email Apply", "2026-10-17T00:00:00Z")
    }

    #[test]
    fn the_defaults_are_off_and_capped_at_ten() {
        let settings = AutoSettings::default();
        assert!(!settings.auto_enabled, "FR-83: Auto is off until enabled");
        assert_eq!(settings.confirmed_at, None);
        assert_eq!(settings.global_daily_cap, DEFAULT_GLOBAL_DAILY_CAP);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn the_global_cap_stays_in_its_range() {
        let mut settings = AutoSettings {
            global_daily_cap: 0,
            ..AutoSettings::default()
        };
        assert_eq!(
            settings.validate(),
            Err(SettingsViolation::GlobalCapOutOfRange)
        );
        settings.global_daily_cap = MAX_DAILY_CAP + 1;
        assert_eq!(
            settings.validate(),
            Err(SettingsViolation::GlobalCapOutOfRange)
        );
        settings.global_daily_cap = MAX_DAILY_CAP;
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn enabling_needs_the_confirmation_and_a_rule() {
        let settings = AutoSettings::default();
        assert_eq!(
            settings.enable(&[rule()], false, "2026-10-17T01:00:00Z"),
            Err(EnableRefused::NotConfirmed),
            "AC3: without the dialog Auto stays off"
        );
        assert_eq!(
            settings.enable(&[], true, "2026-10-17T01:00:00Z"),
            Err(EnableRefused::NoRules),
            "FR-83: Auto is off until at least one rule exists"
        );

        let mut disabled = rule();
        disabled.enabled = false;
        assert_eq!(
            settings.enable(&[disabled], true, "2026-10-17T01:00:00Z"),
            Err(EnableRefused::NoRules),
            "a turned-off rule is not a rule Auto can use"
        );

        let enabled = settings
            .enable(&[rule()], true, "2026-10-17T01:00:00Z")
            .expect("confirmed + a rule");
        assert!(enabled.auto_enabled);
        assert_eq!(
            enabled.confirmed_at.as_deref(),
            Some("2026-10-17T01:00:00Z")
        );
    }

    #[test]
    fn reset_turns_auto_off_and_restores_the_default_caps() {
        let mut settings = AutoSettings {
            auto_enabled: true,
            confirmed_at: Some("2026-10-17T01:00:00Z".into()),
            global_daily_cap: 22,
        };
        let mut rules = vec![rule()];
        rules[0].daily_cap = 25;
        rules[0].per_company_cap = 7;
        rules[0].per_company_days = 90;
        rules[0].min_rank = 55.0;

        let reset = reset_to_defaults(&mut rules);

        assert_eq!(reset, AutoSettings::default());
        assert!(!reset.auto_enabled, "FR-108: reset turns Auto off");
        assert_eq!(reset.confirmed_at, None, "enabling again asks again");
        assert_eq!(rules[0].daily_cap, DEFAULT_DAILY_CAP);
        assert_eq!(rules[0].per_company_cap, DEFAULT_PER_COMPANY_CAP);
        assert_eq!(rules[0].per_company_days, DEFAULT_PER_COMPANY_DAYS);
        assert_eq!(
            rules[0].min_rank, 55.0,
            "reset restores caps, not thresholds"
        );
        assert_eq!(
            settings.global_daily_cap, 22,
            "the argument itself is left alone"
        );
        settings.auto_enabled = false; // silence the read warning for the fixture
    }
}
