//! F5 education / certifications (FR-55, `offer-scoring.md` §3): a soft factor,
//! not a second gate — the hard version of "a degree is mandatory" belongs to
//! the Gates' eligibility check, not here.
//!
//! Both sides are read as a *level*, never as a title: the posting's study claim
//! ("university degree", "master's degree", "phd") and the Profile's own words
//! (`BSc Computer Science`, `Ingeniería Informática`, `MSc Statistics`) are each
//! mapped onto the same small ladder, so an equivalent degree counts even when
//! the words differ — `offer-scoring.md` §3's "prefer equivalence reasoning over
//! exact-title matching".
//!
//! A posting that asks for no study at all is a full score: nothing asked is
//! nothing to fail, and the confidence says the factor had little to look at. A
//! Profile that states no education *while the posting asks for one* is a
//! missing input — never a zero, because "not filled in yet" and "no degree" are
//! different things (AC2).

use super::*;

/// Levels one below the study the posting asks for.
const ONE_SHORT: f32 = 60.0;
/// Levels further below than that: the requirement is effectively unmet.
const FAR_SHORT: f32 = 25.0;

/// The credential ladder both sides are read on, weakest first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    /// A certificate or a vocational qualification.
    Credential,
    Bachelor,
    Master,
    Doctorate,
}

/// Words that name a level, lowercased; the first match wins, so the strongest
/// word in a phrase is the one that counts ("bachelor's degree" → Bachelor).
const LEVELS: &[(&str, Level)] = &[
    ("phd", Level::Doctorate),
    ("doctorate", Level::Doctorate),
    ("doctoral", Level::Doctorate),
    ("doctorado", Level::Doctorate),
    ("master", Level::Master),
    ("mba", Level::Master),
    ("msc", Level::Master),
    ("maestría", Level::Master),
    ("maestria", Level::Master),
    ("bachelor", Level::Bachelor),
    ("bsc", Level::Bachelor),
    ("university degree", Level::Bachelor),
    ("degree", Level::Bachelor),
    ("licenciatura", Level::Bachelor),
    ("ingeniería", Level::Bachelor),
    ("ingenieria", Level::Bachelor),
    ("grado", Level::Bachelor),
    ("técnico", Level::Credential),
    ("tecnico", Level::Credential),
    ("tecnólogo", Level::Credential),
    ("tecnologo", Level::Credential),
    ("certified", Level::Credential),
    ("certificate", Level::Credential),
    ("certificación", Level::Credential),
];

/// The strongest level any of `phrases` names, if one names any.
fn level_of(phrases: &[String]) -> Option<Level> {
    phrases
        .iter()
        .filter_map(|phrase| {
            let folded = phrase.to_lowercase();
            LEVELS
                .iter()
                .filter(|(word, _)| folded.contains(word))
                .map(|(_, level)| *level)
                .max()
        })
        .max()
}

/// The study the posting asks for: its strongest education claim, with the claim
/// that carried it.
fn asked_study(inputs: &ScoreInputs<'_>) -> Option<(String, Level, Evidence)> {
    let claims: Vec<(String, Level, Evidence)> = inputs
        .features
        .of_kind(crate::posting_features::FeatureKind::Education)
        .filter_map(|claim| {
            let level = level_of(std::slice::from_ref(&claim.value))?;
            Some((
                claim.value.trim().to_string(),
                level,
                claim.evidence.clone(),
            ))
        })
        .collect();
    let strongest = claims.iter().map(|(_, level, _)| *level).max()?;
    claims.into_iter().find(|(_, level, _)| *level == strongest)
}

