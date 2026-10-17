//! F4 location / work mode (FR-55, `offer-scoring.md` §3): can the user actually
//! take this job where it is, in the way the posting says it is worked?
//!
//! Tiered, as the research document asks: remote where the user already is is
//! the top tier, hybrid or onsite inside a place they accept is just below it,
//! and relocation is low unless the Preferences say the user would move for the
//! right role. A work mode the user does not accept at all (an onsite role for
//! someone who only takes remote) is penalised hardest and named in `missing`,
//! so the UI can say why.
//!
//! The gate has already had its say (FR-53): a posting that would break a hard
//! location constraint was VETOed before scoring. This factor is the *nuance* —
//! the difference between "possible" and "easy", which is what Fit means.

use super::*;

/// Remote, in a place the user has declared.
const REMOTE_IN_PLACE: f32 = 95.0;
/// Hybrid or onsite, in a place the user accepts.
const HERE: f32 = 90.0;
/// Remote, but the user has declared no place to match it against.
const REMOTE_UNPLACED: f32 = 80.0;
/// Onsite or hybrid elsewhere, for a user who would relocate.
const RELOCATION: f32 = 45.0;
/// Onsite or hybrid elsewhere, for a user who would not.
const OUT_OF_REACH: f32 = 15.0;
/// A work mode the Preferences exclude outright.
const MODE_REFUSED: f32 = 10.0;

/// Whether the posting's mode is one the user accepts; an empty Preferences list
/// means no restriction (FR-7).
fn mode_accepted(inputs: &ScoreInputs<'_>, mode: WorkMode) -> bool {
    inputs.prefs.work_modes.is_empty() || inputs.prefs.work_modes.contains(&mode)
}

/// The first place the posting names that the user has declared, either as a
/// Profile location or as an accepted Preferences location. Compared folded and
/// by containment both ways, because a posting writes "Remote, Spain" where the
/// user wrote "spain".
fn shared_place(inputs: &ScoreInputs<'_>) -> Option<String> {
    let declared: Vec<String> = inputs
        .profile
        .locations
        .iter()
        .chain(inputs.prefs.locations.iter())
        .map(|place| place.trim().to_lowercase())
        .filter(|place| !place.is_empty())
        .collect();
    inputs.posting.locations.iter().find_map(|place| {
        let posting = place.trim().to_lowercase();
        declared
            .iter()
            .any(|want| posting.contains(want.as_str()) || want.contains(&posting))
            .then(|| place.trim().to_string())
    })
}

