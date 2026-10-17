//! Scoring use case (S6.3, FR-55..57, FR-61, FR-14, AD-8).
//!
//! The core decides *what* a score is (`boba_domain::scoring::rank::score` is
//! pure, offline and deterministic); this module decides *when* it is taken and
//! *what is kept*:
//!
//! * a card is stored for the `(posting, profile version, prefs version, model
//!   version)` it was computed at (AC3) and is never rewritten — a change to any
//!   of the three writes a **new** card and leaves the old one readable;
//! * scoring the same inputs again hands back the stored card instead of writing
//!   a second one (FR-61), which is what makes "Re-score now" and the daily run
//!   cheap: the features are reused and no LLM is called (architecture §8);
//! * an Offer the model knows nothing about — no pay, no date, no requirements,
//!   no preferences pointing at it — produces **no card** rather than a card
//!   saying "0" (AC2: nothing is imputed).
//!
//! Everything the factors read arrives in the [`ScoreRequest`]: features (S6.2's
//! cache, keyed by the Profile version), the Offer's own normalised fields, the
//! Profile snapshot, the Preferences, market data, F3's optional similarity
//! (S6.9) and the Gates that ran first (S6.1). Time comes from the injected
//! [`ClockPort`] (AD-17), never from the machine.

use std::sync::Arc;

use boba_domain::posting::Offer;
use boba_domain::posting_features::PostingFeatures;
use boba_domain::scoring::factors::{
    MarketData, Prefs, ProfileSnapshot, ScoreInputs, SemanticScore,
};
use boba_domain::scoring::gates::GateReport;
use boba_domain::scoring::rank::{self, CardVersions, ScoreCardRow};

use crate::ports::clock::ClockPort;
use crate::ports::repo::scoring::{ScoringRepo, ScoringRepoError};

/// Everything one Offer is scored from, plus the versions the card is keyed by
/// (AC1, AC3). The caller reads it: this use case never fetches anything itself,
/// which is what keeps it testable with an in-memory repo.
#[derive(Debug, Clone)]
pub struct ScoreRequest {
    pub posting_id: String,
    /// The Profile version the snapshot was taken at (FR-13, AD-15).
    pub profile_version: u32,
    /// The Preferences version in force (FR-14, S3.7).
    pub prefs_version: u32,
    pub features: PostingFeatures,
    /// The Offer as S4.2 normalised it (FR-48): what Q3 and Q4 read.
    pub posting: Offer,
    pub profile: ProfileSnapshot,
    pub prefs: Prefs,
    pub market: MarketData,
    /// F3's similarity, `None` without the optional embeddings (S6.9).
    pub semantic: Option<SemanticScore>,
    /// The Gates that ran before scoring (FR-53): stored with the card.
    pub gates: GateReport,
}

impl ScoreRequest {
    /// The three versions this request would store a card under (AC3).
    pub fn versions(&self) -> CardVersions {
        CardVersions {
            profile_version: self.profile_version,
            prefs_version: self.prefs_version,
            model_version: rank::MODEL_VERSION,
        }
    }
}

/// A scoring failure the UI shows through a `message_key` (AD-3).
#[derive(Debug, Clone, thiserror::Error)]
pub enum ScoringError {
    /// The ScoreCard could not be read or written.
    #[error(transparent)]
    Repo(#[from] ScoringRepoError),
}

impl ScoringError {
    /// The i18n key the UI shows (AD-3: keys cross IPC, never raw text).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Repo(_) => "errors.scoring.store",
        }
    }
}

/// Score Offers and keep their cards.
///
/// The ports are erased (`Arc<dyn …>`) so the composition root mounts one handle
/// in Tauri state (ADR-012).
pub struct ScoringService {
    repo: Arc<dyn ScoringRepo>,
    clock: Arc<dyn ClockPort>,
}

impl ScoringService {
    /// The use case over a store and a clock.
    pub fn new(repo: Arc<dyn ScoringRepo>, clock: Arc<dyn ClockPort>) -> Self {
        Self { repo, clock }
    }