pub(super) fn score(inputs: &ScoreInputs<'_>, weight: f32) -> FactorScore {
    let Some((asked, required, evidence)) = asked_study(inputs) else {
        // A posting that asks for no study states no requirement: the factor has
        // no input and says so. Scoring it 100 would let an empty posting earn a
        // perfect Fit (AC2: nothing is imputed).
        return FactorScore::missing_input(
            Factor::Education,
            weight,
            "the posting asks for no study",
        );
    };

    // The Profile's own words, plus its certifications: a certificate is a
    // credential, and never a degree (the ladder keeps the two apart).
    let mut stated = inputs.profile.education.clone();
    stated.extend(inputs.profile.certifications.iter().cloned());
    let Some(held) = level_of(&stated) else {
        return FactorScore::missing_input(
            Factor::Education,
            weight,
            "the profile states no education",
        );
    };

    let score = match (held as i32) - (required as i32) {
        gap if gap >= 0 => 100.0,
        -1 => ONE_SHORT,
        _ => FAR_SHORT,
    };

    let met = score == 100.0;
    FactorScore::scored(
        Factor::Education,
        score,
        weight,
        Confidence::High,
        vec![evidence],
    )
    .with_matched(if met { vec![asked.clone()] } else { Vec::new() })
    .with_missing(if met { Vec::new() } else { vec![asked] })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting_features::PostingFeatures;
    use crate::scoring::factors::testing::*;

    fn f5(fixture: &Fixture) -> FactorScore {
        score(
            &fixture.inputs(),
            fixture.prefs.weights.of(Factor::Education),
        )
    }

    /// A fixture whose posting asks for `study` and nothing else.
    fn asking(study: &str) -> Fixture {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.features.extend([education(study)]);
        fixture
    }

    // AC2: a posting that asks for no degree states no requirement, so the
    // factor drops out instead of earning a free 100 (§9's illustrative column
    // reads 100 there; nothing is imputed here).
    #[test]
    fn a_posting_that_asks_for_no_study_leaves_the_factor_missing() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        let factor = f5(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(factor.effective_weight, 0.0);
        assert_eq!(
            factor.missing,
            vec!["the posting asks for no study".to_string()]
        );
    }

    #[test]
    fn a_degree_the_profile_holds_is_a_full_score() {
        let factor = f5(&asking("bachelor's degree"));
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.matched, vec!["bachelor's degree".to_string()]);
        assert_eq!(factor.confidence, Confidence::High);
    }

    #[test]
    fn one_level_short_is_partial_credit_not_a_wall() {
        let factor = f5(&asking("master's degree"));
        assert_eq!(factor.score, Some(60.0));
        assert_eq!(factor.missing, vec!["master's degree".to_string()]);
        assert_eq!(factor.matched, Vec::<String>::new());
    }

    #[test]
    fn a_doctorate_the_profile_cannot_reach_is_a_low_score() {
        let mut fixture = asking("phd");
        fixture.profile.education = vec!["BSc Statistics".to_string()];
        assert_eq!(f5(&fixture).score, Some(25.0));

        // One rung short is the softer case: a master's against a doctorate.
        fixture.profile.education = vec!["MSc Statistics".to_string()];
        assert_eq!(f5(&fixture).score, Some(60.0));
    }

    #[test]
    fn an_equivalent_degree_counts_even_when_the_title_differs() {
        let mut fixture = asking("university degree");
        fixture.profile.education = vec!["Ingeniería Informática".to_string()];
        let factor = f5(&fixture);
        assert_eq!(factor.score, Some(100.0));
        assert_eq!(factor.matched, vec!["university degree".to_string()]);
    }

    #[test]
    fn a_certification_is_a_credential_but_not_a_degree() {
        let mut fixture = asking("bachelor's degree");
        fixture.profile.education = Vec::new();
        fixture.profile.certifications = vec!["AWS Certified Developer".to_string()];
        assert_eq!(f5(&fixture).score, Some(60.0));

        let mut vocational = asking("técnico");
        vocational.profile.education = Vec::new();
        vocational.profile.certifications = vec!["AWS Certified Developer".to_string()];
        assert_eq!(f5(&vocational).score, Some(100.0));
    }

    // AC2: a posting that asks for a degree and a Profile that states none is a
    // missing input, not a zero — the axis renormalises instead of punishing a
    // user who simply has not filled the section in.
    #[test]
    fn a_profile_that_states_no_education_leaves_the_factor_missing() {
        let mut fixture = asking("master's degree");
        fixture.profile.education = Vec::new();
        fixture.profile.certifications = Vec::new();
        let factor = f5(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(factor.effective_weight, 0.0);
        assert_eq!(
            factor.missing,
            vec!["the profile states no education".to_string()]
        );
    }

    #[test]
    fn the_postings_own_words_are_the_evidence() {
        let factor = f5(&asking("master's degree"));
        assert_eq!(factor.evidence.len(), 1);
    }
}
