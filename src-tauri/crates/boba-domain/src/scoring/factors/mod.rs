//! Fit (F1–F6) and Quality (Q1–Q5): the eleven factors, their weights and the
//! inputs they read (FR-55, FR-56, AD-8, `offer-scoring.md` §§3–5).
//!
//! Every factor answers the same six things — `{score, weight, matched,
//! missing, evidence, confidence}` — so the UI and the narration explain any
//! Offer the same way (FR-59: an LLM narrates these values, it never sets one).
//!
//! Two rules run through all eleven (AC2, AD-8):
//!
//! * **a missing input is never imputed.** `score` is `None`, the factor's
//!   confidence drops, and the factors that *are* present share its weight
//!   ([`weights::weighted_mean`]). No embeddings means no F3; no benchmark
//!   means no Q1 — the axis is still a number, and the card says on which
//!   evidence it rests;
//! * **nothing is read that the caller did not hand over.** The factors take
//!   the Offer's own normalised fields ([`Offer`]: what Q3 and Q4 read), S6.2's
//!   features (what the posting *asks*), the Profile snapshot, the Preferences,
//!   market data and the injected clock. No IO, no LLM, no clock of its own —
//!   the same inputs always produce the same numbers (FR-61).
//!
//! The scoring-side value object of the weights lives here ([`FactorWeights`]);
//! the Settings page that persists a user's overrides owns
//! `boba_domain::scoring::weights` (S6.8) and maps onto it.

use serde::{Deserialize, Serialize};

use crate::posting::{Offer, PayPeriod, WorkMode};
use crate::posting_features::{PostingFeatures, Seniority};
use crate::scoring::gates::{Confidence, DeclaredLanguage, Evidence};

mod alignment;
mod comp;
mod company;
mod education;
mod experience;
mod language;
mod legitimacy;
mod location;
mod semantic;
mod skills;
mod terms;
pub mod weights;

pub use semantic::SemanticScore;
pub use weights::{FactorWeights, weighted_mean};

/// The eleven factors of `offer-scoring.md` §§3–5, in report order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Factor {
    Skills,
    Experience,
    Semantic,
    Location,
    Education,
    Language,
    Compensation,
    Company,
    Terms,
    Legitimacy,
    Alignment,
}

/// Which axis a factor belongs to (FR-55, FR-56).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    /// "Can I get it?"
    Fit,
    /// "Do I want it?"
    Quality,
}

impl Factor {
    /// F1–F6, in weight order as the PRD lists them.
    pub const FIT: [Factor; 6] = [
        Factor::Skills,
        Factor::Experience,
        Factor::Semantic,
        Factor::Location,
        Factor::Education,
        Factor::Language,
    ];

    /// Q1–Q5, in weight order as the PRD lists them.
    pub const QUALITY: [Factor; 5] = [
        Factor::Compensation,
        Factor::Company,
        Factor::Terms,
        Factor::Legitimacy,
        Factor::Alignment,
    ];

    /// Every factor, Fit first.
    pub const ALL: [Factor; 11] = [
        Factor::Skills,
        Factor::Experience,
        Factor::Semantic,
        Factor::Location,
        Factor::Education,
        Factor::Language,
        Factor::Compensation,
        Factor::Company,
        Factor::Terms,
        Factor::Legitimacy,
        Factor::Alignment,
    ];

    /// The axis this factor is scored on.
    pub fn axis(self) -> Axis {
        match self {
            Factor::Skills
            | Factor::Experience
            | Factor::Semantic
            | Factor::Location
            | Factor::Education
            | Factor::Language => Axis::Fit,
            Factor::Compensation
            | Factor::Company
            | Factor::Terms
            | Factor::Legitimacy
            | Factor::Alignment => Axis::Quality,
        }
    }

    /// The stored / serialised form: the model's own id for the factor, which
    /// `score_card.factors_json` and the UI both keep (F1 skills, Q4 legitimacy…).
    pub fn as_str(self) -> &'static str {
        match self {
            Factor::Skills => "f1_skills",
            Factor::Experience => "f2_experience",
            Factor::Semantic => "f3_semantic",
            Factor::Location => "f4_location",
            Factor::Education => "f5_education",
            Factor::Language => "f6_language",
            Factor::Compensation => "q1_compensation",
            Factor::Company => "q2_company",
            Factor::Terms => "q3_terms",
            Factor::Legitimacy => "q4_legitimacy",
            Factor::Alignment => "q5_alignment",
        }
    }

    /// Parse the stored form, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|factor| factor.as_str() == value)
    }
}

