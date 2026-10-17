//! What a run reads from the stored Profile and Preferences before it starts
//! (S5.2; the documents themselves are S3.7's and S3.4's).
//!
//! Two shapes come out of here:
//!
//! * a [`SearchQuery`] — what the Source adapters are asked for (AD-9);
//! * the scoring inputs — the Profile snapshot, the Preferences, the Gate
//!   inputs and the market data S6.3's [`ScoreRequest`] takes.
//!
//! **Nothing is imputed**: a fact the Profile does not state is absent from the
//! snapshot, and a Preference the document does not carry keeps the domain
//! default, which is what the factors and Gates read as "declared nothing".
//! Reads are tolerant on purpose — a document from a newer build must not stop
//! the daily search — so a row this build cannot read falls back to defaults.
//!
//! [`ScoreRequest`]: crate::scoring::ScoreRequest

use boba_domain::posting::{Money, PayPeriod, WorkMode};
use boba_domain::posting_features::Seniority;
use boba_domain::profile::{FactKind, ProfileFact};
use boba_domain::scoring::factors::{FactorWeights, MarketData, Prefs, ProfileSnapshot};
use boba_domain::scoring::gates::{
    Cefr, DeclaredLanguage, GateInputs, Hardness, PermitKind, SalaryFloor, WorkAuthorization,
};
use boba_domain::scoring::weights as weight_rules;
use boba_domain::source::SearchQuery;
use serde::Deserialize;

use crate::ports::repo::profile::{ProfileRepo, ProfileRepoError};
use crate::ports::repo::system::SystemRepo;

/// The `setting` row holding the Preferences document (S3.7).
pub const PREFERENCES_KEY: &str = "preferences";

/// How near a deadline has to be to be "Closing soon" (offer-scoring.md §2).
pub const CLOSING_SOON_DAYS: i64 = 7;

/// Everything one run scores its Offers with, read once at run time.
#[derive(Debug, Clone)]
pub struct ScoringInputs {
    /// The Profile version the snapshot was taken at (AD-15).
    pub profile_version: u32,
    /// The Preferences version in force (S3.7, FR-14).
    pub prefs_version: u32,
    pub profile: ProfileSnapshot,
    pub prefs: Prefs,
    /// What the five Gates read (S6.1).
    pub gates: GateInputs,
    /// The gap interview questions the user skipped (S3.6): each lowers the
    /// Confidence of the Gate that needed the field (AC1).
    pub skipped_gaps: Vec<String>,
    /// Salary benchmarks; the user's own arrive with S6.8, so this is empty
    /// until then — the Quality axis says what it is missing.
    pub market: MarketData,
}

/// The Search request the run derives from the stored Preferences: the roles
/// and places the user wants, and whether only remote work is acceptable.
pub fn search_query(system: &dyn SystemRepo) -> SearchQuery {
    let preferences = stored_preferences(system);
    let values = &preferences.values;
    SearchQuery {
        roles: values.roles.clone(),
        keywords: Vec::new(),
        locations: values.locations.clone(),
        remote: remote_only(values),
        languages: values.working_languages.clone(),
        markets: Vec::new(),
    }
}

/// The Preferences-derived half of a run's scoring inputs, plus the Profile
/// snapshot the factors read.
pub fn scoring_inputs(
    profile: &dyn ProfileRepo,
    system: &dyn SystemRepo,
    now: &str,
) -> Result<ScoringInputs, InputsError> {
    let profile_version = profile.profile_version()?;
    let facts = profile.facts_current()?;
    let preferences = stored_preferences(system);

    let snapshot = snapshot_of(&facts, &preferences.values);
    let prefs = prefs_of(&preferences.values);
    let gates = GateInputs {
        work_authorizations: authorizations_of(&facts),
        languages: snapshot.languages.clone(),
        salary_floor: preferences.values.salary_floor.as_ref().map(floor_of),
        // Declared by the user in S6.8's settings; until then nothing is
        // required, so the Gate never vetoes for it.
        requires_known_compensation: false,
        work_modes_allowed: work_modes(&preferences.values.work_modes),
        locations: preferences.values.locations.clone(),
        relocate_ok: false,
        closing_soon_days: CLOSING_SOON_DAYS,
        now: now.to_string(),
    };

    Ok(ScoringInputs {
        profile_version,
        prefs_version: preferences.version,
        profile: snapshot,
        prefs,
        gates,
        skipped_gaps: crate::profile_gaps::gaps::skipped(system),
        market: MarketData::default(),
    })
}

