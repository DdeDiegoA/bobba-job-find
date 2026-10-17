//! `decide()` — Auto, Review or Block (S9.6, FR-83/FR-87, AC2).

use serde::{Deserialize, Serialize};

use crate::scoring::gates::{Confidence, Verdict as GateVerdict};
use crate::scoring::rank::Verdict;

use super::rule::{AutoRule, RuleChannel};
use super::settings::AutoSettings;

/// The facts about one candidate Auto may send (AC2). The callers that own each
/// fact fill it: the scoring card, the package verification, the duplicate guard
/// (S9.5), the fraud badge (S6.x) and the agent runtime.
#[derive(Debug, Clone, PartialEq)]
pub struct PolicyCandidate {
    pub source_id: String,
    pub channel: RuleChannel,
    /// The company in the form the per-company cap counts it (`acme`).
    pub company_key: String,
    pub rank: Option<f32>,
    pub fit: Option<f32>,
    pub verdict: Verdict,
    pub confidence: Confidence,
    /// The worst of the five gates (FR-53).
    pub gates: GateVerdict,
    /// The offer carries a fraud badge (FR-60).
    pub fraud_badge: bool,
    /// Every document in the Package is Ready and verified (AD-14).
    pub documents_verified: bool,
    /// No document's language variant is waiting for the user's word.
    pub language_variant_confirmed: bool,
    /// An Application to the same company and similar role already exists
    /// inside the duplicate window (FR-92).
    pub duplicate: bool,
    /// Whether the Package was produced with an agent CLI.
    pub agent_used: bool,
    /// That CLI's verified `tool_lockdown` (ADR-010).
    pub cli_tool_lockdown: bool,
    /// The Source's own Auto opt-in is active (FR-91). Only a Session Board
    /// candidate needs it; the rule is the opt-in for every other channel.
    pub source_auto_opt_in: bool,
    /// The Session Board Auto facts (S11.3, FR-91): `Some` exactly for a
    /// `SessionBoard` candidate, counted by the caller from the ledger.
    pub session_auto: Option<SessionAutoFacts>,
    /// Minute of the user's local day, for the active hours (FR-83).
    pub local_minute: u16,
    /// The user's offset from UTC in minutes (east positive). The caps count
    /// the local day, so the ledger window starts at their local midnight —
    /// the same clock the active hours use (S9.6 review F1).
    pub utc_offset_minutes: i32,
}

/// The per-Source Session Board limits, read by the caller (S11.3, FR-91).
///
/// They are stricter than any rule's: a default of three a day (hard maximum
/// ten) and at least ten minutes between submissions on the same Source. The
/// caller counts them, so `decide` stays a pure function of its arguments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionAutoFacts {
    /// The Source's daily cap: three unless the user raised it, never above 10.
    pub daily_cap: u8,
    /// Automatic submissions to this Source today (the local day).
    pub today: u32,
    /// Whole minutes since the newest automatic submission there, `None` when
    /// the Source never took one.
    pub minutes_since_last: Option<u32>,
}

/// What already went out, as the caps read it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PolicyCounts {
    /// Attempts recorded today, across every rule.
    pub global_today: u32,
    /// Attempts recorded today per rule id (missing = 0).
    pub per_rule_today: Vec<(String, u32)>,
    /// Attempts per rule id inside that rule's per-company window.
    pub per_company: Vec<(String, u32)>,
    /// Minutes since the last Auto submission, `None` when there was none.
    pub minutes_since_last_auto: Option<u32>,
    /// A caller-supplied random sample: the 5–20 minute gap is derived from it
    /// (AC4), so the rule is pure and the test is deterministic.
    pub spacing_sample: u64,
}

/// What `decide` concluded (AC2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Decision {
    /// Auto may send, under this rule.
    Auto { rule_id: String },
    /// A human keeps the decision; the reasons say what to look at.
    Review { reasons: Vec<Reason> },
    /// Auto never sends this one (FR-87); the reasons are the hard limits.
    Block { reasons: Vec<Reason> },
}

