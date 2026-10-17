//! Q5 career alignment (FR-56, `offer-scoring.md` §4): does this role move the
//! user toward what they said they want?
//!
//! The research document calls Q5 "mostly LLM-assisted with evidence from the
//! posting". This is the deterministic half of it, and the Explaination (FR-59)
//! narrates it — the score itself is core-computed, like every other number:
//!
//! * **the declared roles** (FR-7: "roles and seniority" are the first thing the
//!   Preferences step asks for) against the posting's title, by word;
//! * **the declared seniority band**, against the level the posting states;
//! * **the excluded companies** (FR-7), which override both: a company the user
//!   never wants to see again cannot be a good career move.
//!
//! A user who has declared neither roles nor a band has stated no goal to align
//! with, and the factor says so instead of inventing one (AC2).

use std::collections::BTreeSet;

use crate::posting_features::{FeatureKind, Seniority};

use super::*;

/// The declared roles matter most; the band is the goal's shape.
const ROLE_WEIGHT: f32 = 0.6;
const BAND_WEIGHT: f32 = 0.4;

/// A role whose words are all in the title, and one word short of it.
const ONE_WORD_SHORT: f32 = 60.0;
/// Further away than that.
const FAR_SHORT: f32 = 25.0;

/// The words of a phrase, folded, without the noise words a title is full of.
fn words(phrase: &str) -> BTreeSet<String> {
    phrase
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '+')
        .filter(|word| word.len() > 1)
        .map(str::to_string)
        .collect()
}

/// How much of a declared role the title names, 0–1. A role with no words at
/// all cannot be compared and reports 0 (`roles: vec!["".into()]` is a bad
/// document, not alignment).
fn role_coverage(role: &str, title: &BTreeSet<String>) -> f32 {
    let asked = words(role);
    if asked.is_empty() {
        return 0.0;
    }
    let found = asked.iter().filter(|word| title.contains(*word)).count();
    found as f32 / asked.len() as f32
}

/// Levels between the posting and the band, 0 inside it.
fn band_distance(level: Seniority, min: Option<Seniority>, max: Option<Seniority>) -> i32 {
    let rank = |value: Seniority| {
        Seniority::ALL
            .iter()
            .position(|candidate| *candidate == value)
            .map_or(0, |index| index as i32)
    };
    let level = rank(level);
    if min.is_some_and(|min| level < rank(min)) {
        rank(min.expect("min is some")) - level
    } else if max.is_some_and(|max| level > rank(max)) {
        level - rank(max.expect("max is some"))
    } else {
        0
    }
}