/// One factor's answer (FR-55, FR-56).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FactorScore {
    pub factor: Factor,
    /// 0–100, or `None` when the input this factor reads is missing (AC2: the
    /// axis renormalises over the others; nothing is imputed).
    pub score: Option<f32>,
    /// The configured weight (FR-58: the user may change it; S6.8 persists it).
    pub weight: f32,
    /// The share of the axis this factor actually carried: its weight over the
    /// present factors' weights, and `0.0` when the factor is missing.
    pub effective_weight: f32,
    /// What this factor found in favour of the Offer, as the model names it.
    pub matched: Vec<String>,
    /// What it found against: a skill the Profile does not back, a signal it
    /// cannot see. These are shown, never filled in (AC2).
    pub missing: Vec<String>,
    /// Where the values came from: the posting's own words (FR-53's
    /// [`Evidence`]), or the field this factor read.
    pub evidence: Vec<Evidence>,
    pub confidence: Confidence,
}

impl FactorScore {
    /// A factor that read its inputs: `score` is `Some`, confidence says how
    /// well it could see them.
    pub fn scored(
        factor: Factor,
        score: f32,
        weight: f32,
        confidence: Confidence,
        evidence: Vec<Evidence>,
    ) -> Self {
        Self {
            factor,
            score: Some(clamp_percent(score)),
            weight,
            effective_weight: weight,
            matched: Vec::new(),
            missing: Vec::new(),
            evidence,
            confidence,
        }
    }

    /// A factor whose input is not there (AC2): no score, low confidence, and
    /// the reason named in `missing` so the UI can say what would change it.
    pub fn missing_input(factor: Factor, weight: f32, reason: &str) -> Self {
        Self {
            factor,
            score: None,
            weight,
            effective_weight: 0.0,
            matched: Vec::new(),
            missing: vec![reason.to_string()],
            evidence: Vec::new(),
            confidence: Confidence::Low,
        }
    }

    /// Builder: what this factor found in favour.
    pub fn with_matched(mut self, matched: Vec<String>) -> Self {
        self.matched = matched;
        self
    }

    /// Builder: what it found against.
    pub fn with_missing(mut self, missing: Vec<String>) -> Self {
        if self.score.is_some() {
            self.missing = missing;
        }
        self
    }

    /// Builder: the factor read its inputs but could not see all of them.
    pub fn with_confidence(mut self, confidence: Confidence) -> Self {
        if self.score.is_some() {
            self.confidence = confidence;
        }
        self
    }
}

/// 0–100, and never NaN (an untrusted number must not poison the card).
pub(crate) fn clamp_percent(value: f32) -> f32 {
    if value.is_nan() {
        return 0.0;
    }
    value.clamp(0.0, 100.0)
}

/// The Profile as one Offer is scored against (FR-11..13, AD-15).
///
/// A snapshot, not a live read: the card is keyed by the `profile_version` this
/// was taken at, so a later Profile change re-scores instead of rewriting
/// (FR-13, FR-14, AC3). Each field names the Profile section it comes from; the
/// caller maps the stored facts onto it, and this module reads nothing else.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileSnapshot {
    /// What the user can do, from the `skill` facts (F1).
    pub skills: Vec<String>,
    /// The user's own level, when the Profile declares one (F2).
    pub seniority: Option<Seniority>,
    /// Years of experience the Profile states, when it states one (F2).
    pub years_experience: Option<u32>,
    /// Degrees the Profile states, in the user's own words (F5).
    pub education: Vec<String>,
    /// Certifications the Profile states (F5).
    pub certifications: Vec<String>,
    /// Declared working languages with their CEFR levels (F6, FR-53's gate).
    pub languages: Vec<DeclaredLanguage>,
    /// Places the user lives or can work from (F4).
    pub locations: Vec<String>,
}

impl ProfileSnapshot {
    /// An empty Profile: every factor that needs it is missing, which is a
    /// correct answer for a user who has not filled anything in yet.
    pub fn empty() -> Self {
        Self {
            skills: Vec::new(),
            seniority: None,
            years_experience: None,
            education: Vec::new(),
            certifications: Vec::new(),
            languages: Vec::new(),
            locations: Vec::new(),
        }
    }
}

