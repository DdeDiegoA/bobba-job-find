//! F3 domain / role similarity (FR-55, `offer-scoring.md` §3; S6.9 supplies the
//! encoder).

use serde::{Deserialize, Serialize};

use super::*;

/// How alike the Offer's text and the Profile are, as the on-device encoder
/// measures it: cosine similarity, 0–1 (`offer-scoring.md` §3's F3).
///
/// A newtype so an out-of-range or NaN cosine from an adapter cannot reach the
/// card: [`SemanticScore::new`] refuses one, and the factor then reports a
/// missing input instead of a wrong number (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SemanticScore(f32);

impl SemanticScore {
    /// A measured cosine, `None` outside 0–1 or when it is not a number.
    pub fn new(value: f32) -> Option<Self> {
        (value.is_finite() && (0.0..=1.0).contains(&value)).then_some(Self(value))
    }

    /// The measured similarity.
    pub fn get(self) -> f32 {
        self.0
    }
}

pub(super) fn score(inputs: &ScoreInputs<'_>, weight: f32) -> FactorScore {
    // The encoder is an optional download and S6.9's adapter is what runs it
    // (FR-55, architecture §8): no similarity in means no F3 at all, and the
    // axis carries its weight to the factors that do have an answer (AC2).
    let Some(similarity) = inputs.semantic else {
        return FactorScore::missing_input(
            Factor::Semantic,
            weight,
            "no embeddings installed (F3 drops out)",
        );
    };
    FactorScore::scored(
        Factor::Semantic,
        100.0 * similarity.get(),
        weight,
        Confidence::High,
        Vec::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoring::factors::testing::*;

    fn f3(fixture: &Fixture) -> FactorScore {
        score(
            &fixture.inputs(),
            fixture.prefs.weights.of(Factor::Semantic),
        )
    }

    // AC2: without embeddings F3 has no input, and the axis renormalises.
    #[test]
    fn without_embeddings_the_factor_says_what_it_misses() {
        let factor = f3(&Fixture::section_9());
        assert_eq!(factor.score, None);
        assert_eq!(factor.effective_weight, 0.0);
        assert_eq!(factor.confidence, Confidence::Low);
        assert_eq!(
            factor.missing,
            vec!["no embeddings installed (F3 drops out)".to_string()]
        );
    }

    #[test]
    fn a_measured_cosine_becomes_the_score() {
        let mut fixture = Fixture::section_9();
        fixture.semantic = SemanticScore::new(0.82);
        let factor = f3(&fixture);
        assert_eq!(factor.score, Some(82.0));
        assert_eq!(factor.confidence, Confidence::High);
        assert!(factor.matched.is_empty());
        assert!(factor.missing.is_empty());
    }

    #[test]
    fn a_perfect_and_a_useless_match_sit_at_the_ends() {
        let mut fixture = Fixture::section_9();
        fixture.semantic = SemanticScore::new(1.0);
        assert_eq!(f3(&fixture).score, Some(100.0));
        fixture.semantic = SemanticScore::new(0.0);
        assert_eq!(f3(&fixture).score, Some(0.0));
    }

    #[test]
    fn a_cosine_outside_zero_to_one_is_refused() {
        assert_eq!(SemanticScore::new(1.4), None);
        assert_eq!(SemanticScore::new(-0.1), None);
        assert_eq!(SemanticScore::new(f32::NAN), None);
        assert_eq!(SemanticScore::new(f32::INFINITY), None);
        assert_eq!(SemanticScore::new(0.5).map(SemanticScore::get), Some(0.5));
    }
}