pub(super) fn score(inputs: &ScoreInputs<'_>, weight: f32) -> FactorScore {
    // FR-7: excluded companies first — nothing else about the role matters.
    let excluded = inputs.prefs.excluded_companies.iter().find(|company| {
        company.trim().to_lowercase() == inputs.posting.company.trim().to_lowercase()
    });
    if let Some(company) = excluded {
        return FactorScore::scored(
            Factor::Alignment,
            0.0,
            weight,
            Confidence::High,
            vec![Evidence::Field("excluded_companies".to_string())],
        )
        .with_missing(vec![company.trim().to_string()]);
    }

    let mut found: Vec<(f32, f32, String, bool)> = Vec::new();

    if !inputs.prefs.roles.is_empty() {
        let title = words(&inputs.posting.title);
        let best = inputs
            .prefs
            .roles
            .iter()
            .map(|role| (role_coverage(role, &title), role.trim().to_string()))
            .max_by(|(left, _), (right, _)| {
                left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
            });
        if let Some((coverage, role)) = best {
            found.push((100.0 * coverage, ROLE_WEIGHT, role, coverage > 0.0));
        }
    }

    if inputs.prefs.seniority_min.is_some() || inputs.prefs.seniority_max.is_some() {
        let level = inputs
            .features
            .first(FeatureKind::Seniority)
            .and_then(|claim| Seniority::parse(&claim.value));
        if let Some(level) = level {
            let distance = band_distance(
                level,
                inputs.prefs.seniority_min,
                inputs.prefs.seniority_max,
            );
            let score = match distance {
                0 => 100.0,
                1 => ONE_WORD_SHORT,
                _ => FAR_SHORT,
            };
            found.push((
                score,
                BAND_WEIGHT,
                level.as_str().to_string(),
                distance == 0,
            ));
        }
    }

    let Some((score, _)) = weighted_mean(
        &found
            .iter()
            .map(|(score, weight, _, _)| (*score, *weight))
            .collect::<Vec<_>>(),
    ) else {
        return FactorScore::missing_input(
            Factor::Alignment,
            weight,
            "the preferences declare no target role or level",
        );
    };

    let confidence = match found.len() {
        0 | 1 => Confidence::Medium,
        _ => Confidence::High,
    };
    FactorScore::scored(
        Factor::Alignment,
        score,
        weight,
        confidence,
        vec![Evidence::Field("title".to_string())],
    )
    .with_matched(
        found
            .iter()
            .filter(|(_, _, _, aligned)| *aligned)
            .map(|(_, _, shown, _)| shown.clone())
            .collect(),
    )
    .with_missing(
        found
            .iter()
            .filter(|(_, _, _, aligned)| !*aligned)
            .map(|(_, _, shown, _)| shown.clone())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting_features::PostingFeatures;
    use crate::scoring::factors::testing::*;

    fn q5(fixture: &Fixture) -> FactorScore {
        score(
            &fixture.inputs(),
            fixture.prefs.weights.of(Factor::Alignment),
        )
    }

    // §9: the user is looking for data engineer roles, and the posting is one.
    #[test]
    fn the_worked_example_matches_the_declared_goal() {
        let factor = q5(&Fixture::section_9());
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.confidence, Confidence::Medium);
        assert_eq!(factor.matched, vec!["data engineer".to_string()]);
        assert_eq!(factor.missing, Vec::<String>::new());
    }

    // FR-7: a company the user excluded is not an alignment question any more.
    #[test]
    fn an_excluded_company_zeroes_the_factor() {
        let mut fixture = Fixture::section_9();
        fixture.prefs.excluded_companies = vec!["Boba Labs".to_string()];
        let factor = q5(&fixture);
        assert_eq!(factor.score, Some(0.0));
        assert_eq!(factor.missing, vec!["Boba Labs".to_string()]);
        assert_eq!(factor.confidence, Confidence::High);
    }

    #[test]
    fn a_role_the_preferences_never_named_is_no_alignment_at_all() {
        let mut fixture = Fixture::section_9();
        fixture.prefs.roles = vec!["product designer".to_string()];
        let factor = q5(&fixture);
        assert_eq!(factor.score, Some(0.0));
        assert_eq!(factor.missing, vec!["product designer".to_string()]);
    }

    #[test]
    fn a_partly_matching_role_is_partly_aligned() {
        let mut fixture = Fixture::section_9();
        fixture.prefs.roles = vec!["data engineer python".to_string()];
        let score = q5(&fixture).score.expect("a score");
        assert!((score - 66.67).abs() < 0.01, "{score}");
    }

    // AC2: no declared goals is a missing input, not a zero — the factor drops
    // out and the axis renormalises.
    #[test]
    fn preferences_with_no_role_and_no_band_leave_the_factor_missing() {
        let mut fixture = Fixture::section_9();
        fixture.prefs.roles = Vec::new();
        let factor = q5(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(factor.effective_weight, 0.0);
        assert_eq!(
            factor.missing,
            vec!["the preferences declare no target role or level".to_string()]
        );
    }

    #[test]
    fn a_role_inside_the_declared_seniority_band_aligns_on_both() {
        let mut fixture = Fixture::section_9();
        fixture.prefs.seniority_min = Some(Seniority::Mid);
        fixture.prefs.seniority_max = Some(Seniority::Lead);
        let factor = q5(&fixture);
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.confidence, Confidence::High);
        assert_eq!(factor.matched.len(), 2);
    }

    #[test]
    fn a_role_two_levels_outside_the_band_aligns_barely() {
        let mut fixture = Fixture::section_9();
        fixture.prefs.roles = Vec::new();
        fixture.prefs.seniority_min = Some(Seniority::Lead);
        fixture.prefs.seniority_max = Some(Seniority::Principal);
        let factor = q5(&fixture);
        // A senior role against a lead band: one level short.
        assert_eq!(factor.score, Some(60.0));
        assert!(!factor.missing.is_empty());

        let mut far = Fixture::section_9();
        far.features = PostingFeatures::new("pst_01SECTION9", 3);
        far.features.extend([seniority(Seniority::Intern)]);
        far.prefs.roles = Vec::new();
        far.prefs.seniority_min = Some(Seniority::Lead);
        far.prefs.seniority_max = Some(Seniority::Principal);
        assert_eq!(q5(&far).score, Some(25.0));
    }

    #[test]
    fn an_unknown_posting_level_is_simply_not_a_band_signal() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.prefs.roles = Vec::new();
        fixture.prefs.seniority_min = Some(Seniority::Mid);
        fixture.prefs.seniority_max = Some(Seniority::Lead);
        // Only the declared band is left, and it cannot be checked.
        let factor = q5(&fixture);
        assert_eq!(factor.score, None);
    }
}
