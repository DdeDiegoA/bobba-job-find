//! Settings → Scoring's weights document (S6.8, FR-58, FR-14).
//!
//! [`FactorWeights`] is what scoring reads; this module is what the screen may
//! write. Two rules make the document honest:
//!
//! * **Core validates, never the UI** (AD-11): a weight that is negative or not
//!   a number is refused, and so is a document whose Fit or Quality axis carries
//!   no weight at all — that axis could not be scored, and silently treating it
//!   as zero would be imputing a judgement (AD-8).
//! * **Each axis shares one unit**: the weights the user typed are normalised so
//!   each axis sums to 1 and the two axis weights sum to 1. Scoring renormalises
//!   anyway ([`weighted_mean`]), so this changes no score; it makes the stored
//!   document mean the same thing however the sliders were scaled, and it makes
//!   "did this change?" answer true only when a score would actually move.
//!
//! Reset is the PRD's defaults ([`FactorWeights::DEFAULT`]); a save that lands
//! on them is an ordinary change, and the Preferences owner gives it a new
//! version like any other (AC1, FR-14).
//!
//! [`weighted_mean`]: crate::scoring::factors::weighted_mean

use crate::scoring::factors::{Axis, Factor, FactorWeights};

/// The fields a [`FactorWeights`] document carries, for a violation that names
/// the one to fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeightField {
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    Q1,
    Q2,
    Q3,
    Q4,
    Q5,
    Fit,
    Quality,
}

/// Why a submitted document cannot be stored (FR-58).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeightViolation {
    /// A weight below zero.
    Negative(WeightField),
    /// A weight that is not a finite number (NaN or an infinity).
    NotFinite(WeightField),
    /// Every factor of one axis weighs zero: that axis could not be scored.
    EmptyAxis(Axis),
    /// Both axis weights are zero: no Rank could be formed.
    NoAxisWeight,
}

impl WeightViolation {
    /// The i18n key the UI shows (AD-3: keys cross IPC, never raw text).
    pub fn message_key(&self) -> &'static str {
        match self {
            WeightViolation::Negative(_) => "errors.settings_scoring.negative",
            WeightViolation::NotFinite(_) => "errors.settings_scoring.not_finite",
            WeightViolation::EmptyAxis(_) => "errors.settings_scoring.empty_axis",
            WeightViolation::NoAxisWeight => "errors.settings_scoring.no_axis_weight",
        }
    }
}

/// The weights a document carries, read in document order.
fn fields(weights: &FactorWeights) -> [(WeightField, f32); 13] {
    [
        (WeightField::F1, weights.f1),
        (WeightField::F2, weights.f2),
        (WeightField::F3, weights.f3),
        (WeightField::F4, weights.f4),
        (WeightField::F5, weights.f5),
        (WeightField::F6, weights.f6),
        (WeightField::Q1, weights.q1),
        (WeightField::Q2, weights.q2),
        (WeightField::Q3, weights.q3),
        (WeightField::Q4, weights.q4),
        (WeightField::Q5, weights.q5),
        (WeightField::Fit, weights.fit),
        (WeightField::Quality, weights.quality),
    ]
}

/// Validate a submitted document and normalise each axis to sum 1 (AC1).
///
/// The validation runs over every field first, so a document with two mistakes
/// always reports the same one — the screen points at one input, not a lottery.
pub fn normalize(weights: FactorWeights) -> Result<FactorWeights, WeightViolation> {
    for (field, value) in fields(&weights) {
        if !value.is_finite() {
            return Err(WeightViolation::NotFinite(field));
        }
    }
    for (field, value) in fields(&weights) {
        if value < 0.0 {
            return Err(WeightViolation::Negative(field));
        }
    }
    for axis in [Axis::Fit, Axis::Quality] {
        if sum(axis, &weights) <= 0.0 {
            return Err(WeightViolation::EmptyAxis(axis));
        }
    }
    let axis_total = weights.fit + weights.quality;
    if axis_total <= 0.0 {
        return Err(WeightViolation::NoAxisWeight);
    }

    let fit_share = sum(Axis::Fit, &weights);
    let quality_share = sum(Axis::Quality, &weights);
    let mut normalized = FactorWeights::DEFAULT;
    for factor in Factor::ALL {
        let value = weights.of(factor) / share(factor.axis(), fit_share, quality_share);
        normalized = with(&normalized, factor, value);
    }
    normalized.fit = weights.fit / axis_total;
    normalized.quality = weights.quality / axis_total;
    Ok(normalized)
}

/// The factors of one axis, in weight order.
fn axis_factors(axis: Axis) -> &'static [Factor] {
    match axis {
        Axis::Fit => &Factor::FIT,
        Axis::Quality => &Factor::QUALITY,
    }
}

/// The sum of one axis's factor weights.
fn sum(axis: Axis, weights: &FactorWeights) -> f32 {
    axis_factors(axis)
        .iter()
        .map(|factor| weights.of(*factor))
        .sum()
}

/// The divisor that turns one axis's weights into shares.
fn share(axis: Axis, fit: f32, quality: f32) -> f32 {
    match axis {
        Axis::Fit => fit,
        Axis::Quality => quality,
    }
}

