//! Fixtures for the factor tests (test-only).
//!
//! The default Offer and Profile are `offer-scoring.md` §9's worked example —
//! a mid-level data engineer in Madrid (Python/SQL/dbt/Airflow, Spanish native,
//! English B2) looking at a Senior Data Engineer role in remote Spain — so a
//! factor test reads like the research document it implements.

use crate::posting::{ApplyHints, Money, Offer, PayPeriod, WorkMode};
use crate::posting_features::{Claim, FeatureKind, PostingFeatures, Seniority};
use crate::scoring::gates::{Cefr, Confidence, DeclaredLanguage, Evidence, QuotedSpan};

use super::{MarketData, Prefs, ProfileSnapshot, ScoreInputs, semantic::SemanticScore};

/// The clock every factor test runs at (AD-8: injected, never read).
pub(crate) const NOW: &str = "2026-10-16T00:00:00Z";

/// A claim the way S6.2's deterministic reader writes one.
pub(crate) fn claim(kind: FeatureKind, value: &str) -> Claim {
    Claim::deterministic(
        kind,
        value,
        Confidence::High,
        Evidence::Field("description_text".to_string()),
    )
}

pub(crate) fn skill(value: &str) -> Claim {
    claim(FeatureKind::Skill, value)
}

pub(crate) fn seniority(value: Seniority) -> Claim {
    claim(FeatureKind::Seniority, value.as_str())
}

pub(crate) fn education(value: &str) -> Claim {
    claim(FeatureKind::Education, value)
}

pub(crate) fn language(value: &str) -> Claim {
    claim(FeatureKind::Language, value)
}

pub(crate) fn red_flag(value: &str) -> Claim {
    claim(FeatureKind::RedFlag, value)
}

/// A quoted span of `text`, as the UI quotes it (FR-53).
pub(crate) fn quoted(text: &str, start: usize, end: usize) -> Evidence {
    Evidence::Quoted(QuotedSpan {
        text: text[start..end].to_string(),
        start,
        end,
    })
}

/// §9's posting: "Senior Data Engineer", remote Spain, €55–70k, posted 12 days
/// before [`NOW`], on a board, for a company with a down-round behind it.
pub(crate) const SECTION_9_TEXT: &str = "Senior Data Engineer. Remote Spain. We build our \
    analytics platform on dbt, Airflow and Snowflake. You will own pipelines end to end and work \
    closely with analysts. Permanent contract, equity, health insurance. Compensation: 55.000 EUR \
    per year.";

pub(crate) fn section_9_posting() -> Offer {
    Offer {
        id: "pst_01SECTION9".to_string(),
        source_id: "greenhouse".to_string(),
        external_id: "gh-42".to_string(),
        title: "Senior Data Engineer".to_string(),
        company: "Boba Labs".to_string(),
        locations: vec!["Remote, Spain".to_string()],
        work_mode: WorkMode::Remote,
        money: Some(Money {
            amount_minor: 5_500_000,
            currency: "EUR".to_string(),
            period: PayPeriod::Year,
        }),
        posted_at: Some("2026-10-04T00:00:00Z".to_string()),
        deadline: None,
        first_seen: "2026-10-04T00:00:00Z".to_string(),
        last_seen: NOW.to_string(),
        lang: "en".to_string(),
        description_html: format!("<p>{SECTION_9_TEXT}</p>"),
        description_text: SECTION_9_TEXT.to_string(),
        requirements: vec![
            "dbt".to_string(),
            "Airflow".to_string(),
            "Snowflake".to_string(),
        ],
        apply: ApplyHints {
            url: "https://boards.example.com/boba/gh-42".to_string(),
            email: None,
        },
    }
}

/// §9's features: the skills the posting asks for and its stated level.
pub(crate) fn section_9_features() -> PostingFeatures {
    let mut features = PostingFeatures::new("pst_01SECTION9", 3);
    features.extend([
        skill("dbt"),
        skill("Airflow"),
        skill("Snowflake"),
        seniority(Seniority::Senior),
        language("en"),
    ]);
    features
}

/// §9's candidate: mid-level, Python/SQL/dbt/Airflow, Spanish native, English
/// B2, in Madrid, remote-preferred.
pub(crate) fn section_9_profile() -> ProfileSnapshot {
    ProfileSnapshot {
        skills: vec![
            "Python".to_string(),
            "SQL".to_string(),
            "dbt".to_string(),
            "Airflow".to_string(),
        ],
        seniority: Some(Seniority::Mid),
        years_experience: Some(5),
        education: vec!["BSc Computer Science".to_string()],
        certifications: Vec::new(),
        languages: vec![
            DeclaredLanguage {
                language: "es".to_string(),
                level: Cefr::C2,
            },
            DeclaredLanguage {
                language: "en".to_string(),
                level: Cefr::B2,
            },
        ],
        locations: vec!["Madrid, ES".to_string(), "remote".to_string()],
    }
}

/// §9's Preferences: remote, Spain, an English/Spanish worker, data roles.
pub(crate) fn section_9_prefs() -> Prefs {
    Prefs {
        roles: vec!["data engineer".to_string()],
        work_modes: vec![WorkMode::Remote, WorkMode::Hybrid],
        locations: vec!["madrid".to_string(), "spain".to_string()],
        working_languages: vec!["en".to_string(), "es".to_string()],
        ..Prefs::default()
    }
}

/// §9's market data: a Madrid p50 for the role and the company's cached
/// research (a down-round and repeated layoff mentions).
pub(crate) fn section_9_market() -> MarketData {
    use super::{CompanySignals, SalaryBenchmark};

    MarketData {
        salary: vec![SalaryBenchmark {
            role: "data engineer".to_string(),
            location: "madrid".to_string(),
            currency: "EUR".to_string(),
            period: PayPeriod::Year,
            p50_minor: 5_200_000,
            basis: super::BenchmarkBasis::User,
            as_of: "2026-09-30T00:00:00Z".to_string(),
        }],
        companies: vec![CompanySignals {
            company: "Boba Labs".to_string(),
            funding_up: Some(false),
            layoffs: Some(3),
            rating: None,
            source: "cached research (agent run)".to_string(),
            as_of: "2026-09-30T00:00:00Z".to_string(),
        }],
    }
}

/// A whole input set, owned so a test can borrow it for one `score` call.
pub(crate) struct Fixture {
    pub features: PostingFeatures,
    pub posting: Offer,
    pub profile: ProfileSnapshot,
    pub prefs: Prefs,
    pub market: MarketData,
    pub semantic: Option<SemanticScore>,
    pub now: String,
}

impl Fixture {
    /// §9's example, with no embeddings (F3 missing until S6.9).
    pub fn section_9() -> Self {
        Self {
            features: section_9_features(),
            posting: section_9_posting(),
            profile: section_9_profile(),
            prefs: section_9_prefs(),
            market: section_9_market(),
            semantic: None,
            now: NOW.to_string(),
        }
    }

    pub fn inputs(&self) -> ScoreInputs<'_> {
        ScoreInputs {
            features: &self.features,
            posting: &self.posting,
            profile: &self.profile,
            prefs: &self.prefs,
            market: &self.market,
            semantic: self.semantic,
            now: &self.now,
        }
    }
}