/// One reason, in the exact form the UI shows (AC2). `code` is the i18n leaf
/// under `applying:reason.`; the params interpolate into it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "code", content = "params", rename_all = "snake_case")]
pub enum Reason {
    AutoOff,
    AutoNotConfirmed,
    NoRuleForSource,
    NoRuleForChannel {
        channel: RuleChannel,
    },
    RankMissing,
    RankBelow {
        found: f32,
        min: f32,
    },
    FitMissing,
    FitBelow {
        found: f32,
        min: f32,
    },
    ConfidenceBelow {
        found: Confidence,
        min: Confidence,
    },
    VerdictBelow {
        found: Verdict,
        min: Verdict,
    },
    FlagNotAllowed,
    Vetoed,
    FraudBadge,
    DocumentsUnverified,
    LanguageVariantUnconfirmed,
    Duplicate,
    CliWithoutLockdown,
    SourceWithoutOptIn,
    GlobalCapReached {
        cap: u8,
    },
    RuleDailyCapReached {
        cap: u8,
    },
    CompanyCapReached {
        cap: u8,
        days: u16,
    },
    OutsideActiveHours {
        start_minute: u16,
        end_minute: u16,
    },
    SpacingNotElapsed {
        wait_minutes: u32,
    },
    /// The Session Board Source's own cap is spent (S11.3, FR-91).
    SessionCapReached {
        cap: u8,
    },
    /// The ten-minute floor between two sends on that Source (S11.3, FR-91).
    SessionSpacingNotElapsed {
        wait_minutes: u32,
    },
}

impl Reason {
    /// The stable i18n leaf (`applying:reason.<code>`).
    pub fn code(&self) -> &'static str {
        match self {
            Reason::AutoOff => "auto_off",
            Reason::AutoNotConfirmed => "auto_not_confirmed",
            Reason::NoRuleForSource => "no_rule_for_source",
            Reason::NoRuleForChannel { .. } => "no_rule_for_channel",
            Reason::RankMissing => "rank_missing",
            Reason::RankBelow { .. } => "rank_below",
            Reason::FitMissing => "fit_missing",
            Reason::FitBelow { .. } => "fit_below",
            Reason::ConfidenceBelow { .. } => "confidence_below",
            Reason::VerdictBelow { .. } => "verdict_below",
            Reason::FlagNotAllowed => "flag_not_allowed",
            Reason::Vetoed => "vetoed",
            Reason::FraudBadge => "fraud_badge",
            Reason::DocumentsUnverified => "documents_unverified",
            Reason::LanguageVariantUnconfirmed => "language_variant_unconfirmed",
            Reason::Duplicate => "duplicate",
            Reason::CliWithoutLockdown => "cli_without_lockdown",
            Reason::SourceWithoutOptIn => "source_without_opt_in",
            Reason::GlobalCapReached { .. } => "global_cap_reached",
            Reason::RuleDailyCapReached { .. } => "rule_daily_cap_reached",
            Reason::CompanyCapReached { .. } => "company_cap_reached",
            Reason::OutsideActiveHours { .. } => "outside_active_hours",
            Reason::SpacingNotElapsed { .. } => "spacing_not_elapsed",
            Reason::SessionCapReached { .. } => "session_cap_reached",
            Reason::SessionSpacingNotElapsed { .. } => "session_spacing_not_elapsed",
        }
    }

    /// The interpolation values, in the order the copy names them.
    pub fn params(&self) -> Vec<(&'static str, String)> {
        match self {
            Reason::AutoOff
            | Reason::AutoNotConfirmed
            | Reason::NoRuleForSource
            | Reason::RankMissing
            | Reason::FitMissing
            | Reason::FlagNotAllowed
            | Reason::Vetoed
            | Reason::FraudBadge
            | Reason::DocumentsUnverified
            | Reason::LanguageVariantUnconfirmed
            | Reason::Duplicate
            | Reason::CliWithoutLockdown
            | Reason::SourceWithoutOptIn => Vec::new(),
            Reason::NoRuleForChannel { channel } => {
                vec![("channel", channel.as_str().to_string())]
            }
            Reason::RankBelow { found, min } => number_params(*found, *min),
            Reason::FitBelow { found, min } => number_params(*found, *min),
            Reason::ConfidenceBelow { found, min } => vec![
                ("found", confidence_str(*found).to_string()),
                ("min", confidence_str(*min).to_string()),
            ],
            Reason::VerdictBelow { found, min } => vec![
                ("found", found.as_str().to_string()),
                ("min", min.as_str().to_string()),
            ],
            Reason::GlobalCapReached { cap } => vec![("cap", cap.to_string())],
            Reason::RuleDailyCapReached { cap } => vec![("cap", cap.to_string())],
            Reason::CompanyCapReached { cap, days } => {
                vec![("cap", cap.to_string()), ("days", days.to_string())]
            }
            Reason::OutsideActiveHours {
                start_minute,
                end_minute,
            } => vec![
                ("start_minute", start_minute.to_string()),
                ("end_minute", end_minute.to_string()),
            ],
            Reason::SpacingNotElapsed { wait_minutes } => {
                vec![("wait_minutes", wait_minutes.to_string())]
            }
            Reason::SessionCapReached { cap } => vec![("cap", cap.to_string())],
            Reason::SessionSpacingNotElapsed { wait_minutes } => {
                vec![("wait_minutes", wait_minutes.to_string())]
            }
        }
    }
}