    /// Score one Offer and store its card (AC1..AC3).
    ///
    /// `Ok(None)` when there is nothing to score: the model would have to invent
    /// both axes, and a card is not the place to invent one.
    pub fn score(&self, request: ScoreRequest) -> Result<Option<ScoreCardRow>, ScoringError> {
        let versions = request.versions();
        // FR-61: this Offer has already been scored for these versions, so the
        // stored card *is* the answer — no second row, no second computation.
        if let Some(stored) = self.repo.card(&request.posting_id, versions)? {
            return Ok(Some(stored));
        }

        let now = self.clock.now_rfc3339();
        let inputs = ScoreInputs {
            features: &request.features,
            posting: &request.posting,
            profile: &request.profile,
            prefs: &request.prefs,
            market: &request.market,
            semantic: request.semantic,
            now: &now,
        };
        let Some(card) = rank::score(&inputs) else {
            return Ok(None);
        };

        let row = ScoreCardRow::new(
            rank::new_card_id(),
            request.posting_id,
            versions,
            card,
            request.gates,
            now,
        );
        self.repo.insert_card(&row)?;
        Ok(Some(row))
    }

    /// The newest card for an Offer, whatever versions it was computed for.
    pub fn latest(&self, posting_id: &str) -> Result<Option<ScoreCardRow>, ScoringError> {
        Ok(self.repo.latest_card(posting_id)?)
    }

    /// Every card an Offer has, oldest first (AC3: what a re-score leaves
    /// behind).
    pub fn cards_for(&self, posting_id: &str) -> Result<Vec<ScoreCardRow>, ScoringError> {
        Ok(self.repo.cards_for(posting_id)?)
    }

    /// Whether the Offer's newest card is the one these versions ask for: false
    /// means it is stale and the next run had better re-score it (FR-14).
    pub fn is_current(
        &self,
        posting_id: &str,
        versions: CardVersions,
    ) -> Result<bool, ScoringError> {
        Ok(self
            .repo
            .latest_card(posting_id)?
            .is_some_and(|row| row.versions() == versions))
    }
}

#[cfg(test)]
pub(crate) mod testing;

#[cfg(test)]
mod tests {
    use super::*;
    use boba_domain::posting::{ApplyHints, Money, PayPeriod, WorkMode};
    use boba_domain::posting_features::{Claim, FeatureKind, Seniority};
    use boba_domain::scoring::factors::{BenchmarkBasis, CompanySignals, SalaryBenchmark};
    use boba_domain::scoring::gates::{
        Cefr, Confidence, DeclaredLanguage, Evidence, GateKind, GateReason, GateResult, QuotedSpan,
        Verdict as GateVerdict,
    };
    use boba_domain::scoring::rank::{MODEL_VERSION, ScoreCard, Verdict as Band, score};

    use crate::scoring::testing::{FakeScoringRepo, FixedTestClock};

    /// The clock the use case runs at, so `created_at` is a fixed string.
    const NOW: &str = "2026-10-16T09:00:00Z";

    fn claim(kind: FeatureKind, value: &str) -> Claim {
        Claim::deterministic(
            kind,
            value,
            Confidence::High,
            Evidence::Field("description_text".to_string()),
        )
    }