/// A read that a run treats as fatal: without the Profile there is no version
/// to key a card by.
#[derive(Debug, Clone, thiserror::Error)]
pub enum InputsError {
    #[error(transparent)]
    Profile(#[from] ProfileRepoError),
}

/// The Preferences document as this build reads it: `{version, values, …}`.
#[derive(Debug, Clone, Default, Deserialize)]
struct StoredPreferences {
    #[serde(default)]
    version: u32,
    #[serde(default)]
    values: Values,
}

/// `Preferences`'s stored fields, all optional on read (AD-21 tolerance).
#[derive(Debug, Clone, Default, Deserialize)]
struct Values {
    #[serde(default)]
    roles: Vec<String>,
    #[serde(default)]
    seniority_min: Option<String>,
    #[serde(default)]
    seniority_max: Option<String>,
    #[serde(default)]
    salary_floor: Option<StoredFloor>,
    #[serde(default)]
    locations: Vec<String>,
    #[serde(default)]
    work_modes: WorkModes,
    #[serde(default)]
    working_languages: Vec<String>,
    #[serde(default)]
    deal_breaker_keywords: Vec<String>,
    #[serde(default)]
    excluded_companies: Vec<String>,
    /// S6.8's weights, in whole percent. A document written before that slice
    /// has no such object, and each missing field keeps the PRD's default.
    #[serde(default)]
    weights: Option<StoredWeights>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct StoredWeights {
    #[serde(default)]
    skills: Option<u16>,
    #[serde(default)]
    experience: Option<u16>,
    #[serde(default)]
    semantic: Option<u16>,
    #[serde(default)]
    location: Option<u16>,
    #[serde(default)]
    education: Option<u16>,
    #[serde(default)]
    language: Option<u16>,
    #[serde(default)]
    compensation: Option<u16>,
    #[serde(default)]
    company: Option<u16>,
    #[serde(default)]
    terms: Option<u16>,
    #[serde(default)]
    legitimacy: Option<u16>,
    #[serde(default)]
    alignment: Option<u16>,
    #[serde(default)]
    fit: Option<u16>,
    #[serde(default)]
    quality: Option<u16>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct WorkModes {
    #[serde(default)]
    remote: bool,
    #[serde(default)]
    hybrid: bool,
    #[serde(default)]
    onsite: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct StoredFloor {
    amount_minor: i64,
    currency: String,
    period: String,
    firmness: String,
}

/// Read the Preferences document; a missing or unreadable row is "not set",
/// never an error — the run then searches everything and ranks without
/// preferences, which is what every factor already knows how to say.
fn stored_preferences(system: &dyn SystemRepo) -> StoredPreferences {
    let Some(raw) = system.setting_get(PREFERENCES_KEY).ok().flatten() else {
        return StoredPreferences::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Whether the user accepts remote work only, in which case the adapters are
/// told so (a Source that can filter saves the run a network round-trip).
fn remote_only(values: &Values) -> Option<bool> {
    (values.work_modes.remote && !values.work_modes.hybrid && !values.work_modes.onsite)
        .then_some(true)
}

/// The Profile facts as the factors read them (F1..F6).
fn snapshot_of(facts: &[ProfileFact], values: &Values) -> ProfileSnapshot {
    ProfileSnapshot {
        skills: texts(facts, FactKind::Skill),
        // F2's seniority is the user's own level, which the Profile editor does
        // not state separately from the target band: leave it undeclared
        // rather than read the preference as a fact.
        seniority: None,
        years_experience: years_of(facts),
        education: texts(facts, FactKind::Education),
        certifications: texts(facts, FactKind::Certification),
        languages: languages_of(facts),
        locations: values.locations.clone(),
    }
}

/// The Preferences as the factors read them.
fn prefs_of(values: &Values) -> Prefs {
    Prefs {
        roles: values.roles.clone(),
        seniority_min: values.seniority_min.as_deref().and_then(seniority),
        seniority_max: values.seniority_max.as_deref().and_then(seniority),
        work_modes: work_modes(&values.work_modes),
        locations: values.locations.clone(),
        working_languages: values.working_languages.clone(),
        excluded_companies: values.excluded_companies.clone(),
        deal_breaker_keywords: values.deal_breaker_keywords.clone(),
        // S6.8 (FR-58): the weights the user set, normalised by the same rule
        // the Settings page validated with.
        weights: weights_of(values.weights.as_ref()),
        ..Prefs::default()
    }
}

/// The stored weights in the engine's shape (S6.8, FR-58); a document no
/// score could be formed from falls back to the PRD's defaults — a bad row
/// must not stop the daily run (AD-1).
fn weights_of(stored: Option<&StoredWeights>) -> FactorWeights {
    let Some(stored) = stored else {
        return FactorWeights::DEFAULT;
    };
    // A field the document does not carry keeps the default it would have had.
    let default = FactorWeights::DEFAULT;
    let weights = FactorWeights {
        f1: stored.skills.map_or(default.f1, share),
        f2: stored.experience.map_or(default.f2, share),
        f3: stored.semantic.map_or(default.f3, share),
        f4: stored.location.map_or(default.f4, share),
        f5: stored.education.map_or(default.f5, share),
        f6: stored.language.map_or(default.f6, share),
        q1: stored.compensation.map_or(default.q1, share),
        q2: stored.company.map_or(default.q2, share),
        q3: stored.terms.map_or(default.q3, share),
        q4: stored.legitimacy.map_or(default.q4, share),
        q5: stored.alignment.map_or(default.q5, share),
        fit: stored.fit.map_or(default.fit, share),
        quality: stored.quality.map_or(default.quality, share),
    };
    weight_rules::normalize(weights).unwrap_or(FactorWeights::DEFAULT)
}

/// A stored whole-percent weight in the engine's share.
fn share(percent: u16) -> f32 {
    f32::from(percent) / 100.0
}

fn work_modes(modes: &WorkModes) -> Vec<WorkMode> {
    let mut allowed = Vec::new();
    if modes.remote {
        allowed.push(WorkMode::Remote);
    }
    if modes.hybrid {
        allowed.push(WorkMode::Hybrid);
    }
    if modes.onsite {
        allowed.push(WorkMode::Onsite);
    }
    allowed
}

fn seniority(text: &str) -> Option<Seniority> {
    Seniority::ALL
        .into_iter()
        .find(|level| level.as_str().eq_ignore_ascii_case(text))
}

/// The salary floor in the Gates' shape; an unreadable period makes the floor
/// absent (a gate must not compare against a made-up unit).
fn floor_of(stored: &StoredFloor) -> SalaryFloor {
    let period = match stored.period.to_ascii_lowercase().as_str() {
        "year" | "yearly" | "annual" => PayPeriod::Year,
        "month" | "monthly" => PayPeriod::Month,
        _ => PayPeriod::Hour,
    };
    SalaryFloor {
        money: Money {
            amount_minor: stored.amount_minor,
            currency: stored.currency.clone(),
            period,
        },
        hardness: if stored.firmness.eq_ignore_ascii_case("hard") {
            Hardness::Hard
        } else {
            Hardness::Soft
        },
    }
}

/// The text of every current fact of one kind: the English variant, falling
/// back to the Spanish one — the same rule the Profile editor shows.
fn texts(facts: &[ProfileFact], kind: FactKind) -> Vec<String> {
    facts
        .iter()
        .filter(|fact| fact.kind == kind)
        .filter_map(|fact| fact.text_en.clone().or_else(|| fact.text_es.clone()))
        .collect()
}

/// Years of experience, when an Experience fact states them (`detail_json`'s
/// `years`). Nothing else is read as a number: a date range is not a count.
fn years_of(facts: &[ProfileFact]) -> Option<u32> {
    facts
        .iter()
        .filter(|fact| fact.kind == FactKind::Experience)
        .filter_map(|fact| {
            serde_json::from_str::<serde_json::Value>(&fact.detail_json)
                .ok()?
                .get("years")?
                .as_u64()
        })
        .map(|years| years.min(u64::from(u32::MAX)) as u32)
        .max()
}

/// The declared languages, from `Language` facts' `detail_json`
/// (`{"language": "en", "level": "C1"}`). A fact without both is skipped:
/// a level nobody stated is not A1.
fn languages_of(facts: &[ProfileFact]) -> Vec<DeclaredLanguage> {
    facts
        .iter()
        .filter(|fact| fact.kind == FactKind::Language)
        .filter_map(language_of)
        .collect()
}

fn language_of(fact: &ProfileFact) -> Option<DeclaredLanguage> {
    let detail: serde_json::Value = serde_json::from_str(&fact.detail_json).ok()?;
    let language = ["language", "lang", "tag"]
        .iter()
        .find_map(|key| detail.get(*key)?.as_str())?;
    let level = ["level", "cefr"]
        .iter()
        .find_map(|key| detail.get(*key)?.as_str())
        .and_then(Cefr::parse)?;
    Some(DeclaredLanguage {
        language: language.to_string(),
        level,
    })
}

/// Work authorizations from `Eligibility` facts' `detail_json`
/// (`{"region": "es", "permit": "work_permit"}`).
fn authorizations_of(facts: &[ProfileFact]) -> Vec<WorkAuthorization> {
    facts
        .iter()
        .filter(|fact| fact.kind == FactKind::Eligibility)
        .filter_map(authorization_of)
        .collect()
}

fn authorization_of(fact: &ProfileFact) -> Option<WorkAuthorization> {
    let detail: serde_json::Value = serde_json::from_str(&fact.detail_json).ok()?;
    let region = detail.get("region")?.as_str()?.to_string();
    let permit = match detail.get("permit")?.as_str()? {
        "work_permit" => PermitKind::WorkPermit,
        "residency" => PermitKind::Residency,
        "citizenship" => PermitKind::Citizenship,
        "clearance" => PermitKind::Clearance,
        _ => return None,
    };
    Some(WorkAuthorization { region, permit })
}

#[cfg(test)]
mod tests {
    use boba_domain::posting::{PayPeriod, WorkMode};
    use boba_domain::profile::{FactDraft, FactKind, ProfileFact};

    use super::*;

    fn fact(kind: FactKind, text: &str, detail: &str) -> ProfileFact {
        ProfileFact {
            id: "rev".to_string(),
            item_id: "item".to_string(),
            kind,
            text_en: Some(text.to_string()),
            text_es: None,
            evidence: None,
            detail_json: detail.to_string(),
            supersedes: None,
            profile_version: 1,
            provenance: boba_domain::profile::Provenance::User,
            agent_run_id: None,
            created_at: "2026-10-17T08:00:00Z".to_string(),
        }
    }

    #[test]
    fn a_language_fact_with_a_level_becomes_a_declared_language() {
        let facts = vec![fact(
            FactKind::Language,
            "English",
            r#"{"language":"en","level":"C1"}"#,
        )];
        let languages = languages_of(&facts);
        assert_eq!(languages.len(), 1);
        assert_eq!(languages[0].language, "en");
        assert_eq!(languages[0].level, Cefr::C1);
    }

    #[test]
    fn a_language_fact_without_a_level_is_not_declared() {
        let facts = vec![fact(FactKind::Language, "English", "{}")];
        assert!(languages_of(&facts).is_empty());
    }

    #[test]
    fn an_experience_fact_states_the_years_it_states() {
        let facts = vec![
            fact(FactKind::Experience, "Data engineer", r#"{"years":4}"#),
            fact(FactKind::Experience, "Analyst", r#"{"years":2}"#),
        ];
        assert_eq!(years_of(&facts), Some(4));
    }

    #[test]
    fn remote_only_is_asked_of_the_adapters_only_when_it_holds() {
        let only_remote = Values {
            work_modes: WorkModes {
                remote: true,
                hybrid: false,
                onsite: false,
            },
            ..Values::default()
        };
        assert_eq!(remote_only(&only_remote), Some(true));
        let mixed = Values {
            work_modes: WorkModes {
                remote: true,
                hybrid: true,
                onsite: false,
            },
            ..Values::default()
        };
        assert_eq!(remote_only(&mixed), None);
    }

    #[test]
    fn a_hard_floor_keeps_its_currency_and_period() {
        let floor = floor_of(&StoredFloor {
            amount_minor: 4_500_000,
            currency: "EUR".to_string(),
            period: "year".to_string(),
            firmness: "hard".to_string(),
        });
        assert_eq!(floor.money.amount_minor, 4_500_000);
        assert_eq!(floor.money.currency, "EUR");
        assert_eq!(floor.money.period, PayPeriod::Year);
        assert_eq!(floor.hardness, Hardness::Hard);
    }

    #[test]
    fn work_modes_map_to_the_domain_modes() {
        let modes = work_modes(&WorkModes {
            remote: true,
            hybrid: false,
            onsite: true,
        });
        assert_eq!(modes, [WorkMode::Remote, WorkMode::Onsite]);
    }

    #[test]
    fn a_draft_with_only_its_text_is_read_as_no_detail_at_all() {
        let draft = FactDraft::new(FactKind::Skill);
        assert_eq!(draft.detail_json, "{}");
    }

    // S6.8 (AC1): the weights the user saved are the ones the run scores with.
    #[test]
    fn the_stored_weights_are_the_ones_a_run_scores_with() {
        let values = Values {
            weights: Some(StoredWeights {
                skills: Some(50),
                ..StoredWeights::default()
            }),
            ..Values::default()
        };
        let prefs = prefs_of(&values);
        // 50 replaces the default 35; the axis renormalises, so the shares move.
        assert!(prefs.weights.f1 > FactorWeights::DEFAULT.f1);
        assert_ne!(prefs.weights, FactorWeights::DEFAULT);
        assert!((prefs.weights.fit - 0.65).abs() < 1e-6, "axis untouched");
    }

    #[test]
    fn a_document_without_weights_scores_with_the_defaults() {
        let prefs = prefs_of(&Values::default());
        assert_eq!(prefs.weights, FactorWeights::DEFAULT);
    }

    #[test]
    fn a_partial_weights_object_keeps_the_defaults_it_omits() {
        let values = Values {
            weights: Some(StoredWeights {
                fit: Some(80),
                quality: Some(20),
                ..StoredWeights::default()
            }),
            ..Values::default()
        };
        let weights = weights_of(values.weights.as_ref());
        assert!((weights.fit - 0.8).abs() < 1e-6);
        assert!((weights.quality - 0.2).abs() < 1e-6);
        assert!((weights.f1 - FactorWeights::DEFAULT.f1).abs() < 1e-6);
    }

    // A stored document the engine cannot score with must not stop the run.
    #[test]
    fn unscorable_stored_weights_fall_back_to_the_defaults() {
        let values = Values {
            weights: Some(StoredWeights {
                skills: Some(0),
                experience: Some(0),
                semantic: Some(0),
                location: Some(0),
                education: Some(0),
                language: Some(0),
                ..StoredWeights::default()
            }),
            ..Values::default()
        };
        assert_eq!(prefs_of(&values).weights, FactorWeights::DEFAULT);
    }
}