pub(super) fn score(inputs: &ScoreInputs<'_>, weight: f32) -> FactorScore {
    let mode = inputs.posting.work_mode;
    if mode == WorkMode::Unknown {
        return FactorScore::missing_input(
            Factor::Location,
            weight,
            "the posting states no work mode",
        );
    }
    let word = mode.as_str();

    if !mode_accepted(inputs, mode) {
        return FactorScore::scored(
            Factor::Location,
            MODE_REFUSED,
            weight,
            Confidence::High,
            vec![Evidence::Field("work_mode".to_string())],
        )
        .with_missing(vec![format!("{word} (not a work mode you accept)")]);
    }

    let evidence = vec![Evidence::Field("work_mode".to_string())];
    if mode == WorkMode::Remote {
        let (score, confidence) = match shared_place(inputs) {
            Some(_) => (REMOTE_IN_PLACE, Confidence::High),
            // Remote is possible anywhere; that the user has declared no place
            // is a thinner answer, not a worse job (AC2: confidence, not score).
            None => (REMOTE_UNPLACED, Confidence::Medium),
        };
        return FactorScore::scored(Factor::Location, score, weight, confidence, evidence)
            .with_matched(vec![word.to_string()]);
    }

    if shared_place(inputs).is_some() {
        return FactorScore::scored(Factor::Location, HERE, weight, Confidence::High, evidence)
            .with_matched(vec![word.to_string()]);
    }

    let (score, missing) = if inputs.prefs.relocate_ok {
        (RELOCATION, Vec::new())
    } else {
        (OUT_OF_REACH, inputs.posting.locations.clone())
    };
    FactorScore::scored(Factor::Location, score, weight, Confidence::High, evidence)
        .with_missing(missing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting::WorkMode;
    use crate::scoring::factors::testing::*;

    fn f4(fixture: &Fixture) -> FactorScore {
        score(
            &fixture.inputs(),
            fixture.prefs.weights.of(Factor::Location),
        )
    }

    // §9: remote in-country — the example's 95.
    #[test]
    fn remote_where_the_user_already_is_is_the_top_tier() {
        let factor = f4(&Fixture::section_9());
        assert_eq!(factor.score, Some(95.0));
        assert_eq!(factor.matched, vec!["remote".to_string()]);
        assert_eq!(factor.missing, Vec::<String>::new());
        assert_eq!(factor.confidence, Confidence::High);
    }

    #[test]
    fn hybrid_or_onsite_inside_a_place_the_user_accepts_is_a_high_score() {
        let mut fixture = Fixture::section_9();
        fixture.posting.work_mode = WorkMode::Hybrid;
        fixture.posting.locations = vec!["Madrid, ES".to_string()];
        let factor = f4(&fixture);
        assert_eq!(factor.score, Some(90.0));
        assert_eq!(factor.matched, vec!["hybrid".to_string()]);
    }

    #[test]
    fn onsite_far_away_without_relocating_scores_low_and_names_the_place() {
        let mut fixture = Fixture::section_9();
        fixture.posting.work_mode = WorkMode::Onsite;
        fixture.posting.locations = vec!["Bogotá, CO".to_string()];
        fixture.prefs.work_modes = vec![WorkMode::Remote, WorkMode::Onsite];
        let factor = f4(&fixture);
        assert_eq!(factor.score, Some(15.0));
        assert_eq!(factor.missing, vec!["Bogotá, CO".to_string()]);
    }

    #[test]
    fn onsite_far_away_is_worth_more_when_the_user_would_relocate() {
        let mut fixture = Fixture::section_9();
        fixture.posting.work_mode = WorkMode::Onsite;
        fixture.posting.locations = vec!["Bogotá, CO".to_string()];
        fixture.prefs.work_modes = vec![WorkMode::Remote, WorkMode::Onsite];
        fixture.prefs.relocate_ok = true;
        let factor = f4(&fixture);
        assert_eq!(factor.score, Some(45.0));
        assert_eq!(factor.missing, Vec::<String>::new());
    }

    #[test]
    fn a_mode_the_user_does_not_accept_is_penalised_and_named() {
        let mut fixture = Fixture::section_9();
        fixture.posting.work_mode = WorkMode::Onsite;
        fixture.posting.locations = vec!["Madrid, ES".to_string()];
        let factor = f4(&fixture);
        assert_eq!(factor.score, Some(10.0));
        assert_eq!(
            factor.missing,
            vec!["onsite (not a work mode you accept)".to_string()]
        );
    }

    #[test]
    fn a_posting_that_states_no_work_mode_leaves_the_factor_missing() {
        let mut fixture = Fixture::section_9();
        fixture.posting.work_mode = WorkMode::Unknown;
        let factor = f4(&fixture);
        assert_eq!(factor.score, None);
        assert_eq!(
            factor.missing,
            vec!["the posting states no work mode".to_string()]
        );
    }

    #[test]
    fn remote_with_no_place_the_user_has_declared_is_worth_less_than_in_country() {
        let mut fixture = Fixture::section_9();
        fixture.posting.locations = Vec::new();
        fixture.profile.locations = Vec::new();
        fixture.prefs.locations = Vec::new();
        let factor = f4(&fixture);
        assert_eq!(factor.score, Some(80.0));
        assert_eq!(factor.confidence, Confidence::Medium);
    }

    #[test]
    fn a_place_the_user_accepts_reads_the_same_however_it_is_written() {
        let mut fixture = Fixture::section_9();
        fixture.posting.work_mode = WorkMode::Hybrid;
        fixture.posting.locations = vec!["BARCELONA, ES".to_string()];
        fixture.profile.locations = vec!["barcelona".to_string()];
        let factor = f4(&fixture);
        assert_eq!(factor.score, Some(90.0));
        assert_eq!(factor.matched, vec!["hybrid".to_string()]);
    }
}
