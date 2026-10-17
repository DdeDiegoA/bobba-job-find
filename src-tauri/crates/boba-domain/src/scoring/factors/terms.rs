//! Q3 role & terms quality (FR-56, `offer-scoring.md` §4): the terms of the
//! deal, as the posting itself states them.
//!
//! Four signals, each read from the Offer's own fields or its own words — no
//! LLM, no cache, no inference:
//!
//! * **remote quality** — fully remote against "hybrid" bait (`work_mode`);
//! * **pay transparency** — a posted salary, which is also one of §5's
//!   anti-ghost signals;
//! * **benefits** — the benefit words the posting mentions (equity, health,
//!   PTO, parental, pension…), counted, never assumed;
//! * **contract type** — permanent wording against contractor wording.
//!
//! The user's **deal-breaker keywords** (FR-7) override all of it: if the
//! posting says a word the user declared a deal-breaker, the factor is 0 and
//! names the word with the sentence it was read from. That is the one place a
//! preference beats a score, and it is deliberate.
//!
//! A signal the posting says nothing about is absent, not half-credit: with
//! nothing at all to read the factor is a missing input (AC2).

use crate::posting::WorkMode;
use crate::scoring::gates::scan;

use super::*;

/// The weight each signal carries inside Q3.
const REMOTE_WEIGHT: f32 = 0.35;
const TRANSPARENCY_WEIGHT: f32 = 0.25;
const BENEFITS_WEIGHT: f32 = 0.25;
const CONTRACT_WEIGHT: f32 = 0.15;

/// How good each work mode is for a user whose Preferences already accepted it
/// (F4 had its say; this is the quality of the arrangement itself).
const REMOTE_QUALITY: f32 = 100.0;
const HYBRID_QUALITY: f32 = 60.0;
const ONSITE_QUALITY: f32 = 40.0;

/// Pay the posting chose to show.
const PAY_SHOWN: f32 = 100.0;

/// Permanent against contractor.
const PERMANENT: f32 = 100.0;
const CONTRACTOR: f32 = 50.0;

/// Benefit words found: one, two, or three and more.
const ONE_BENEFIT: f32 = 60.0;
const TWO_BENEFITS: f32 = 80.0;
const MANY_BENEFITS: f32 = 100.0;

/// Benefit words, as postings write them (EN and ES).
const BENEFITS: &[&str] = &[
    "equity",
    "stock options",
    "rsu",
    "health insurance",
    "dental",
    "vision",
    "parental leave",
    "paid time off",
    "pto",
    "vacation",
    "pension",
    "401k",
    "seguro médico",
    "seguro medico",
    "acciones",
    "vacaciones",
];

/// Wording that says the job is not a permanent one.
const CONTRACTOR_WORDS: &[&str] = &[
    "contractor",
    "freelance",
    "b2b",
    "fixed term",
    "fixed-term",
    "temporary contract",
    "contrato temporal",
    "autónomo",
    "autonomo",
];

/// Wording that says it is.
const PERMANENT_WORDS: &[&str] = &["permanent", "indefinido", "full-time permanent", "cdi"];

/// One signal: what it is worth, what it weighs and how the factor names it.
struct Signal {
    score: f32,
    weight: f32,
    shown: String,
}

