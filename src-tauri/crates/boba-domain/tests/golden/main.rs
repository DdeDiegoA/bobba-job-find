//! Golden ScoreCards (S6.3, AC4, FR-55..57, FR-61).
//!
//! The card the scoring model produces for `docs/research/offer-scoring.md` §9's
//! worked example, pinned as a regression: a mid-level data engineer in Madrid
//! looking at a "Senior Data Engineer" role, remote in Spain, at €55–70k, posted
//! twelve days ago, at a company with a down-round and layoff mentions behind it.
//!
//! Two things this file is deliberate about:
//!
//! * **the Rank the AC pins (75.2) is the published example's own arithmetic** —
//!   `0.65 · 81.3 + 0.35 · 63.9` on the axes §9 publishes. Those axes do *not*
//!   follow from §9's own factor column (which averages to 82.1 and 73.05); the
//!   research document is illustrative there and its factor column is off by a
//!   few points against its own weights. Both computations are pinned below, so
//!   the discrepancy is visible in the tests rather than hidden;
//! * the end-to-end card is what *this* model produces for §9's inputs, which is
//!   what a golden test can honestly freeze (FR-61: the same inputs always give
//!   the same card).
//!
//! It is an integration test on purpose: it uses the public API only, the way
//! S6.4 and S6.5 will.

use std::time::Instant;

use boba_domain::posting::{ApplyHints, Money, Offer, PayPeriod, WorkMode};
use boba_domain::posting_features::{Claim, FeatureKind, PostingFeatures, Seniority};
use boba_domain::scoring::factors::{
    BenchmarkBasis, CompanySignals, Factor, FactorWeights, MarketData, Prefs, ProfileSnapshot,
    SalaryBenchmark, ScoreInputs,
};
use boba_domain::scoring::gates::{
    Cefr, Confidence, DeclaredLanguage, Evidence, GateReport, QuotedSpan,
};
use boba_domain::scoring::rank::{
    CardVersions, ScoreCard, ScoreCardRow, Verdict, rank_from_axes, score,
};

/// The clock the golden card is scored against (AD-8: injected, never read).
const NOW: &str = "2026-10-16T00:00:00Z";

/// §9's posting text, verbatim in spirit: the stack, the contract, the pay.
const TEXT: &str = "Senior Data Engineer. Remote Spain. We build our analytics platform on dbt, \
    Airflow and Snowflake. You will own pipelines end to end and work closely with analysts. \
    Permanent contract, equity, health insurance. Compensation: 55.000 EUR per year.";

fn claim(kind: FeatureKind, value: &str) -> Claim {
    Claim::deterministic(
        kind,
        value,
        Confidence::High,
        Evidence::Field("description_text".to_string()),
    )
}

/// §9's posting as an Offer (S4.2's shape): the range keeps its lower bound.
fn offer() -> Offer {
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
        description_html: format!("<p>{TEXT}</p>"),
        description_text: TEXT.to_string(),
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

fn features() -> PostingFeatures {
    let mut features = PostingFeatures::new("pst_01SECTION9", 3);
    features.extend([
        claim(FeatureKind::Skill, "dbt"),
        claim(FeatureKind::Skill, "Airflow"),
        claim(FeatureKind::Skill, "Snowflake"),
        claim(FeatureKind::Seniority, Seniority::Senior.as_str()),
        claim(FeatureKind::Language, "en"),
    ]);
    features
}

fn profile() -> ProfileSnapshot {
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

fn prefs() -> Prefs {
    Prefs {
        roles: vec!["data engineer".to_string()],
        work_modes: vec![WorkMode::Remote, WorkMode::Hybrid],
        locations: vec!["madrid".to_string(), "spain".to_string()],
        working_languages: vec!["en".to_string(), "es".to_string()],
        ..Prefs::default()
    }
}

fn market() -> MarketData {
    MarketData {
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
            source: "cached research (agent run)".to_string(),
            as_of: "2026-09-30T00:00:00Z".to_string(),
        }],
    }
}

/// The §9 example scored by this model, with no embeddings (F3 drops out, S6.9
/// is what fills it).
fn golden_card() -> ScoreCard {
    let features = features();
    let posting = offer();
    let profile = profile();
    let prefs = prefs();
    let market = market();
    score(&ScoreInputs {
        features: &features,
        posting: &posting,
        profile: &profile,
        prefs: &prefs,
        market: &market,
        semantic: None,
        now: NOW,
    })
    .expect("§9's inputs are scorable")
}

#[test]
fn the_published_worked_example_ranks_75_2() {
    let ranked = rank_from_axes(Some(81.3), Some(63.9), &FactorWeights::DEFAULT).expect("a rank");
    assert!(
        (ranked.rank - 75.2).abs() <= 0.1,
        "the AC pins Rank 75.2 ±0.1, got {}",
        ranked.rank
    );
    assert_eq!(ranked.verdict, Verdict::Strong);
}