/// What the user is looking for (FR-7, S3.7): the part of the Preferences the
/// factors read. The stored document is the Settings one; this is the snapshot
/// a re-score was asked for, with the version it was read at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    /// FR-58: the weights in force for this re-score.
    pub weights: FactorWeights,
    /// Roles the user is looking for, in their own words (F3, Q5).
    pub roles: Vec<String>,
    /// The seniority band the user targets (F2, Q5).
    pub seniority_min: Option<Seniority>,
    pub seniority_max: Option<Seniority>,
    /// Work modes the user accepts (F4, FR-7).
    pub work_modes: Vec<WorkMode>,
    /// Places the user accepts (`"madrid"`, `"remote"`); empty means anywhere.
    pub locations: Vec<String>,
    /// Whether the user would relocate for the right role (F4).
    pub relocate_ok: bool,
    /// BCP-47 languages the user works in (F6).
    pub working_languages: Vec<String>,
    /// Companies the user never wants to see again (Q5).
    pub excluded_companies: Vec<String>,
    /// Words that make an Offer a no for the user (Q3).
    pub deal_breaker_keywords: Vec<String>,
}

impl Default for Prefs {
    /// An untouched Preferences document: the PRD's default weights and no
    /// declared preference at all. Every preference-dependent factor then says
    /// what it is missing instead of guessing.
    fn default() -> Self {
        Self {
            weights: FactorWeights::DEFAULT,
            roles: Vec::new(),
            seniority_min: None,
            seniority_max: None,
            work_modes: Vec::new(),
            locations: Vec::new(),
            relocate_ok: false,
            working_languages: Vec::new(),
            excluded_companies: Vec::new(),
            deal_breaker_keywords: Vec::new(),
        }
    }
}

/// Where a salary benchmark came from, in the order Q1 prefers them
/// (`offer-scoring.md` §4: the user's own table, then BLS OES, then Adzuna).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkBasis {
    /// The user's own imported salary table.
    User,
    /// BLS OES (US, public domain).
    Bls,
    /// The Adzuna salary endpoint, with the user's key.
    Adzuna,
}

impl BenchmarkBasis {
    /// Every basis, most trusted first.
    pub const ALL: [BenchmarkBasis; 3] = [
        BenchmarkBasis::User,
        BenchmarkBasis::Bls,
        BenchmarkBasis::Adzuna,
    ];

    /// The stored form; the UI shows the basis next to Q1 (FR-56).
    pub fn as_str(self) -> &'static str {
        match self {
            BenchmarkBasis::User => "user",
            BenchmarkBasis::Bls => "bls",
            BenchmarkBasis::Adzuna => "adzuna",
        }
    }

    /// Parse the stored form, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|basis| basis.as_str() == value)
    }
}

/// One market benchmark for a role in a place (Q1). `p50_minor` is the median in
/// the AD-17 money shape, so a comparison is always same-currency and
/// same-period (or the factor is missing, never converted by guesswork).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SalaryBenchmark {
    /// The role, as the benchmark names it (`"data engineer"`).
    pub role: String,
    /// The place, lowercase (`"madrid"`, `"remote"`).
    pub location: String,
    /// ISO-4217.
    pub currency: String,
    pub period: PayPeriod,
    pub p50_minor: i64,
    pub basis: BenchmarkBasis,
    /// UTC RFC 3339: how fresh the benchmark is (shown with Q1).
    pub as_of: String,
}

/// Cached company research (Q2, FR-56's MVP note: only what an agent task or
/// the user already gathered, never a live fetch during scoring).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompanySignals {
    /// The company as the cache names it; matched case-insensitively.
    pub company: String,
    /// Funding trajectory, `Some(false)` for a reported down-round — the
    /// signal `offer-scoring.md` §9's worked example is dragged by. `None` is
    /// "not known", never "healthy".
    pub funding_up: Option<bool>,
    /// Layoff mentions the cached research found; `Some(0)` is "none found".
    pub layoffs: Option<u32>,
    /// Review rating 0–5, when the cache holds one.
    pub rating: Option<f32>,
    /// Where the cache came from, shown as the basis (FR-56).
    pub source: String,
    /// UTC RFC 3339: the cache's age (a 30-day TTL upstream; the reader decides).
    pub as_of: String,
}