    /// A whole, scorable request: §9's Offer with everything the factors read.
    fn request(prefs_version: u32) -> ScoreRequest {
        let mut features = PostingFeatures::new("pst_01", 3);
        features.extend([
            claim(FeatureKind::Skill, "dbt"),
            claim(FeatureKind::Skill, "Airflow"),
            claim(FeatureKind::Seniority, Seniority::Senior.as_str()),
            claim(FeatureKind::Language, "en"),
        ]);
        ScoreRequest {
            posting_id: "pst_01".to_string(),
            profile_version: 3,
            prefs_version,
            features,
            posting: Offer {
                id: "pst_01".to_string(),
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
                description_html: "<p>dbt, Airflow, Snowflake.</p>".to_string(),
                description_text: "dbt, Airflow and Snowflake. Permanent contract. Compensation: \
                    55.000 EUR per year."
                    .to_string(),
                requirements: vec!["dbt".to_string()],
                apply: ApplyHints {
                    url: "https://boards.example.com/boba/gh-42".to_string(),
                    email: None,
                },
            },
            profile: ProfileSnapshot {
                skills: vec!["Python".to_string(), "dbt".to_string()],
                seniority: Some(Seniority::Mid),
                years_experience: Some(5),
                education: Vec::new(),
                certifications: Vec::new(),
                languages: vec![DeclaredLanguage {
                    language: "en".to_string(),
                    level: Cefr::B2,
                }],
                locations: vec!["Madrid, ES".to_string()],
            },
            prefs: Prefs {
                roles: vec!["data engineer".to_string()],
                locations: vec!["madrid".to_string()],
                ..Prefs::default()
            },
            market: MarketData {
                salary: vec![SalaryBenchmark {
                    role: "data engineer".to_string(),
                    location: "madrid".to_string(),
                    currency: "EUR".to_string(),
                    period: PayPeriod::Year,
                    p50_minor: 5_200_000,
                    basis: BenchmarkBasis::User,
                    as_of: "2026-09-30T00:00:00Z".to_string(),
                }],
                companies: vec![CompanySignals {
                    company: "Boba Labs".to_string(),
                    funding_up: Some(false),
                    layoffs: Some(3),
                    rating: None,
                    source: "cached research".to_string(),
                    as_of: "2026-09-30T00:00:00Z".to_string(),
                }],
            },
            semantic: None,
            gates: GateReport {
                results: Vec::new(),
            },
        }
    }

    /// A request the model can say nothing about: no pay, no date, no text.
    fn blank_request() -> ScoreRequest {
        let mut request = request(2);
        request.posting.money = None;
        request.posting.posted_at = None;
        request.posting.work_mode = WorkMode::Unknown;
        request.posting.locations = Vec::new();
        request.posting.description_text = String::new();
        request.features = PostingFeatures::new("pst_01", 3);
        request.profile = ProfileSnapshot::empty();
        request.prefs = Prefs::default();
        request.market = MarketData::default();
        request
    }

    fn service(repo: Arc<FakeScoringRepo>) -> ScoringService {
        ScoringService::new(repo, Arc::new(FixedTestClock::at(NOW)))
    }

    /// What the pure function makes of the same request, so the use case can be
    /// shown to add storage and versions and nothing else.
    fn card_of(request: &ScoreRequest) -> ScoreCard {
        score(&ScoreInputs {
            features: &request.features,
            posting: &request.posting,
            profile: &request.profile,
            prefs: &request.prefs,
            market: &request.market,
            semantic: request.semantic,
            now: NOW,
        })
        .expect("a card")
    }

    #[test]
    fn a_card_is_stored_for_the_versions_it_was_requested_for() {
        let repo = Arc::new(FakeScoringRepo::default());
        let stored = service(repo.clone())
            .score(request(2))
            .expect("no storage failure")
            .expect("a card");

        assert!(stored.id.starts_with("sc_"), "{}", stored.id);
        assert_eq!(stored.posting_id, "pst_01");
        assert_eq!(
            stored.versions(),
            CardVersions {
                profile_version: 3,
                prefs_version: 2,
                model_version: MODEL_VERSION,
            }
        );
        assert_eq!(stored.created_at, NOW);
        assert_eq!(stored.card, card_of(&request(2)));
        assert_eq!(repo.cards_for("pst_01").expect("read").len(), 1);
    }

    // FR-61: the same inputs are the same card, and no second row is written.
    #[test]
    fn scoring_the_same_inputs_twice_stores_one_card() {
        let repo = Arc::new(FakeScoringRepo::default());
        let service = service(repo.clone());
        let first = service
            .score(request(2))
            .expect("no failure")
            .expect("a card");
        let second = service
            .score(request(2))
            .expect("no failure")
            .expect("a card");
        assert_eq!(first, second, "the stored card is handed back");
        assert_eq!(repo.cards_for("pst_01").expect("read").len(), 1);
    }