/// `found` / `min` as the copy interpolates them: `61`, not `61.0`.
fn number_params(found: f32, min: f32) -> Vec<(&'static str, String)> {
    vec![("found", format_number(found)), ("min", format_number(min))]
}

fn format_number(value: f32) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.1}")
    }
}

/// The stored form of a [`Confidence`], mirroring its serde rename.
fn confidence_str(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Low => "low",
        Confidence::Medium => "medium",
        Confidence::High => "high",
    }
}

/// The band's position, best highest: `Strong` 4 … `Poor` 0.
fn verdict_rank(verdict: Verdict) -> u8 {
    match verdict {
        Verdict::Strong => 4,
        Verdict::Good => 3,
        Verdict::Moderate => 2,
        Verdict::Weak => 1,
        Verdict::Poor => 0,
    }
}

/// Auto, Review or Block (AC2).
pub fn decide(
    settings: &AutoSettings,
    rules: &[AutoRule],
    candidate: &PolicyCandidate,
    counts: &PolicyCounts,
) -> Decision {
    // The hard limits come first: they hold whatever the rules or the Auto
    // switch say, so a review queue always names them (FR-87).
    let blocks = hard_limits(candidate);
    if !blocks.is_empty() {
        return Decision::Block { reasons: blocks };
    }

    if !settings.auto_enabled {
        return Decision::Review {
            reasons: vec![Reason::AutoOff],
        };
    }
    if settings.confirmed_at.is_none() {
        return Decision::Review {
            reasons: vec![Reason::AutoNotConfirmed],
        };
    }

    let mut in_channel: Vec<&AutoRule> = rules
        .iter()
        .filter(|rule| rule.enabled && rule.covers_source(&candidate.source_id))
        .collect();
    if in_channel.is_empty() {
        return Decision::Review {
            reasons: vec![Reason::NoRuleForSource],
        };
    }
    in_channel.retain(|rule| rule.covers_channel(candidate.channel));
    if in_channel.is_empty() {
        return Decision::Review {
            reasons: vec![Reason::NoRuleForChannel {
                channel: candidate.channel,
            }],
        };
    }

    // A rule that names the Source was written for it, so it is tried before a
    // catch-all; the id breaks ties so the order is stable across runs.
    in_channel.sort_by(|a, b| {
        (a.source_ids.is_empty(), a.id.as_str()).cmp(&(b.source_ids.is_empty(), b.id.as_str()))
    });

    let mut closest: Option<Vec<Reason>> = None;
    for rule in &in_channel {
        let reasons = rule_reasons(rule, settings, candidate, counts);
        if reasons.is_empty() {
            return Decision::Auto {
                rule_id: rule.id.clone(),
            };
        }
        if closest
            .as_ref()
            .is_none_or(|best| reasons.len() < best.len())
        {
            closest = Some(reasons);
        }
    }

    Decision::Review {
        reasons: closest.unwrap_or_default(),
    }
}

/// The reasons Auto never sends, whatever the rule says (FR-87, AC2).
fn hard_limits(candidate: &PolicyCandidate) -> Vec<Reason> {
    let mut reasons = Vec::new();
    if candidate.gates == GateVerdict::Veto {
        reasons.push(Reason::Vetoed);
    }
    if candidate.fraud_badge {
        reasons.push(Reason::FraudBadge);
    }
    if !candidate.documents_verified {
        reasons.push(Reason::DocumentsUnverified);
    }
    if !candidate.language_variant_confirmed {
        reasons.push(Reason::LanguageVariantUnconfirmed);
    }
    if candidate.duplicate {
        reasons.push(Reason::Duplicate);
    }
    if candidate.agent_used && !candidate.cli_tool_lockdown {
        reasons.push(Reason::CliWithoutLockdown);
    }
    if candidate.channel == RuleChannel::SessionBoard && !candidate.source_auto_opt_in {
        reasons.push(Reason::SourceWithoutOptIn);
    }
    reasons
}

