//! Q2 company signals (FR-56, `offer-scoring.md` §4): how healthy the employer
//! looks, from the research the app already has.
//!
//! FR-56's MVP note is the boundary of this factor: it reads **only** what is in
//! the cache the user's own runs and the user produced (or the bundled
//! benchmark), never a live fetch during scoring — scoring is offline and
//! deterministic (AD-8). Each sub-signal is scored on its own and the present
//! ones share the weight of the ones the cache does not have, because
//! `offer-scoring.md` §4 is explicit that a missing signal must never be imputed
//! as a neutral high.
//!
//! The confidence follows the same rule: three signals is a confident answer,
//! two is a reasonable one, one is a hint, and none is no answer at all.

use super::*;

/// The sub-signals a cached company record can carry, and their weights inside
/// Q2 (`offer-scoring.md` §4: funding and layoffs carry most of the signal).
const FUNDING_WEIGHT: f32 = 0.40;
const LAYOFFS_WEIGHT: f32 = 0.35;
const RATING_WEIGHT: f32 = 0.25;

/// A company with money behind it; a down-round is the drag §9's example shows.
const FUNDING_HEALTHY: f32 = 90.0;
const FUNDING_DOWN_ROUND: f32 = 20.0;

/// Layoff mentions, by how many the research found.
const LAYOFFS_NONE: f32 = 100.0;
const LAYOFFS_FEW: f32 = 60.0;
const LAYOFFS_MANY: f32 = 25.0;

/// One sub-signal: what it is worth and how the factor names it.
struct Signal {
    score: f32,
    weight: f32,
    shown: String,
}

fn signals(signals: &CompanySignals) -> Vec<Signal> {
    let mut found = Vec::new();
    if let Some(up) = signals.funding_up {
        found.push(Signal {
            score: if up {
                FUNDING_HEALTHY
            } else {
                FUNDING_DOWN_ROUND
            },
            weight: FUNDING_WEIGHT,
            shown: if up {
                "funding: growing".to_string()
            } else {
                "funding: down-round".to_string()
            },
        });
    }
    if let Some(layoffs) = signals.layoffs {
        found.push(Signal {
            score: match layoffs {
                0 => LAYOFFS_NONE,
                1..=2 => LAYOFFS_FEW,
                _ => LAYOFFS_MANY,
            },
            weight: LAYOFFS_WEIGHT,
            shown: format!("layoffs: {layoffs} mention(s)"),
        });
    }
    if let Some(rating) = signals.rating {
        found.push(Signal {
            score: 100.0 * (rating / 5.0),
            weight: RATING_WEIGHT,
            shown: format!("rating: {rating}/5"),
        });
    }
    found
}

pub(super) fn score(inputs: &ScoreInputs<'_>, weight: f32) -> FactorScore {
    let Some(cached) = inputs.market.company(&inputs.posting.company) else {
        return FactorScore::missing_input(Factor::Company, weight, "no cached company research");
    };
    let found = signals(cached);
    let Some((score, _)) = weighted_mean(
        &found
            .iter()
            .map(|signal| (signal.score, signal.weight))
            .collect::<Vec<_>>(),
    ) else {
        return FactorScore::missing_input(
            Factor::Company,
            weight,
            "the cached research says nothing about funding, layoffs or reviews",
        );
    };

    let confidence = match found.len() {
        0 | 1 => Confidence::Low,
        2 => Confidence::Medium,
        _ => Confidence::High,
    };
    FactorScore::scored(
        Factor::Company,
        score,
        weight,
        confidence,
        vec![
            Evidence::Field("company_signals".to_string()),
            Evidence::Field(format!("company_research:{}", cached.source)),
        ],
    )
    .with_matched(found.into_iter().map(|signal| signal.shown).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoring::factors::testing::*;

    fn q2(fixture: &Fixture) -> FactorScore {
        score(&fixture.inputs(), fixture.prefs.weights.of(Factor::Company))
    }

    // §9: a recent down-round and three layoff mentions, no reviews cached —
    // the company risk that drags the example's Quality down.
    #[test]
    fn the_worked_examples_company_is_dragged_by_its_two_signals() {
        let factor = q2(&Fixture::section_9());
        let score = factor.score.expect("a score");
        assert!(score > 15.0 && score < 30.0, "{score}");
        assert_eq!(factor.confidence, Confidence::Medium);
        assert_eq!(factor.matched.len(), 2);
        assert_eq!(factor.missing, Vec::<String>::new());
    }

    #[test]
    fn a_healthy_company_with_every_signal_scores_high() {
        let mut fixture = Fixture::section_9();
        let signals = &mut fixture.market.companies[0];
        signals.funding_up = Some(true);
        signals.layoffs = Some(0);
        signals.rating = Some(4.5);
        let factor = q2(&fixture);
        assert_eq!(factor.score, Some(93.5));
        assert_eq!(factor.confidence, Confidence::High);
        assert_eq!(factor.matched.len(), 3);
    }

    #[test]
    fn one_signal_alone_is_a_low_confidence_answer() {
        let mut fixture = Fixture::section_9();
        let signals = &mut fixture.market.companies[0];
        signals.funding_up = None;
        signals.layoffs = None;
        signals.rating = Some(4.0);
        let factor = q2(&fixture);
        assert_eq!(factor.score, Some(80.0));
        assert_eq!(factor.confidence, Confidence::Low);
    }

    // AC2: no cached research is a missing input, never a neutral score.
    #[test]
    fn a_company_with_no_cached_research_leaves_the_factor_missing() {
        let mut fixture = Fixture::section_9();
        fixture.market.companies = Vec::new();
        let factor = q2(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(factor.effective_weight, 0.0);
        assert_eq!(
            factor.missing,
            vec!["no cached company research".to_string()]
        );
    }

    #[test]
    fn cached_research_that_says_nothing_is_still_a_missing_input() {
        let mut fixture = Fixture::section_9();
        let signals = &mut fixture.market.companies[0];
        signals.funding_up = None;
        signals.layoffs = None;
        signals.rating = None;
        let factor = q2(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(
            factor.missing,
            vec!["the cached research says nothing about funding, layoffs or reviews".to_string()]
        );
    }

    #[test]
    fn the_company_name_is_matched_without_case() {
        let mut fixture = Fixture::section_9();
        fixture.posting.company = "BOBA LABS".to_string();
        let score = q2(&fixture).score.expect("a score");
        assert!((score - 22.33).abs() < 0.01, "{score}");
    }

    #[test]
    fn the_cache_age_and_source_travel_as_evidence() {
        let factor = q2(&Fixture::section_9());
        assert!(!factor.evidence.is_empty());
    }
}