/// One field of a document replaced.
fn with(weights: &FactorWeights, factor: Factor, value: f32) -> FactorWeights {
    let mut next = *weights;
    match factor {
        Factor::Skills => next.f1 = value,
        Factor::Experience => next.f2 = value,
        Factor::Semantic => next.f3 = value,
        Factor::Location => next.f4 = value,
        Factor::Education => next.f5 = value,
        Factor::Language => next.f6 = value,
        Factor::Compensation => next.q1 = value,
        Factor::Company => next.q2 = value,
        Factor::Terms => next.q3 = value,
        Factor::Legitimacy => next.q4 = value,
        Factor::Alignment => next.q5 = value,
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scoring::factors::weighted_mean;
    use crate::scoring::rank::rank_from_axes;

    fn scaled(factor: f32) -> FactorWeights {
        FactorWeights {
            f1: 0.35 * factor,
            f2: 0.25 * factor,
            f3: 0.15 * factor,
            f4: 0.10 * factor,
            f5: 0.10 * factor,
            f6: 0.05 * factor,
            q1: 0.30 * factor,
            q2: 0.25 * factor,
            q3: 0.15 * factor,
            q4: 0.20 * factor,
            q5: 0.10 * factor,
            fit: 0.65,
            quality: 0.35,
        }
    }

    #[test]
    fn the_defaults_normalise_to_themselves() {
        assert_eq!(
            normalize(FactorWeights::DEFAULT),
            Ok(FactorWeights::DEFAULT)
        );
    }

    #[test]
    fn a_uniformly_scaled_document_is_the_same_document() {
        let normalized = normalize(scaled(10.0)).expect("valid");
        for factor in Factor::ALL {
            assert!(
                (normalized.of(factor) - FactorWeights::DEFAULT.of(factor)).abs() < 1e-6,
                "{factor:?}"
            );
        }
    }

    // AC1: the axis weights are the user's own, and they are shares of one.
    #[test]
    fn the_axis_weights_normalise_to_one_unit() {
        let normalized = normalize(FactorWeights {
            fit: 3.0,
            quality: 1.0,
            ..FactorWeights::DEFAULT
        })
        .expect("valid");
        assert!((normalized.fit - 0.75).abs() < 1e-6);
        assert!((normalized.quality - 0.25).abs() < 1e-6);
    }

    // AC1: a changed axis weight moves the Rank, which is what a re-score is for.
    #[test]
    fn the_normalised_weights_are_the_ones_that_rank() {
        let quality_led = normalize(FactorWeights {
            fit: 0.5,
            quality: 0.5,
            ..FactorWeights::DEFAULT
        })
        .expect("valid");
        let ranked = rank_from_axes(Some(81.3), Some(63.9), &quality_led).expect("a rank");
        assert!((ranked.rank - 72.6).abs() < 0.05, "{}", ranked.rank);
    }

    #[test]
    fn a_negative_weight_names_its_field() {
        assert_eq!(
            normalize(FactorWeights {
                f2: -0.1,
                ..FactorWeights::DEFAULT
            }),
            Err(WeightViolation::Negative(WeightField::F2))
        );
        assert_eq!(
            WeightViolation::Negative(WeightField::F2).message_key(),
            "errors.settings_scoring.negative"
        );
    }

    #[test]
    fn a_weight_that_is_not_a_number_names_its_field() {
        assert_eq!(
            normalize(FactorWeights {
                q3: f32::NAN,
                ..FactorWeights::DEFAULT
            }),
            Err(WeightViolation::NotFinite(WeightField::Q3))
        );
        assert_eq!(
            normalize(FactorWeights {
                fit: f32::INFINITY,
                ..FactorWeights::DEFAULT
            }),
            Err(WeightViolation::NotFinite(WeightField::Fit))
        );
    }

    #[test]
    fn an_axis_with_no_weight_cannot_be_scored() {
        let empty_fit = FactorWeights {
            f1: 0.0,
            f2: 0.0,
            f3: 0.0,
            f4: 0.0,
            f5: 0.0,
            f6: 0.0,
            ..FactorWeights::DEFAULT
        };
        assert_eq!(
            normalize(empty_fit),
            Err(WeightViolation::EmptyAxis(Axis::Fit))
        );
        assert_eq!(
            WeightViolation::EmptyAxis(Axis::Quality).message_key(),
            "errors.settings_scoring.empty_axis"
        );
    }

    #[test]
    fn both_axis_weights_at_zero_have_no_rank() {
        assert_eq!(
            normalize(FactorWeights {
                fit: 0.0,
                quality: 0.0,
                ..FactorWeights::DEFAULT
            }),
            Err(WeightViolation::NoAxisWeight)
        );
        assert_eq!(
            WeightViolation::NoAxisWeight.message_key(),
            "errors.settings_scoring.no_axis_weight"
        );
    }

    // The one validation order: a document with two mistakes reports the same
    // one every time (the screen points at one input).
    #[test]
    fn a_document_with_two_mistakes_reports_the_first_in_document_order() {
        let broken = FactorWeights {
            f1: -1.0,
            q2: f32::NAN,
            ..FactorWeights::DEFAULT
        };
        assert_eq!(
            normalize(broken),
            Err(WeightViolation::NotFinite(WeightField::Q2))
        );
    }

    // The shares the scoring engine reads are exactly the normalised ones.
    #[test]
    fn a_normalised_document_feeds_the_weighted_mean_unchanged() {
        let normalized = normalize(FactorWeights {
            f1: 1.0,
            f2: 0.0,
            f3: 0.0,
            f4: 0.0,
            f5: 0.0,
            f6: 0.0,
            ..FactorWeights::DEFAULT
        })
        .expect("valid");
        let (mean, shares) =
            weighted_mean(&[(90.0, normalized.f1), (60.0, normalized.f2)]).expect("two present");
        assert!((mean - 90.0).abs() < 1e-6, "{mean}");
        assert!((shares[0] - 1.0).abs() < 1e-6);
    }
}