/// Every reason this rule refuses the candidate, in a stable order.
fn rule_reasons(
    rule: &AutoRule,
    settings: &AutoSettings,
    candidate: &PolicyCandidate,
    counts: &PolicyCounts,
) -> Vec<Reason> {
    let mut reasons = Vec::new();

    match candidate.rank {
        None => reasons.push(Reason::RankMissing),
        Some(found) if found < rule.min_rank => reasons.push(Reason::RankBelow {
            found,
            min: rule.min_rank,
        }),
        Some(_) => {}
    }
    match candidate.fit {
        None => reasons.push(Reason::FitMissing),
        Some(found) if found < rule.min_fit => reasons.push(Reason::FitBelow {
            found,
            min: rule.min_fit,
        }),
        Some(_) => {}
    }
    if candidate.confidence < rule.min_confidence {
        reasons.push(Reason::ConfidenceBelow {
            found: candidate.confidence,
            min: rule.min_confidence,
        });
    }
    if verdict_rank(candidate.verdict) < verdict_rank(rule.min_verdict) {
        reasons.push(Reason::VerdictBelow {
            found: candidate.verdict,
            min: rule.min_verdict,
        });
    }
    if candidate.gates == GateVerdict::Flag && !rule.allow_flags {
        reasons.push(Reason::FlagNotAllowed);
    }
    if !rule.active_hours.contains(candidate.local_minute) {
        reasons.push(Reason::OutsideActiveHours {
            start_minute: rule.active_hours.start_minute,
            end_minute: rule.active_hours.end_minute,
        });
    }
    if counts.global_today >= u32::from(settings.global_daily_cap) {
        reasons.push(Reason::GlobalCapReached {
            cap: settings.global_daily_cap,
        });
    }
    if count_for(&counts.per_rule_today, &rule.id) >= u32::from(rule.daily_cap) {
        reasons.push(Reason::RuleDailyCapReached {
            cap: rule.daily_cap,
        });
    }
    if count_for(&counts.per_company, &rule.id) >= u32::from(rule.per_company_cap) {
        reasons.push(Reason::CompanyCapReached {
            cap: rule.per_company_cap,
            days: rule.per_company_days,
        });
    }
    // S11.3 (FR-91): a Session Board Source keeps its own, stricter limits on
    // top of the rule. `decide` runs at every send point, so a spent cap or a
    // too-short gap stops the send there too.
    if candidate.channel == RuleChannel::SessionBoard
        && let Some(session) = &candidate.session_auto
    {
        if session.today >= u32::from(session.daily_cap) {
            reasons.push(Reason::SessionCapReached {
                cap: session.daily_cap,
            });
        }
        if let Some(minutes) = session.minutes_since_last {
            let gap = super::super::session_policy::MIN_SUBMISSION_GAP_MINUTES;
            if minutes < gap {
                reasons.push(Reason::SessionSpacingNotElapsed {
                    wait_minutes: gap - minutes,
                });
            }
        }
    }
    if let Some(minutes) = counts.minutes_since_last_auto {
        let gap = super::pace::gap_minutes(counts.spacing_sample);
        if minutes < gap {
            reasons.push(Reason::SpacingNotElapsed {
                wait_minutes: gap - minutes,
            });
        }
    }
    reasons
}