/// The market side of Q1 and Q2: benchmarks and cached company research.
/// Empty is the honest default — no data is not neutral data (AD-8).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MarketData {
    pub salary: Vec<SalaryBenchmark>,
    pub companies: Vec<CompanySignals>,
}

impl MarketData {
    /// The benchmark for one role and place, prefering the user's own table
    /// (`offer-scoring.md` §4's order); `None` when nothing matches.
    pub fn benchmark(&self, role: &str, location: &str) -> Option<&SalaryBenchmark> {
        BenchmarkBasis::ALL.into_iter().find_map(|basis| {
            self.salary.iter().find(|benchmark| {
                benchmark.basis == basis
                    && benchmark.role.eq_ignore_ascii_case(role)
                    && benchmark.location.eq_ignore_ascii_case(location)
            })
        })
    }

    /// The cached research for one company, matched case-insensitively.
    pub fn company(&self, company: &str) -> Option<&CompanySignals> {
        self.companies
            .iter()
            .find(|signals| signals.company.eq_ignore_ascii_case(company))
    }
}

/// Everything one Offer is scored from (AC1).
///
/// AD-8 sketches this as `score(&PostingFeatures, &ProfileSnapshot, &Prefs,
/// &MarketData)`. The four are here, under those names, plus the two more the
/// posting-side factors genuinely need and no feature set carries: the Offer's
/// own normalised fields ([`Offer`]: work mode, pay, age, title — Q3 and Q4 read
/// them) and the injected clock (AD-8: the factors never read one).
#[derive(Debug, Clone, Copy)]
pub struct ScoreInputs<'a> {
    /// What the posting asks for (S6.2, FR-61: cached per Profile version).
    pub features: &'a PostingFeatures,
    /// The Offer itself, as S4.2 normalised it (FR-48).
    pub posting: &'a Offer,
    pub profile: &'a ProfileSnapshot,
    pub prefs: &'a Prefs,
    pub market: &'a MarketData,
    /// F3's cosine similarity of the Offer's text and the Profile, when the
    /// on-device encoder ran (S6.9). `None` without embeddings: F3 drops out
    /// and its weight is renormalised (AC2).
    pub semantic: Option<semantic::SemanticScore>,
    /// The injected clock, UTC RFC 3339 (AD-17): Q4 reads a posting's age.
    pub now: &'a str,
}

/// The six Fit factors, in [`Factor::FIT`] order (FR-55).
pub fn fit(inputs: &ScoreInputs<'_>) -> Vec<FactorScore> {
    let weights = inputs.prefs.weights;
    vec![
        skills::score(inputs, weights.of(Factor::Skills)),
        experience::score(inputs, weights.of(Factor::Experience)),
        semantic::score(inputs, weights.of(Factor::Semantic)),
        location::score(inputs, weights.of(Factor::Location)),
        education::score(inputs, weights.of(Factor::Education)),
        language::score(inputs, weights.of(Factor::Language)),
    ]
}

/// The five Quality factors, in [`Factor::QUALITY`] order (FR-56).
pub fn quality(inputs: &ScoreInputs<'_>) -> Vec<FactorScore> {
    let weights = inputs.prefs.weights;
    vec![
        comp::score(inputs, weights.of(Factor::Compensation)),
        company::score(inputs, weights.of(Factor::Company)),
        terms::score(inputs, weights.of(Factor::Terms)),
        legitimacy::score(inputs, weights.of(Factor::Legitimacy)),
        alignment::score(inputs, weights.of(Factor::Alignment)),
    ]
}