// The document's factor column, aggregated with its own weights, gives 82.1 and
// 73.05 — not the 81.3 and 63.9 it publishes next to it. This test keeps that
// visible: the AC's 75.2 comes from the published *axes*, and the end-to-end
// golden card below is what this model makes of the same inputs.
#[test]
fn the_published_factor_column_does_not_average_to_the_published_axes() {
    let fit_factors = [
        (88.0, 0.35),
        (58.0, 0.25),
        (82.0, 0.15),
        (95.0, 0.10),
        (100.0, 0.10),
        (100.0, 0.05),
    ];
    let fit: f32 = fit_factors
        .iter()
        .map(|(score, weight)| score * weight)
        .sum();
    assert!((fit - 82.1).abs() < 0.05, "{fit}");

    let quality_factors = [
        (92.0, 0.30),
        (35.0, 0.25),
        (78.0, 0.15),
        (85.0, 0.20),
        (80.0, 0.10),
    ];
    let quality: f32 = quality_factors
        .iter()
        .map(|(score, weight)| score * weight)
        .sum();
    assert!((quality - 73.05).abs() < 0.05, "{quality}");

    // And the rank those two would give, for the record.
    let ranked = rank_from_axes(Some(fit), Some(quality), &FactorWeights::DEFAULT).expect("a rank");
    assert!((ranked.rank - 78.9).abs() < 0.1, "{}", ranked.rank);
}

#[test]
fn the_worked_example_scores_end_to_end() {
    let card = golden_card();
    assert_eq!(card.model_version, 1);

    // Every factor reports, in F1..Q5 order — the two without an input included.
    let reported: Vec<Factor> = card.factors.iter().map(|factor| factor.factor).collect();
    assert_eq!(reported, Factor::ALL.to_vec());

    let find = |factor: Factor| -> Option<f32> {
        card.factors
            .iter()
            .find(|score| score.factor == factor)
            .expect("the factor is reported")
            .score
    };
    let near = |value: Option<f32>, expected: f32| {
        let value = value.expect("the factor has a score");
        assert!((value - expected).abs() < 0.01, "{value} is not {expected}");
    };
    near(find(Factor::Skills), 61.67);
    assert_eq!(find(Factor::Experience), Some(58.0), "§9's own F2");
    assert_eq!(find(Factor::Semantic), None, "no embeddings in this build");
    assert_eq!(find(Factor::Location), Some(95.0), "§9's own F4");
    assert_eq!(
        find(Factor::Education),
        None,
        "the posting asks for no study"
    );
    assert_eq!(find(Factor::Language), Some(100.0), "§9's own F6");

    let fit = card.fit.expect("a Fit axis");
    assert!((fit - 67.44).abs() < 0.05, "{fit}");
    let quality = card.quality.expect("a Quality axis");
    assert!((quality - 66.56).abs() < 0.05, "{quality}");
    assert!((card.rank - 67.14).abs() < 0.05, "{}", card.rank);
    assert_eq!(card.verdict, Verdict::Good);
    assert_eq!(card.confidence, Confidence::Medium);
    assert!(!card.is_lead(), "nine of eleven factors is not a lead");

    // The Company factor keeps §9's story: a down-round with layoffs is the
    // weakness that drags Quality down, and says so.
    let company = card.factor(Factor::Company).expect("Q2 is reported");
    near(company.score, 22.33);
    assert_eq!(company.matched.len(), 2);
}

// FR-61: the same Offer, Profile version, Preferences and inputs give the same
// card, whatever the machine.
#[test]
fn the_same_inputs_give_the_same_card() {
    assert_eq!(golden_card(), golden_card());
}

#[test]
fn a_stored_card_is_immutable_by_construction() {
    let card = golden_card();
    let row = ScoreCardRow::new(
        "sc_01GOLDEN",
        "pst_01SECTION9",
        CardVersions {
            profile_version: 3,
            prefs_version: 2,
            model_version: card.model_version,
        },
        card.clone(),
        GateReport {
            results: Vec::new(),
        },
        NOW,
    );
    // A change to any of the three versions is what tells the app to re-score
    // (FR-14); the row itself is never rewritten (AC3).
    assert_eq!(row.versions().profile_version, 3);
    assert_ne!(
        row.versions(),
        CardVersions {
            profile_version: 4,
            prefs_version: 2,
            model_version: card.model_version,
        }
    );
    assert_eq!(row.card, card);
}

// AC4: scoring is ≤ 50 ms per Offer. A debug build and 500 Offers leave plenty
// of headroom above the microseconds this takes, so the assertion is a real
// ceiling rather than a flake.
#[test]
fn a_score_takes_under_fifty_milliseconds() {
    let features = features();
    let posting = offer();
    let profile = profile();
    let prefs = prefs();
    let market = market();
    let inputs = ScoreInputs {
        features: &features,
        posting: &posting,
        profile: &profile,
        prefs: &prefs,
        market: &market,
        semantic: None,
        now: NOW,
    };

    // One warm-up pass, so the measurement is of scoring and not of first-touch
    // page faults.
    let _ = score(&inputs);

    let runs = 500;
    let started = Instant::now();
    for _ in 0..runs {
        let card = score(&inputs).expect("a card");
        assert_eq!(card.verdict, Verdict::Good);
    }
    let per_offer = started.elapsed().as_secs_f64() / f64::from(runs);
    assert!(
        per_offer <= 0.050,
        "scoring takes {:.3} ms per Offer, the budget is 50 ms",
        per_offer * 1000.0
    );
}

// A quote the factor reports points back into the posting's text, which is what
// the Offer detail renders as the evidence (FR-53, FR-65).
#[test]
fn a_quoted_span_points_back_into_the_posting_text() {
    let start = TEXT
        .find("Remote Spain")
        .expect("the phrase is in the posting");
    let quoted = Evidence::Quoted(QuotedSpan {
        text: "Remote Spain".to_string(),
        start,
        end: start + "Remote Spain".len(),
    });
    let Evidence::Quoted(span) = &quoted else {
        panic!("expected a quote");
    };
    assert_eq!(&TEXT[span.start..span.end], "Remote Spain");
}
