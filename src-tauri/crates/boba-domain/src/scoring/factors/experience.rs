//! F2 experience / seniority (FR-55, `offer-scoring.md` §3): does the level the
//! posting asks for match the level the Profile declares?
//!
//! The posting's level comes from S6.2's seniority claim; the candidate's from
//! the Profile's own declared level, or — only when the Profile names none — from
//! the years it states, mapped onto the same scale. Both directions of a gap
//! cost: a role above the candidate is a stretch (the bigger penalty, and the
//! one §9's worked example scores 58), a role below is a retention risk the
//! research document flags (`offer-scoring.md` §3: "over-qualification flagged
//! as retention risk").
//!
//! Nothing is guessed: a posting that states no level and a Profile that states
//! neither a level nor years both leave the factor with no score, and the axis
//! renormalises over the factors that do have one (AC2).

use super::*;

/// The penalty for each level the posting asks *above* the candidate.
const UNDER_QUALIFIED_STEP: f32 = 42.0;
/// The penalty for each level the role sits *below* the candidate.
const OVER_QUALIFIED_STEP: f32 = 18.0;

/// Where the candidate sits: the level they declared, or the band their stated
/// years fall in.
fn candidate_level(inputs: &ScoreInputs<'_>) -> Option<(Seniority, Confidence)> {
    if let Some(level) = inputs.profile.seniority {
        return Some((level, Confidence::High));
    }
    let years = inputs.profile.years_experience?;
    let level = match years {
        0 => Seniority::Intern,
        1..=2 => Seniority::Junior,
        3..=5 => Seniority::Mid,
        6..=9 => Seniority::Senior,
        10..=14 => Seniority::Lead,
        _ => Seniority::Principal,
    };
    // A band read off years is coarser than a level the user chose.
    Some((level, Confidence::Medium))
}

/// The posting's stated level, with the claim behind it.
fn asked_level(inputs: &ScoreInputs<'_>) -> Option<(Seniority, Evidence)> {
    let claim = inputs
        .features
        .first(crate::posting_features::FeatureKind::Seniority)?;
    let level = Seniority::parse(&claim.value)?;
    Some((level, claim.evidence.clone()))
}

fn rank(level: Seniority) -> i32 {
    Seniority::ALL
        .iter()
        .position(|candidate| *candidate == level)
        .map_or(0, |index| index as i32)
}

pub(super) fn score(inputs: &ScoreInputs<'_>, weight: f32) -> FactorScore {
    let Some((asked, evidence)) = asked_level(inputs) else {
        return FactorScore::missing_input(
            Factor::Experience,
            weight,
            "the posting states no level",
        );
    };
    let Some((mine, confidence)) = candidate_level(inputs) else {
        return FactorScore::missing_input(
            Factor::Experience,
            weight,
            "the profile states no level or years of experience",
        );
    };

    let distance = rank(asked) - rank(mine);
    let score = if distance > 0 {
        100.0 - UNDER_QUALIFIED_STEP * distance as f32
    } else {
        // At the level, or above it: over-qualification is a retention risk the
        // score reflects, not a requirement the candidate is missing.
        100.0 - OVER_QUALIFIED_STEP * (-distance) as f32
    };

    let mut factor = FactorScore::scored(
        Factor::Experience,
        score,
        weight,
        confidence,
        vec![evidence],
    );
    if distance <= 0 {
        factor = factor.with_matched(vec![asked.as_str().to_string()]);
    } else {
        factor = factor.with_missing(vec![asked.as_str().to_string()]);
    }
    factor
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting_features::PostingFeatures;
    use crate::scoring::factors::testing::*;

    fn f2(fixture: &Fixture) -> FactorScore {
        score(
            &fixture.inputs(),
            fixture.prefs.weights.of(Factor::Experience),
        )
    }

    /// The posting asks for `level` — and nothing else about the role.
    fn asking(level: Seniority) -> Fixture {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.features.extend([seniority(level)]);
        fixture
    }

    // §9: "Senior Data Engineer" against a declared mid — a warn, not a veto,
    // and the factor the example scores 58.
    #[test]
    fn the_worked_examples_seniority_gap_scores_fifty_eight() {
        let factor = f2(&asking(Seniority::Senior));
        assert_eq!(factor.score, Some(58.0));
        assert_eq!(factor.matched, Vec::<String>::new());
        assert_eq!(factor.missing, vec!["senior".to_string()]);
        assert_eq!(factor.confidence, Confidence::High);
    }

    #[test]
    fn a_level_the_profile_has_is_a_full_score() {
        let mut fixture = asking(Seniority::Senior);
        fixture.profile.seniority = Some(Seniority::Senior);
        let factor = f2(&fixture);
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.matched, vec!["senior".to_string()]);
        assert_eq!(factor.missing, Vec::<String>::new());
    }

    #[test]
    fn two_levels_short_of_the_role_hurt_more_than_one() {
        let mut fixture = asking(Seniority::Senior);
        fixture.profile.seniority = Some(Seniority::Junior);
        assert_eq!(f2(&fixture).score, Some(16.0));
    }

    #[test]
    fn a_gap_no_score_can_recover_from_bottoms_out_at_zero() {
        let mut fixture = asking(Seniority::Principal);
        fixture.profile.seniority = Some(Seniority::Intern);
        assert_eq!(f2(&fixture).score, Some(0.0));
    }

    #[test]
    fn an_over_qualified_candidate_is_flagged_as_a_retention_risk_in_the_score() {
        let mut fixture = asking(Seniority::Junior);
        fixture.profile.seniority = Some(Seniority::Senior);
        let factor = f2(&fixture);
        assert_eq!(factor.score, Some(64.0));
        assert_eq!(factor.matched, vec!["junior".to_string()]);
        assert_eq!(factor.missing, Vec::<String>::new());
    }

    #[test]
    fn a_posting_that_states_no_level_leaves_the_factor_missing() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        let factor = f2(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(
            factor.missing,
            vec!["the posting states no level".to_string()]
        );
    }

    #[test]
    fn a_profile_that_states_neither_level_nor_years_leaves_the_factor_missing() {
        let mut fixture = asking(Seniority::Senior);
        fixture.profile.seniority = None;
        fixture.profile.years_experience = None;
        let factor = f2(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(
            factor.missing,
            vec!["the profile states no level or years of experience".to_string()]
        );
    }

    #[test]
    fn stated_years_stand_in_for_a_level_the_profile_never_named() {
        let mut fixture = asking(Seniority::Senior);
        fixture.profile.seniority = None;
        fixture.profile.years_experience = Some(12);
        let factor = f2(&fixture);
        // Twelve years reads as lead: one level above the role (a mild
        // over-qualification, never a fabricated senior level).
        assert_eq!(factor.score, Some(82.0));
        assert_eq!(factor.matched, vec!["senior".to_string()]);
        assert_eq!(factor.confidence, Confidence::Medium);
    }

    #[test]
    fn the_postings_level_word_is_the_evidence() {
        let factor = f2(&asking(Seniority::Senior));
        assert_eq!(factor.evidence.len(), 1);
    }
}