    // AC3: a Preferences change re-scores, and the earlier card stays readable
    // exactly as it was.
    #[test]
    fn a_preferences_change_adds_a_card_and_leaves_the_earlier_one_alone() {
        let repo = Arc::new(FakeScoringRepo::default());
        let service = service(repo.clone());
        let before = service
            .score(request(2))
            .expect("no failure")
            .expect("a card");
        let after = service
            .score(request(3))
            .expect("no failure")
            .expect("a card");

        assert_ne!(before.id, after.id);
        assert_eq!(after.versions().prefs_version, 3);
        let history = repo.cards_for("pst_01").expect("read");
        assert_eq!(history.len(), 2, "the earlier card is kept");
        assert_eq!(history[0], before, "and it is byte-for-byte what it was");
        assert_eq!(history[1], after);

        assert!(
            !service
                .is_current("pst_01", before.versions())
                .expect("no failure"),
            "the old versions are no longer current"
        );
        assert!(
            service
                .is_current("pst_01", after.versions())
                .expect("no failure")
        );
    }

    #[test]
    fn a_profile_change_also_re_scores() {
        let repo = Arc::new(FakeScoringRepo::default());
        let service = service(repo.clone());
        service.score(request(2)).expect("no failure");
        let mut changed = request(2);
        changed.profile_version = 4;
        let stored = service.score(changed).expect("no failure").expect("a card");
        assert_eq!(stored.versions().profile_version, 4);
        assert_eq!(repo.cards_for("pst_01").expect("read").len(), 2);
    }

    #[test]
    fn an_offer_the_model_knows_nothing_about_is_not_stored() {
        let repo = Arc::new(FakeScoringRepo::default());
        let stored = service(repo.clone())
            .score(blank_request())
            .expect("no failure");
        assert!(stored.is_none(), "nothing to score is nothing to store");
        assert!(repo.cards_for("pst_01").expect("read").is_empty());
    }

    #[test]
    fn the_latest_card_is_the_one_just_stored() {
        let repo = Arc::new(FakeScoringRepo::default());
        let service = service(repo.clone());
        assert!(service.latest("pst_01").expect("no failure").is_none());
        service.score(request(2)).expect("no failure");
        let latest = service
            .score(request(3))
            .expect("no failure")
            .expect("a card");
        assert_eq!(service.latest("pst_01").expect("no failure"), Some(latest));
        assert_eq!(service.cards_for("pst_01").expect("no failure").len(), 2);
    }

    #[test]
    fn the_gates_that_preceded_the_score_are_stored_with_it() {
        let repo = Arc::new(FakeScoringRepo::default());
        let mut request = request(2);
        request.gates = GateReport {
            results: vec![GateResult {
                gate: GateKind::Location,
                verdict: GateVerdict::Flag,
                reason: GateReason::Unverified,
                evidence: Some(Evidence::Quoted(QuotedSpan {
                    text: "Remote, Spain".to_string(),
                    start: 23,
                    end: 36,
                })),
                confidence: Confidence::High,
            }],
        };
        let stored = service(repo)
            .score(request)
            .expect("no failure")
            .expect("a card");
        assert_eq!(stored.gates.verdict(), GateVerdict::Flag);
        assert_eq!(stored.gates.results.len(), 1);
        assert_eq!(stored.card.verdict, Band::from_score(stored.card.rank));
    }

    #[test]
    fn a_storage_failure_is_reported_with_the_ui_key() {
        let repo = Arc::new(FakeScoringRepo::default());
        repo.break_storage();
        let error = service(repo).score(request(2)).expect_err("a failure");
        assert_eq!(error.message_key(), "errors.scoring.store");
        assert!(matches!(error, ScoringError::Repo(_)));
        assert!(!ScoringRepoError::new("boom").to_string().is_empty());
    }
}
