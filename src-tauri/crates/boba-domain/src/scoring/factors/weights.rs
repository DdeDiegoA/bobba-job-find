//! The factor weights and the renormalisation AC2 asks for (FR-55..58, AD-8).
//!
//! The defaults are the PRD's table, and the two axis weights are
//! `offer-scoring.md` §1's 0.65 / 0.35. A user may change any of them (FR-58);
//! S6.8 owns the document that stores the change and the Settings screen that
//! edits it, and hands the values over as one [`FactorWeights`].
//!
//! [`weighted_mean`] is the rule that makes a missing input honest: only the
//! factors that *have* a score share the weight of the ones that do not, so the
//! axis stays a 0–100 number instead of being pulled toward an invented middle.
//! With nothing present there is no number at all — `None`, never `0.0`.

use serde::{Deserialize, Serialize};

use super::Factor;

/// The weight each factor carries, and how Fit and Quality combine into Rank.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FactorWeights {
    pub f1: f32,
    pub f2: f32,
    pub f3: f32,
    pub f4: f32,
    pub f5: f32,
    pub f6: f32,
    pub q1: f32,
    pub q2: f32,
    pub q3: f32,
    pub q4: f32,
    pub q5: f32,
    /// `w_fit` of `Rank = w_fit·Fit + w_quality·Quality`.
    pub fit: f32,
    /// `w_quality` of the same formula.
    pub quality: f32,
}

impl FactorWeights {
    /// The PRD's defaults (FR-55, FR-56, FR-57): what a user who never opened
    /// the weights screen scores with.
    pub const DEFAULT: Self = Self {
        f1: 0.35,
        f2: 0.25,
        f3: 0.15,
        f4: 0.10,
        f5: 0.10,
        f6: 0.05,
        q1: 0.30,
        q2: 0.25,
        q3: 0.15,
        q4: 0.20,
        q5: 0.10,
        fit: 0.65,
        quality: 0.35,
    };

    /// The weight one factor carries.
    pub fn of(self, factor: Factor) -> f32 {
        match factor {
            Factor::Skills => self.f1,
            Factor::Experience => self.f2,
            Factor::Semantic => self.f3,
            Factor::Location => self.f4,
            Factor::Education => self.f5,
            Factor::Language => self.f6,
            Factor::Compensation => self.q1,
            Factor::Company => self.q2,
            Factor::Terms => self.q3,
            Factor::Legitimacy => self.q4,
            Factor::Alignment => self.q5,
        }
    }
}

impl Default for FactorWeights {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// The weighted mean of the factors that have a score, renormalised over the
/// weights they carry (AC2): `(mean, the share each one ended up with)`.
///
/// `None` when nothing is present, or when the present weights are all zero —
/// there is no mean of nothing, and inventing one would be exactly the
/// imputation AD-8 forbids.
pub fn weighted_mean(present: &[(f32, f32)]) -> Option<(f32, Vec<f32>)> {
    let total: f32 = present.iter().map(|(_, weight)| weight).sum();
    // A total that is zero, negative or not a number has no shares to give:
    // refuse it rather than divide by it.
    if present.is_empty() || !total.is_finite() || total <= 0.0 {
        return None;
    }
    let shares: Vec<f32> = present.iter().map(|(_, weight)| weight / total).collect();
    let mean: f32 = present
        .iter()
        .zip(&shares)
        .map(|((score, _), share)| score * share)
        .sum();
    Some((mean, shares))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_the_ones_the_prd_pins() {
        let weights = FactorWeights::DEFAULT;
        assert_eq!(weights.f1, 0.35);
        assert_eq!(weights.f2, 0.25);
        assert_eq!(weights.f3, 0.15);
        assert_eq!(weights.f4, 0.10);
        assert_eq!(weights.f5, 0.10);
        assert_eq!(weights.f6, 0.05);
        assert_eq!(weights.q1, 0.30);
        assert_eq!(weights.q2, 0.25);
        assert_eq!(weights.q3, 0.15);
        assert_eq!(weights.q4, 0.20);
        assert_eq!(weights.q5, 0.10);
        assert_eq!(weights.fit, 0.65);
        assert_eq!(weights.quality, 0.35);
        assert_eq!(FactorWeights::default(), FactorWeights::DEFAULT);
    }

    #[test]
    fn each_factor_reads_its_own_weight() {
        let weights = FactorWeights::DEFAULT;
        assert_eq!(weights.of(Factor::Skills), 0.35);
        assert_eq!(weights.of(Factor::Language), 0.05);
        assert_eq!(weights.of(Factor::Compensation), 0.30);
        assert_eq!(weights.of(Factor::Alignment), 0.10);
    }

    #[test]
    fn the_fit_axis_weights_sum_to_one() {
        let weights = FactorWeights::DEFAULT;
        let fit: f32 = Factor::FIT.iter().map(|f| weights.of(*f)).sum();
        let quality: f32 = Factor::QUALITY.iter().map(|f| weights.of(*f)).sum();
        assert!((fit - 1.0).abs() < 1e-6, "{fit}");
        assert!((quality - 1.0).abs() < 1e-6, "{quality}");
    }

    // AC2: the present factors share the missing one's weight; nothing is imputed.
    #[test]
    fn a_missing_factor_leaves_its_weight_to_the_present_ones() {
        let (mean, shares) = weighted_mean(&[(80.0, 0.15), (100.0, 0.10)]).expect("two present");
        assert!((shares[0] - 0.6).abs() < 1e-6, "{shares:?}");
        assert!((shares[1] - 0.4).abs() < 1e-6, "{shares:?}");
        assert!((mean - 88.0).abs() < 1e-4, "{mean}");
    }

    #[test]
    fn the_configured_weights_are_the_ones_used_when_nothing_is_missing() {
        let (mean, shares) = weighted_mean(&[
            (88.0, 0.35),
            (58.0, 0.25),
            (82.0, 0.15),
            (95.0, 0.10),
            (100.0, 0.10),
            (100.0, 0.05),
        ])
        .expect("six present");
        assert!((mean - 82.1).abs() < 1e-4, "{mean}");
        assert!((shares.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn nothing_present_is_no_score_at_all() {
        assert_eq!(weighted_mean(&[]), None);
        assert_eq!(weighted_mean(&[(50.0, 0.0), (60.0, 0.0)]), None);
    }
}