fn count_for(counts: &[(String, u32)], rule_id: &str) -> u32 {
    counts
        .iter()
        .find(|(id, _)| id == rule_id)
        .map(|(_, count)| *count)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::policy::rule::AutoRule;
    use crate::apply::policy::settings::AutoSettings;
    use crate::scoring::gates::Confidence;
    use crate::scoring::rank::Verdict;

    const NOW: &str = "2026-10-17T09:00:00Z";

    fn settings_on() -> AutoSettings {
        AutoSettings {
            auto_enabled: true,
            confirmed_at: Some(NOW.into()),
            ..AutoSettings::default()
        }
    }

    fn rule() -> AutoRule {
        AutoRule::defaults("rule_1", "Email × remote", NOW)
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
            gates: GateVerdict::Pass,
            fraud_badge: false,
            documents_verified: true,
            language_variant_confirmed: true,
            duplicate: false,
            agent_used: false,
            cli_tool_lockdown: true,
            source_auto_opt_in: true,
            session_auto: None,
            local_minute: 10 * 60,
            utc_offset_minutes: 0,
        }
    }

    fn counts() -> PolicyCounts {
        PolicyCounts::default()
    }

    fn decide_one(settings: &AutoSettings, rules: &[AutoRule], c: &PolicyCandidate) -> Decision {
        decide(settings, rules, c, &counts())
    }

    fn reasons(decision: &Decision) -> Vec<Reason> {
        match decision {
            Decision::Review { reasons } | Decision::Block { reasons } => reasons.clone(),
            Decision::Auto { .. } => panic!("expected a reason, got {decision:?}"),
        }
    }

    #[test]
    fn auto_off_is_review_and_a_confirmation_is_required() {
        let off = AutoSettings::default();
        assert_eq!(
            decide_one(&off, &[rule()], &candidate()),
            Decision::Review {
                reasons: vec![Reason::AutoOff]
            }
        );

        let unconfirmed = AutoSettings {
            auto_enabled: true,
            confirmed_at: None,
            ..AutoSettings::default()
        };
        assert_eq!(
            decide_one(&unconfirmed, &[rule()], &candidate()),
            Decision::Review {
                reasons: vec![Reason::AutoNotConfirmed]
            },
            "AC3: enabled without the consequences dialog stays off"
        );
    }

    #[test]
    fn the_hard_limits_block_even_when_auto_is_on() {
        let settings = settings_on();
        let rules = vec![rule()];

        type Mutation = (&'static str, Box<dyn Fn(&mut PolicyCandidate)>, Reason);
        let cases: Vec<Mutation> = vec![
            (
                "veto",
                Box::new(|c| c.gates = GateVerdict::Veto),
                Reason::Vetoed,
            ),
            (
                "fraud",
                Box::new(|c| c.fraud_badge = true),
                Reason::FraudBadge,
            ),
            (
                "documents",
                Box::new(|c| c.documents_verified = false),
                Reason::DocumentsUnverified,
            ),
            (
                "language",
                Box::new(|c| c.language_variant_confirmed = false),
                Reason::LanguageVariantUnconfirmed,
            ),
            (
                "duplicate",
                Box::new(|c| c.duplicate = true),
                Reason::Duplicate,
            ),
            (
                "lockdown",
                Box::new(|c| {
                    c.agent_used = true;
                    c.cli_tool_lockdown = false;
                }),
                Reason::CliWithoutLockdown,
            ),
        ];

        for (name, mutate, reason) in cases {
            let mut c = candidate();
            mutate(&mut c);
            match decide(&settings, &rules, &c, &counts()) {
                Decision::Block { reasons } => assert_eq!(reasons, vec![reason], "{name}"),
                other => panic!("{name}: {other:?}"),
            }
        }
    }

    #[test]
    fn a_hard_limit_wins_over_auto_being_off() {
        let mut c = candidate();
        c.gates = GateVerdict::Veto;
        assert!(matches!(
            decide(&AutoSettings::default(), &[rule()], &c, &counts()),
            Decision::Block { .. }
        ));
    }

    #[test]
    fn the_session_board_needs_the_sources_own_opt_in() {
        let mut rule = rule();
        rule.channels = vec![RuleChannel::SessionBoard];
        rule.source_ids = vec!["linkedin".into()];
        let mut c = candidate();
        c.source_id = "linkedin".into();
        c.channel = RuleChannel::SessionBoard;
        c.source_auto_opt_in = false;

        assert_eq!(
            decide(&settings_on(), &[rule.clone()], &c, &counts()),
            Decision::Block {
                reasons: vec![Reason::SourceWithoutOptIn]
            }
        );
        c.source_auto_opt_in = true;
        assert_eq!(
            decide(&settings_on(), &[rule], &c, &counts()),
            Decision::Auto {
                rule_id: "rule_1".into()
            }
        );
    }

    #[test]
    fn a_session_board_send_keeps_its_own_cap_and_ten_minute_gap() {
        let mut session_rule = rule();
        session_rule.channels = vec![RuleChannel::SessionBoard];
        session_rule.source_ids = vec!["linkedin".into()];
        // The rule itself allows ten; the Source's own cap is stricter.
        session_rule.daily_cap = 10;

        let mut c = candidate();
        c.source_id = "linkedin".into();
        c.channel = RuleChannel::SessionBoard;

        let facts = |cap: u8, today: u32, since: Option<u32>| SessionAutoFacts {
            daily_cap: cap,
            today,
            minutes_since_last: since,
        };

        c.session_auto = Some(facts(3, 2, Some(10)));
        assert!(
            matches!(
                decide(&settings_on(), &[session_rule.clone()], &c, &counts()),
                Decision::Auto { .. }
            ),
            "two today, ten minutes ago: the send may go"
        );

        c.session_auto = Some(facts(3, 3, Some(10)));
        assert_eq!(
            decide(&settings_on(), &[session_rule.clone()], &c, &counts()),
            Decision::Review {
                reasons: vec![Reason::SessionCapReached { cap: 3 }]
            },
            "FR-91: three a day by default, whatever the rule allows"
        );

        c.session_auto = Some(facts(10, 9, Some(10)));
        assert!(matches!(
            decide(&settings_on(), &[session_rule.clone()], &c, &counts()),
            Decision::Auto { .. }
        ));

        c.session_auto = Some(facts(3, 1, Some(4)));
        assert_eq!(
            decide(&settings_on(), &[session_rule.clone()], &c, &counts()),
            Decision::Review {
                reasons: vec![Reason::SessionSpacingNotElapsed { wait_minutes: 6 }]
            },
            "FR-91: at least ten minutes between two submissions on a Source"
        );

        c.session_auto = Some(facts(3, 0, None));
        assert!(matches!(
            decide(&settings_on(), &[session_rule.clone()], &c, &counts()),
            Decision::Auto { .. }
        ));

        let mut email = candidate();
        email.session_auto = Some(facts(1, 99, Some(0)));
        let email_rule = rule();
        assert!(
            matches!(
                decide(&settings_on(), &[email_rule], &email, &counts()),
                Decision::Auto { .. }
            ),
            "the session facts never gate another channel"
        );
    }

    #[test]
    fn a_flag_is_a_review_unless_the_rule_allows_it() {
        let settings = settings_on();
        let mut c = candidate();
        c.gates = GateVerdict::Flag;

        assert_eq!(
            reasons(&decide(&settings, &[rule()], &c, &counts())),
            vec![Reason::FlagNotAllowed],
            "FR-83: FLAGs are off by default"
        );

        let mut permissive = rule();
        permissive.allow_flags = true;
        assert!(matches!(
            decide(&settings, &[permissive], &c, &counts()),
            Decision::Auto { .. }
        ));
    }

    #[test]
    fn every_threshold_is_a_reason_and_they_are_all_listed() {
        let settings = settings_on();
        let mut c = candidate();
        c.rank = Some(61.0);
        c.fit = None;
        c.verdict = Verdict::Moderate;
        c.confidence = Confidence::Low;

        let got = reasons(&decide(&settings, &[rule()], &c, &counts()));
        assert_eq!(
            got,
            vec![
                Reason::RankBelow {
                    found: 61.0,
                    min: 80.0
                },
                Reason::FitMissing,
                Reason::ConfidenceBelow {
                    found: Confidence::Low,
                    min: Confidence::Medium
                },
                Reason::VerdictBelow {
                    found: Verdict::Moderate,
                    min: Verdict::Good
                },
            ]
        );

        let mut only_rank = candidate();
        only_rank.rank = None;
        assert_eq!(
            reasons(&decide(&settings, &[rule()], &only_rank, &counts())),
            vec![Reason::RankMissing]
        );
    }

    #[test]
    fn a_rule_only_covers_its_sources_and_channels() {
        let settings = settings_on();

        let mut scoped = rule();
        scoped.source_ids = vec!["linkedin".into()];
        assert_eq!(
            reasons(&decide(&settings, &[scoped], &candidate(), &counts())),
            vec![Reason::NoRuleForSource]
        );

        assert!(
            matches!(
                decide(&settings, &[rule()], &candidate(), &counts()),
                Decision::Auto { .. }
            ),
            "sanity: the baseline is an Auto decision"
        );

        let mut c = candidate();
        c.channel = RuleChannel::AtsForm;
        assert_eq!(
            reasons(&decide(&settings, &[rule()], &c, &counts())),
            vec![Reason::NoRuleForChannel {
                channel: RuleChannel::AtsForm
            }]
        );
    }

    #[test]
    fn a_disabled_rule_never_sends() {
        let mut off = rule();
        off.enabled = false;
        assert_eq!(
            reasons(&decide(&settings_on(), &[off], &candidate(), &counts())),
            vec![Reason::NoRuleForSource],
            "a turned-off rule is not a rule"
        );
    }

    #[test]
    fn the_caps_stop_auto_at_the_rule_and_at_the_company() {
        let settings = settings_on();

        let mut c = candidate();
        c.local_minute = 10 * 60;

        let mut at_global = counts();
        at_global.global_today = settings.global_daily_cap as u32;
        assert_eq!(
            decide(&settings, &[rule()], &c, &at_global),
            Decision::Review {
                reasons: vec![Reason::GlobalCapReached {
                    cap: settings.global_daily_cap
                }]
            }
        );

        let mut at_rule = counts();
        at_rule.per_rule_today = vec![("rule_1".into(), 5)];
        assert_eq!(
            decide(&settings, &[rule()], &c, &at_rule),
            Decision::Review {
                reasons: vec![Reason::RuleDailyCapReached { cap: 5 }]
            }
        );

        let mut at_company = counts();
        at_company.per_company = vec![("rule_1".into(), 1)];
        assert_eq!(
            decide(&settings, &[rule()], &c, &at_company),
            Decision::Review {
                reasons: vec![Reason::CompanyCapReached { cap: 1, days: 30 }]
            }
        );

        let mut below = counts();
        below.global_today = 2;
        below.per_rule_today = vec![("rule_1".into(), 2)];
        below.per_company = vec![("rule_1".into(), 0)];
        assert!(matches!(
            decide(&settings, &[rule()], &c, &below),
            Decision::Auto { .. }
        ));
    }

    #[test]
    fn active_hours_gate_the_send() {
        let mut c = candidate();
        c.local_minute = 21 * 60;
        assert_eq!(
            reasons(&decide(&settings_on(), &[rule()], &c, &counts())),
            vec![Reason::OutsideActiveHours {
                start_minute: 9 * 60,
                end_minute: 18 * 60
            }]
        );
        c.local_minute = 9 * 60;
        assert!(matches!(
            decide(&settings_on(), &[rule()], &c, &counts()),
            Decision::Auto { .. }
        ));
    }

    #[test]
    fn automatic_sends_are_spaced_five_to_twenty_minutes() {
        let settings = settings_on();
        let c = candidate();

        let mut too_soon = counts();
        too_soon.minutes_since_last_auto = Some(3);
        too_soon.spacing_sample = 0; // gap of 5
        assert_eq!(
            reasons(&decide(&settings, &[rule()], &c, &too_soon)),
            vec![Reason::SpacingNotElapsed { wait_minutes: 2 }]
        );

        let mut far_enough = counts();
        far_enough.minutes_since_last_auto = Some(5);
        far_enough.spacing_sample = 0;
        assert!(
            matches!(
                decide(&settings, &[rule()], &c, &far_enough),
                Decision::Auto { .. }
            ),
            "the gap itself is enough"
        );

        let mut longest = counts();
        longest.minutes_since_last_auto = Some(19);
        longest.spacing_sample = 15; // gap of 20
        assert_eq!(
            reasons(&decide(&settings, &[rule()], &c, &longest)),
            vec![Reason::SpacingNotElapsed { wait_minutes: 1 }]
        );

        let mut first_ever = counts();
        first_ever.minutes_since_last_auto = None;
        assert!(matches!(
            decide(&settings, &[rule()], &c, &first_ever),
            Decision::Auto { .. }
        ));
    }

    #[test]
    fn the_rule_most_specific_to_the_source_wins() {
        let settings = settings_on();
        let general = rule();
        let mut specific = rule();
        specific.id = "rule_2".into();
        specific.source_ids = vec!["remotive".into()];

        assert_eq!(
            decide(&settings, &[general, specific], &candidate(), &counts()),
            Decision::Auto {
                rule_id: "rule_2".into()
            },
            "an explicit source scope beats a catch-all"
        );
    }

    #[test]
    fn every_failing_rule_is_reported_and_the_closest_one_is_shown() {
        let settings = settings_on();
        let mut far = rule();
        far.id = "rule_far".into();
        far.min_rank = 99.0;
        far.min_fit = 99.0;

        let mut close = rule();
        close.id = "rule_close".into();
        close.min_rank = 90.0;

        let got = reasons(&decide(&settings, &[far, close], &candidate(), &counts()));
        assert_eq!(
            got,
            vec![Reason::RankBelow {
                found: 85.0,
                min: 90.0
            }],
            "the closest rule explains why nothing matched"
        );
    }

    #[test]
    fn a_company_key_groups_the_same_employer() {
        assert_eq!(company_key("Acme Corp"), "acme");
        assert_eq!(company_key("ACME"), "acme");
        assert_eq!(company_key("Bogotá Labs S.A.S."), "bogotá-labs");
        assert_eq!(company_key("Nimbus GmbH"), "nimbus");
        assert_eq!(
            company_key("Tapi Tea Inc."),
            "tapi-tea",
            "punctuation is a separator"
        );
        assert_eq!(company_key(""), "");
        assert_eq!(
            company_key("Corp"),
            "corp",
            "a company named only with its legal form keeps a word"
        );
    }

    #[test]
    fn the_ledger_entry_carries_the_facts_the_caps_read() {
        let entry = AutoAttemptEntry {
            id: new_auto_attempt_id(),
            rule_id: "rule_1".into(),
            application_id: "app_1".into(),
            source_id: "remotive".into(),
            channel: RuleChannel::Email,
            company_key: company_key("Acme Corp"),
            created_at: "2026-10-17T09:00:00Z".into(),
        };
        assert!(entry.id.starts_with("aut_"));
        assert_eq!(entry.company_key, "acme");
    }

    #[test]
    fn reason_codes_and_params_are_the_ui_contract() {
        let below = Reason::RankBelow {
            found: 61.0,
            min: 80.0,
        };
        assert_eq!(below.code(), "rank_below");
        assert_eq!(
            below.params(),
            vec![("found", "61".to_string()), ("min", "80".to_string())]
        );

        assert_eq!(Reason::AutoOff.code(), "auto_off");
        assert_eq!(Reason::AutoOff.params(), vec![]);
        assert_eq!(
            Reason::ConfidenceBelow {
                found: Confidence::Low,
                min: Confidence::Medium
            }
            .params(),
            vec![("found", "low".to_string()), ("min", "medium".to_string())]
        );
        assert_eq!(
            Reason::OutsideActiveHours {
                start_minute: 540,
                end_minute: 1080
            }
            .params(),
            vec![
                ("start_minute", "540".to_string()),
                ("end_minute", "1080".to_string())
            ]
        );
    }
}