pub(super) fn score(inputs: &ScoreInputs<'_>, weight: f32) -> FactorScore {
    let text = inputs.posting.description_text.as_str();

    // FR-7's deal breakers first: one hit and the terms are not the point any more.
    for keyword in &inputs.prefs.deal_breaker_keywords {
        let hits = scan::matches(text, &keyword.to_lowercase());
        if let Some(start) = hits.first() {
            let span = scan::sentence(text, *start, start + keyword.len());
            return FactorScore::scored(
                Factor::Terms,
                0.0,
                weight,
                Confidence::High,
                vec![Evidence::Quoted(span)],
            )
            .with_missing(vec![keyword.trim().to_lowercase()]);
        }
    }

    let mut found = Vec::new();
    if let Some(quality) = match inputs.posting.work_mode {
        WorkMode::Remote => Some(REMOTE_QUALITY),
        WorkMode::Hybrid => Some(HYBRID_QUALITY),
        WorkMode::Onsite => Some(ONSITE_QUALITY),
        WorkMode::Unknown => None,
    } {
        found.push(Signal {
            score: quality,
            weight: REMOTE_WEIGHT,
            shown: inputs.posting.work_mode.as_str().to_string(),
        });
    }
    // Pay shown is the positive signal (FR-56: salary transparency). Pay
    // *hidden* contributes nothing here: what a posting withholds is a
    // legitimacy signal, and Q4 is where it is counted — scoring it twice would
    // also let an otherwise blank posting earn a terms score out of nothing
    // (AC2).
    if inputs.posting.money.is_some() {
        found.push(Signal {
            score: PAY_SHOWN,
            weight: TRANSPARENCY_WEIGHT,
            shown: "pay shown".to_string(),
        });
    }

    let folded = text.to_lowercase();
    let benefits: Vec<&str> = BENEFITS
        .iter()
        .filter(|benefit| !scan::matches(&folded, benefit).is_empty())
        .copied()
        .collect();
    if !benefits.is_empty() {
        // A stepped scale, not a linear one: naming one benefit already says the
        // employer writes them down, and a third adds little.
        let score = match benefits.len() {
            1 => ONE_BENEFIT,
            2 => TWO_BENEFITS,
            _ => MANY_BENEFITS,
        };
        found.push(Signal {
            score,
            weight: BENEFITS_WEIGHT,
            shown: benefits.join(", "),
        });
    }

    let permanent = PERMANENT_WORDS
        .iter()
        .any(|word| !scan::matches(&folded, word).is_empty());
    let contractor = CONTRACTOR_WORDS
        .iter()
        .any(|word| !scan::matches(&folded, word).is_empty());
    if permanent || contractor {
        let (score, shown) = if contractor {
            (CONTRACTOR, "contractor".to_string())
        } else {
            (PERMANENT, "permanent".to_string())
        };
        found.push(Signal {
            score,
            weight: CONTRACT_WEIGHT,
            shown,
        });
    }

    let Some((score, _)) = weighted_mean(
        &found
            .iter()
            .map(|signal| (signal.score, signal.weight))
            .collect::<Vec<_>>(),
    ) else {
        return FactorScore::missing_input(
            Factor::Terms,
            weight,
            "the posting says nothing about its terms",
        );
    };

    let confidence = match found.len() {
        0 | 1 => Confidence::Low,
        2 => Confidence::Medium,
        _ => Confidence::High,
    };
    FactorScore::scored(
        Factor::Terms,
        score,
        weight,
        confidence,
        vec![Evidence::Field("description_text".to_string())],
    )
    .with_matched(found.into_iter().map(|signal| signal.shown).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting::WorkMode;
    use crate::scoring::factors::testing::*;

    fn q3(fixture: &Fixture) -> FactorScore {
        score(&fixture.inputs(), fixture.prefs.weights.of(Factor::Terms))
    }

    // §9: permanent, equity, health insurance, remote, with the pay posted.
    #[test]
    fn the_worked_examples_terms_score_at_the_top() {
        let factor = q3(&Fixture::section_9());
        assert_eq!(factor.score, Some(95.0));
        assert_eq!(factor.confidence, Confidence::High);
        assert_eq!(factor.matched.len(), 4);
    }

    #[test]
    fn a_deal_breaker_zeroes_the_factor_and_names_the_word() {
        let mut fixture = Fixture::section_9();
        fixture.posting.description_text = format!("{} Frequent on-call rotation.", SECTION_9_TEXT);
        fixture.prefs.deal_breaker_keywords = vec!["on-call".to_string()];
        let factor = q3(&fixture);
        assert_eq!(factor.score, Some(0.0));
        assert_eq!(factor.missing, vec!["on-call".to_string()]);
        assert_eq!(factor.confidence, Confidence::High);
    }

    // AC2: a posting that states no terms gives the factor nothing to read. Pay
    // withheld is counted by Q4, not turned into a terms score here.
    #[test]
    fn a_posting_that_terms_nothing_leaves_the_factor_missing() {
        let mut fixture = Fixture::section_9();
        fixture.posting.work_mode = WorkMode::Unknown;
        fixture.posting.money = None;
        fixture.posting.description_text = "We are hiring engineers.".to_string();
        let factor = q3(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(factor.effective_weight, 0.0);
        assert_eq!(
            factor.missing,
            vec!["the posting says nothing about its terms".to_string()]
        );
    }

    #[test]
    fn pay_shown_alone_is_a_thin_but_real_terms_score() {
        let mut fixture = Fixture::section_9();
        fixture.posting.work_mode = WorkMode::Unknown;
        fixture.posting.description_text = String::new();
        let factor = q3(&fixture);
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.confidence, Confidence::Low);
        assert_eq!(factor.matched, vec!["pay shown".to_string()]);
    }

    #[test]
    fn contractor_wording_scores_below_a_permanent_contract() {
        let mut fixture = Fixture::section_9();
        fixture.posting.description_text =
            "Contractor position, no benefits. Remote within Spain.".to_string();
        fixture.posting.money = None;
        let factor = q3(&fixture);
        // Remote (100 · 0.35) and a contractor contract (50 · 0.15), with no
        // pay shown and no benefit words in the posting.
        assert_eq!(factor.score, Some(85.0));
        assert_eq!(factor.confidence, Confidence::Medium);
    }

    #[test]
    fn pay_the_posting_does_not_show_costs_the_transparency_signal() {
        let mut fixture = Fixture::section_9();
        let with_pay = q3(&fixture).score.expect("a score");
        fixture.posting.money = None;
        let without_pay = q3(&fixture).score.expect("a score");
        assert_eq!(with_pay, 95.0);
        // Remote (100 · 0.35), benefits (80 · 0.25), permanent (100 · 0.15).
        assert!((without_pay - 93.33).abs() < 0.01, "{without_pay}");
        assert!(without_pay < with_pay);
    }

    #[test]
    fn every_benefit_word_the_posting_shows_is_kept() {
        let factor = q3(&Fixture::section_9());
        let shown = factor.matched.join(" ");
        assert!(shown.contains("equity"), "{shown}");
        assert!(shown.contains("health insurance"), "{shown}");
    }

    #[test]
    fn a_switch_to_hybrid_lowers_the_remote_quality_signal() {
        let mut fixture = Fixture::section_9();
        fixture.posting.work_mode = WorkMode::Hybrid;
        let score = q3(&fixture).score.expect("a score");
        assert!(score < 95.0 && score > 80.0, "{score}");
    }
}
