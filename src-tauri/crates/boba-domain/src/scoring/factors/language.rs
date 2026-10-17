//! F6 language nuance (FR-55, `offer-scoring.md` §3): the bonus or small penalty
//! *beyond* the language gate.
//!
//! FR-53's gate is the hard half — a language the posting requires and the
//! Profile never declares is a FLAG or a VETO before scoring. This factor is the
//! soft half: the posting names the languages it works in, the Profile declares
//! levels on the CEFR scale, and a declared CEFR B2 or better clears the
//! "professional working proficiency" bar those postings mean. Meeting it or
//! beating it is a full score; sitting below it is the small penalty the
//! research document asks for, named in `missing` so the user can see which
//! language it is.
//!
//! Language tags are compared on their primary subtag, because a posting
//! writing `en-US` and a user writing `en` are talking about the same language.

use super::*;
use crate::scoring::gates::Cefr;

/// The bar a posting means when it names a working language without a level.
const PROFESSIONAL: Cefr = Cefr::B2;
/// A declared level below that bar.
const BELOW_BAR: f32 = 70.0;

/// The primary subtag of a BCP-47 tag, folded: `EN-us` and `en` are `en`.
fn primary(tag: &str) -> String {
    tag.trim()
        .split(['-', '_'])
        .next()
        .unwrap_or_default()
        .to_lowercase()
}

pub(super) fn score(inputs: &ScoreInputs<'_>, weight: f32) -> FactorScore {
    let asked: Vec<String> = inputs
        .features
        .values(crate::posting_features::FeatureKind::Language)
        .into_iter()
        .map(primary)
        .collect();

    if asked.is_empty() {
        // The posting names no language: no stated bar to be above or below, and
        // an absent requirement is not a full score (AC2).
        return FactorScore::missing_input(
            Factor::Language,
            weight,
            "the posting names no language",
        );
    }

    let declared: Vec<(String, Cefr)> = inputs
        .profile
        .languages
        .iter()
        .map(|language| (primary(&language.language), language.level))
        .collect();
    let named: Vec<(String, Cefr)> = asked
        .iter()
        .filter_map(|tag| {
            declared
                .iter()
                .filter(|(mine, _)| mine == tag)
                .map(|(_, level)| (tag.clone(), *level))
                .max_by_key(|(_, level)| *level)
        })
        .collect();

    let Some((tag, level)) = named.iter().max_by_key(|(_, level)| *level).cloned() else {
        return FactorScore::missing_input(
            Factor::Language,
            weight,
            "the profile declares no level for the languages the posting names",
        );
    };

    let evidence = inputs
        .features
        .first(crate::posting_features::FeatureKind::Language)
        .map(|claim| vec![claim.evidence.clone()])
        .unwrap_or_default();

    if level >= PROFESSIONAL {
        let matched = named.into_iter().map(|(tag, _)| tag).collect();
        FactorScore::scored(Factor::Language, 100.0, weight, Confidence::High, evidence)
            .with_matched(matched)
    } else {
        FactorScore::scored(
            Factor::Language,
            BELOW_BAR,
            weight,
            Confidence::High,
            evidence,
        )
        .with_missing(vec![tag])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting_features::PostingFeatures;
    use crate::scoring::factors::testing::*;
    use crate::scoring::gates::{Cefr, DeclaredLanguage};

    fn f6(fixture: &Fixture) -> FactorScore {
        score(
            &fixture.inputs(),
            fixture.prefs.weights.of(Factor::Language),
        )
    }

    /// A fixture whose posting names `tag` and nothing else.
    fn asking(tag: &str) -> Fixture {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.features.extend([language(tag)]);
        fixture.profile.languages = Vec::new();
        fixture
    }

    fn speaking(tag: &str, level: Cefr) -> DeclaredLanguage {
        DeclaredLanguage {
            language: tag.to_string(),
            level,
        }
    }

    // §9: English B2 clears the "professional" bar the posting implies.
    #[test]
    fn the_worked_examples_b2_english_is_a_full_score() {
        let factor = f6(&Fixture::section_9());
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.matched, vec!["en".to_string()]);
        assert_eq!(factor.missing, Vec::<String>::new());
        assert_eq!(factor.confidence, Confidence::High);
    }

    #[test]
    fn exceeding_the_bar_is_still_a_full_score() {
        let mut fixture = asking("en");
        fixture.profile.languages = vec![speaking("en", Cefr::C1)];
        assert_eq!(f6(&fixture).score, Some(100.0));
    }

    #[test]
    fn a_level_below_the_bar_is_a_small_penalty_and_is_named() {
        let mut fixture = asking("en");
        fixture.profile.languages = vec![speaking("en", Cefr::B1)];
        let factor = f6(&fixture);
        assert_eq!(factor.score, Some(70.0));
        assert_eq!(factor.missing, vec!["en".to_string()]);
        assert_eq!(factor.matched, Vec::<String>::new());
    }

    #[test]
    fn the_best_declared_language_of_the_ones_asked_for_counts() {
        let mut fixture = asking("es");
        fixture.profile.languages = vec![speaking("en", Cefr::C1), speaking("es", Cefr::C2)];
        let factor = f6(&fixture);
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.matched, vec!["es".to_string()]);
    }

    #[test]
    fn a_language_the_profile_never_declared_leaves_the_factor_missing() {
        let mut fixture = asking("de");
        fixture.profile.languages = vec![speaking("en", Cefr::C1)];
        let factor = f6(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(
            factor.missing,
            vec!["the profile declares no level for the languages the posting names".to_string()]
        );
    }

    // AC2: a posting that names no language sets no bar, so there is nothing to
    // be a bonus or a penalty against.
    #[test]
    fn a_posting_that_names_no_language_leaves_the_factor_missing() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        let factor = f6(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(
            factor.missing,
            vec!["the posting names no language".to_string()]
        );
    }

    #[test]
    fn a_region_tag_meets_a_language_the_profile_declared_plainly() {
        let mut fixture = asking("en-US");
        fixture.profile.languages = vec![speaking("en", Cefr::C2)];
        assert_eq!(f6(&fixture).score, Some(100.0));
    }
}