/// A fresh ULID for one auto-attempt ledger row, prefixed by type (AD-17).
pub fn new_auto_attempt_id() -> String {
    format!("aut_{}", ulid::Ulid::generate())
}

/// One automatic submission, as the caps count it (S9.6, FR-83).
///
/// The row is written when Auto decides to send, before the channel does
/// anything, so a send that dies mid-flight still counts against the caps and
/// the next run cannot forget it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutoAttemptEntry {
    pub id: String,
    pub rule_id: String,
    pub application_id: String,
    pub source_id: String,
    pub channel: RuleChannel,
    /// The company, normalised by [`company_key`].
    pub company_key: String,
    pub created_at: String,
}

/// The company as the per-company cap counts it: lower-case, no legal form, one
/// `-` between words (`Acme Corp S.A.S.` → `acme`).
///
/// It mirrors `dedup::key::company_key` (a different module, one purpose each):
/// the cap must recognise `Acme Corp` and `acme` as the same employer.
pub fn company_key(company: &str) -> String {
    let mut words: Vec<String> = company
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect();
    loop {
        if words.len() < 2 {
            break;
        }
        // A dotted legal form (`S.A.S.`) splits into single letters: try the
        // trailing run as one word before trying the last word alone.
        let tail: String = words
            .iter()
            .rev()
            .take_while(|word| word.len() == 1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .cloned()
            .collect();
        if !tail.is_empty() && is_legal_form(&tail) {
            words.truncate(words.len() - tail.len());
            continue;
        }
        if is_legal_form(&words[words.len() - 1]) {
            words.pop();
            continue;
        }
        break;
    }
    words.join("-")
}

/// A trailing word that names the legal form, not the employer.
fn is_legal_form(word: &str) -> bool {
    matches!(
        word,
        "inc"
            | "llc"
            | "ltd"
            | "corp"
            | "corporation"
            | "gmbh"
            | "sa"
            | "sas"
            | "sl"
            | "bv"
            | "co"
            | "company"
            | "limited"
            | "srl"
            | "ag"
    )
}