#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_factor_round_trips_through_its_stored_id() {
        assert_eq!(Factor::ALL.len(), 11);
        for factor in Factor::ALL {
            assert_eq!(Factor::parse(factor.as_str()), Some(factor));
        }
        assert_eq!(Factor::parse("f7_luck"), None);
        assert_eq!(Factor::FIT.len() + Factor::QUALITY.len(), 11);
    }

    #[test]
    fn the_axes_hold_their_own_factors() {
        assert_eq!(
            Factor::ALL
                .into_iter()
                .filter(|f| f.axis() == Axis::Fit)
                .collect::<Vec<_>>(),
            Factor::FIT.to_vec()
        );
        assert_eq!(
            Factor::ALL
                .into_iter()
                .filter(|f| f.axis() == Axis::Quality)
                .collect::<Vec<_>>(),
            Factor::QUALITY.to_vec()
        );
        assert_eq!(Factor::Semantic.axis(), Axis::Fit);
        assert_eq!(Factor::Legitimacy.axis(), Axis::Quality);
    }

    #[test]
    fn a_score_is_held_inside_zero_and_hundred() {
        assert_eq!(clamp_percent(-4.0), 0.0);
        assert_eq!(clamp_percent(104.0), 100.0);
        assert_eq!(clamp_percent(f32::NAN), 0.0);
        assert_eq!(clamp_percent(72.5), 72.5);
    }

    #[test]
    fn a_missing_input_carries_no_score_and_low_confidence() {
        let factor = FactorScore::missing_input(Factor::Semantic, 0.15, "no embeddings installed");
        assert_eq!(factor.score, None);
        assert_eq!(factor.effective_weight, 0.0);
        assert_eq!(factor.confidence, Confidence::Low);
        assert_eq!(factor.missing, vec!["no embeddings installed".to_string()]);
    }

    #[test]
    fn a_missing_score_ignores_the_builders_that_would_claim_otherwise() {
        let factor = FactorScore::missing_input(Factor::Compensation, 0.30, "no benchmark")
            .with_missing(vec!["ignored".to_string()])
            .with_confidence(Confidence::High);
        assert_eq!(factor.missing, vec!["no benchmark".to_string()]);
        assert_eq!(factor.confidence, Confidence::Low);
    }

    #[test]
    fn benchmarks_and_companies_are_found_case_insensitively() {
        let market = MarketData {
            salary: vec![SalaryBenchmark {
                role: "Data Engineer".to_string(),
                location: "Madrid".to_string(),
                currency: "EUR".to_string(),
                period: PayPeriod::Year,
                p50_minor: 5_200_000,
                basis: BenchmarkBasis::User,
                as_of: "2026-10-01T00:00:00Z".to_string(),
            }],
            companies: vec![CompanySignals {
                company: "Boba Labs".to_string(),
                funding_up: Some(false),
                layoffs: Some(3),
                rating: None,
                source: "cached research".to_string(),
                as_of: "2026-10-01T00:00:00Z".to_string(),
            }],
        };
        assert!(market.benchmark("data engineer", "madrid").is_some());
        assert!(market.benchmark("data engineer", "bogota").is_none());
        assert!(market.company("boba labs").is_some());
        assert!(market.company("other").is_none());
    }

    #[test]
    fn the_users_own_benchmark_wins_over_the_public_ones() {
        let market = MarketData {
            salary: vec![
                SalaryBenchmark {
                    role: "data engineer".to_string(),
                    location: "madrid".to_string(),
                    currency: "EUR".to_string(),
                    period: PayPeriod::Year,
                    p50_minor: 5_200_000,
                    basis: BenchmarkBasis::Adzuna,
                    as_of: "2026-10-01T00:00:00Z".to_string(),
                },
                SalaryBenchmark {
                    role: "data engineer".to_string(),
                    location: "madrid".to_string(),
                    currency: "EUR".to_string(),
                    period: PayPeriod::Year,
                    p50_minor: 4_800_000,
                    basis: BenchmarkBasis::User,
                    as_of: "2026-10-01T00:00:00Z".to_string(),
                },
            ],
            companies: Vec::new(),
        };
        let found = market
            .benchmark("data engineer", "madrid")
            .expect("a benchmark");
        assert_eq!(found.basis, BenchmarkBasis::User);
        assert_eq!(found.p50_minor, 4_800_000);
    }

    #[test]
    fn a_prefs_document_with_nothing_set_uses_the_prd_weights() {
        let prefs = Prefs::default();
        assert_eq!(prefs.weights, FactorWeights::DEFAULT);
        assert!(prefs.roles.is_empty());
        assert!(!prefs.relocate_ok);
    }

    #[test]
    fn an_empty_profile_leaves_every_factor_to_say_what_it_misses() {
        let profile = ProfileSnapshot::empty();
        assert!(profile.skills.is_empty());
        assert_eq!(profile.seniority, None);
    }
}
