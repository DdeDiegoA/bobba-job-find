//! Rank, Verdict and the ScoreCard (FR-57, FR-61, AD-8, `offer-scoring.md` §6).
//!
//! This module turns eleven factors into the two axes and the one number the UI
//! leads with:
//!
//! ```text
//! Fit     = weighted mean of F1..F6      (can I get it?)
//! Quality = weighted mean of Q1..Q5      (do I want it?)
//! Rank    = w_fit·Fit + w_quality·Quality    (0.65 / 0.35 by default)
//! Verdict = the band Rank falls in       (Strong · Good · Moderate · Weak · Poor)
//! ```
//!
//! Both levels renormalise when something is missing, and that is the whole
//! point of AC2: a factor with no input carries `score: None` and a share of
//! `0.0`, and the factors that did answer share its weight. The same rule runs
//! one level up — an axis nothing is known about leaves its share of the Rank to
//! the other axis. **Renormalising is not imputing**: the number that comes out
//! is a mean of real readings, and the lower [`Confidence`] is what tells the
//! user (and FR-57's "shown as a lead") that less of the picture was visible.
//!
//! A card is immutable once stored (AC3): it is keyed by the Profile version,
//! the Preferences version and the model version it was computed for
//! ([`CardVersions`]), so a change to any of the three produces a *new* card and
//! leaves the old one readable exactly as it was.
//!
//! [`score`] is the pure function of AD-8, and [`rank_from_axes`] is the same
//! rank step on its own — the seam `offer-scoring.md` §9's published example
//! anchors to.

use serde::{Deserialize, Serialize};

use crate::scoring::gates::{Confidence, GateReport};

use super::factors::{
    Axis, Factor, FactorScore, FactorWeights, Prefs, ScoreInputs, fit as fit_factors,
    quality as quality_factors, weighted_mean,
};

/// The version of this scoring model: the weights' defaults, the lexicons, the
/// family tables and the formulas. Bump it whenever a change to any of them
/// would move a score of an unchanged Offer, because a ScoreCard is keyed by it
/// (AC3, FR-58) and the app re-scores when it changes.
pub const MODEL_VERSION: u32 = 1;

/// The band a Rank falls in (FR-57, `offer-scoring.md` §6's thresholds).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// 75 and above.
    Strong,
    /// 60–74.
    Good,
    /// 45–59.
    Moderate,
    /// 30–44.
    Weak,
    /// Below 30.
    Poor,
}

impl Verdict {
    /// Every band, best first.
    pub const ALL: [Verdict; 5] = [
        Verdict::Strong,
        Verdict::Good,
        Verdict::Moderate,
        Verdict::Weak,
        Verdict::Poor,
    ];

    /// The band a Rank falls in. A NaN rank is `Poor`: it can only come from a
    /// caller that bypassed [`rank_from_axes`], and guessing "Strong" for it
    /// would be the worst kind of wrong.
    pub fn from_score(rank: f32) -> Self {
        if rank.is_nan() {
            return Verdict::Poor;
        }
        if rank >= 75.0 {
            Verdict::Strong
        } else if rank >= 60.0 {
            Verdict::Good
        } else if rank >= 45.0 {
            Verdict::Moderate
        } else if rank >= 30.0 {
            Verdict::Weak
        } else {
            Verdict::Poor
        }
    }

    /// The stored / serialised form: the `score_card.verdict` column and the
    /// Inbox's section ids (FR-63).
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Strong => "strong",
            Verdict::Good => "good",
            Verdict::Moderate => "moderate",
            Verdict::Weak => "weak",
            Verdict::Poor => "poor",
        }
    }

    /// Parse the stored form, `None` for anything else.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|band| band.as_str() == value)
    }
}

/// The two axes and the Rank they combine into (FR-57).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ranked {
    /// `None` when no Fit factor had an input (AC2).
    pub fit: Option<f32>,
    /// `None` when no Quality factor had an input (AC2).
    pub quality: Option<f32>,
    /// The Rank the axes combine into, renormalised over the ones that exist.
    pub rank: f32,
    pub verdict: Verdict,
}

/// The Rank of two axis values (FR-57), with each axis's configured weight.
///
/// `None` when neither axis has a value: there is nothing to rank, and a zero
/// would rank the Offer last as if the model had judged it. One axis alone ranks
/// on that axis (its weight renormalised to all of it) — the honest answer when
/// only half the picture is visible.
pub fn rank_from_axes(
    fit: Option<f32>,
    quality: Option<f32>,
    weights: &FactorWeights,
) -> Option<Ranked> {
    let present: Vec<(f32, f32)> = match (fit, quality) {
        (Some(fit), Some(quality)) => vec![(fit, weights.fit), (quality, weights.quality)],
        (Some(fit), None) => vec![(fit, 1.0)],
        (None, Some(quality)) => vec![(quality, 1.0)],
        (None, None) => return None,
    };
    let (rank, _) = weighted_mean(&present)?;
    Some(Ranked {
        fit,
        quality,
        rank,
        verdict: Verdict::from_score(rank),
    })
}

/// Everything one Offer's card is (FR-55..57, FR-61).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreCard {
    /// The model version this card was computed by part of its identity (AC3).
    pub model_version: u32,
    /// All eleven factors, Fit then Quality, each with its own evidence (FR-55).
    pub factors: Vec<FactorScore>,
    /// The Fit axis, `None` when no Fit factor had an input.
    pub fit: Option<f32>,
    /// The Quality axis, `None` when no Quality factor had an input.
    pub quality: Option<f32>,
    /// `w_fit·Fit + w_quality·Quality`, renormalised over the axes that exist.
    pub rank: f32,
    pub verdict: Verdict,
    /// How much of the picture the model could see (offer-scoring.md §6:
    /// "coverage of available data"). FR-57: a [`Confidence::Low`] card is shown
    /// as a lead, not as a judgement.
    pub confidence: Confidence,
}

impl ScoreCard {
    /// One factor's answer, `None` only for a factor this card does not hold.
    pub fn factor(&self, factor: Factor) -> Option<&FactorScore> {
        self.factors.iter().find(|score| score.factor == factor)
    }

    /// FR-57: a card the model could barely see is a lead, not a verdict.
    pub fn is_lead(&self) -> bool {
        self.confidence == Confidence::Low
    }
}

/// How much of the eleven-factor picture the card could see, as the card's
/// overall confidence (`offer-scoring.md` §6). Counting is enough: every factor
/// reports its own confidence for its own inputs, and this is the one number the
/// Inbox's "Lead" badge and FR-57 read.
fn coverage(present: usize) -> Confidence {
    let ratio = present as f32 / Factor::ALL.len() as f32;
    if ratio >= 1.0 {
        Confidence::High
    } else if ratio >= 0.7 {
        Confidence::Medium
    } else {
        Confidence::Low
    }
}

/// The weighted mean of one axis, writing each present factor's share into
/// `effective_weight` (0.0 for a factor that could not answer).
fn axis(factors: &mut [FactorScore]) -> Option<f32> {
    let present: Vec<(f32, f32)> = factors
        .iter()
        .filter_map(|factor| Some((factor.score?, factor.weight)))
        .collect();
    let (mean, shares) = weighted_mean(&present)?;
    let mut shares = shares.into_iter();
    for factor in factors.iter_mut() {
        factor.effective_weight = if factor.score.is_some() {
            shares.next().unwrap_or(0.0)
        } else {
            0.0
        };
    }
    Some(mean)
}

/// Build the card from factors already computed (FR-57).
///
/// `None` when neither axis has a single factor to stand on: an Offer the model
/// knows nothing about has no card, rather than a card saying "0".
pub fn card(
    model_version: u32,
    mut factors: Vec<FactorScore>,
    weights: &FactorWeights,
) -> Option<ScoreCard> {
    let (fit, quality) = {
        let (mut fit_factors, mut quality_factors): (Vec<FactorScore>, Vec<FactorScore>) = factors
            .drain(..)
            .partition(|factor| factor.factor.axis() == Axis::Fit);
        let fit = axis(&mut fit_factors);
        let quality = axis(&mut quality_factors);
        fit_factors.append(&mut quality_factors);
        factors = fit_factors;
        (fit, quality)
    };

    let ranked = rank_from_axes(fit, quality, weights)?;
    let present = factors
        .iter()
        .filter(|factor| factor.score.is_some())
        .count();
    Some(ScoreCard {
        model_version,
        factors,
        fit: ranked.fit,
        quality: ranked.quality,
        rank: ranked.rank,
        verdict: ranked.verdict,
        confidence: coverage(present),
    })
}

/// Score one Offer (AD-8, FR-55..57). Pure: the same inputs always give the same
/// card (FR-61), and nothing here reads a clock, a file or a model.
pub fn score(inputs: &ScoreInputs<'_>) -> Option<ScoreCard> {
    let mut factors = fit_factors(inputs);
    factors.extend(quality_factors(inputs));
    card(MODEL_VERSION, factors, &inputs.prefs.weights)
}

/// A fresh ULID for a stored card, prefixed by type (AD-17: `sc_…`), so a
/// `score_card` row is addressable the way every other row is.
pub fn new_card_id() -> String {
    format!("sc_{}", ulid::Ulid::generate())
}

/// What a card was computed for: the identity half of a `score_card` row
/// (AC3, FR-61). Any change to one of the three means the card is no longer
/// current and the Offer is re-scored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CardVersions {
    /// The `profile.version` the Profile snapshot was taken at (FR-13, AD-15).
    pub profile_version: u32,
    /// The Preferences version (FR-14, S3.7's `PreferencesSnapshot::version`).
    pub prefs_version: u32,
    /// [`MODEL_VERSION`] at the time of scoring.
    pub model_version: u32,
}

/// One stored ScoreCard: a `score_card` row with the gates that preceded it
/// (architecture §12). Immutable — it is written once and read forever, so a
/// re-score adds a row and never rewrites one (AC3).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScoreCardRow {
    /// ULID of the row (`sc_…`).
    pub id: String,
    pub posting_id: String,
    /// The three versions the card is keyed by (AC3).
    pub profile_version: u32,
    pub prefs_version: u32,
    pub model_version: u32,
    pub card: ScoreCard,
    /// The five Gates' outcome for this Offer (FR-53): stored with the card so
    /// the Excluded list and the Offer detail show one consistent report.
    pub gates: GateReport,
    /// UTC RFC 3339, from the injected clock (AD-17).
    pub created_at: String,
}

impl ScoreCardRow {
    /// A row for one Offer, as the scoring use case writes it.
    pub fn new(
        id: impl Into<String>,
        posting_id: impl Into<String>,
        versions: CardVersions,
        card: ScoreCard,
        gates: GateReport,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            posting_id: posting_id.into(),
            profile_version: versions.profile_version,
            prefs_version: versions.prefs_version,
            model_version: versions.model_version,
            card,
            gates,
            created_at: created_at.into(),
        }
    }

    /// The versions this row was computed for (AC3: what a re-score compares).
    pub fn versions(&self) -> CardVersions {
        CardVersions {
            profile_version: self.profile_version,
            prefs_version: self.prefs_version,
            model_version: self.model_version,
        }
    }
}

/// The weights a Preferences document carries, for callers that only hold the
/// card's own contract (S6.8's Settings page links here).
pub fn default_weights(prefs: &Prefs) -> FactorWeights {
    prefs.weights
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posting::WorkMode;
    use crate::posting_features::PostingFeatures;
    use crate::scoring::factors::testing::*;
    use crate::scoring::factors::{MarketData, Prefs, ProfileSnapshot};
    use crate::scoring::gates::Verdict as GateVerdict;

    #[test]
    fn the_five_bands_cover_the_scale() {
        assert_eq!(Verdict::from_score(100.0), Verdict::Strong);
        assert_eq!(Verdict::from_score(75.0), Verdict::Strong);
        assert_eq!(Verdict::from_score(74.9), Verdict::Good);
        assert_eq!(Verdict::from_score(60.0), Verdict::Good);
        assert_eq!(Verdict::from_score(59.9), Verdict::Moderate);
        assert_eq!(Verdict::from_score(45.0), Verdict::Moderate);
        assert_eq!(Verdict::from_score(44.9), Verdict::Weak);
        assert_eq!(Verdict::from_score(30.0), Verdict::Weak);
        assert_eq!(Verdict::from_score(29.9), Verdict::Poor);
        assert_eq!(Verdict::from_score(0.0), Verdict::Poor);
        assert_eq!(Verdict::from_score(f32::NAN), Verdict::Poor);
    }

    #[test]
    fn the_bands_round_trip_through_their_stored_form() {
        for verdict in Verdict::ALL {
            assert_eq!(Verdict::parse(verdict.as_str()), Some(verdict));
        }
        assert_eq!(Verdict::ALL.len(), 5);
        assert_eq!(Verdict::parse("excellent"), None);
    }

    // AC4: the §9 worked example's published axes and its Rank of 75.2.
    #[test]
    fn the_worked_examples_published_axes_rank_75_2() {
        let ranked =
            rank_from_axes(Some(81.3), Some(63.9), &FactorWeights::DEFAULT).expect("a rank");
        assert!((ranked.rank - 75.2).abs() <= 0.1, "{}", ranked.rank);
        assert_eq!(ranked.verdict, Verdict::Strong);
    }

    #[test]
    fn the_axis_weights_the_user_set_are_the_ones_used() {
        let quality_first = FactorWeights {
            fit: 0.5,
            quality: 0.5,
            ..FactorWeights::DEFAULT
        };
        let ranked = rank_from_axes(Some(81.3), Some(63.9), &quality_first).expect("a rank");
        assert!((ranked.rank - 72.6).abs() < 0.05, "{}", ranked.rank);
    }

    // AC2: an axis nothing is known about leaves its share to the other.
    #[test]
    fn a_missing_axis_leaves_its_weight_to_the_other() {
        let ranked = rank_from_axes(Some(80.0), None, &FactorWeights::DEFAULT).expect("a rank");
        // The Fit alone, at its whole weight: 80, not 0.65·80.
        assert!((ranked.rank - 80.0).abs() < 1e-4, "{}", ranked.rank);
        assert_eq!(ranked.verdict, Verdict::Strong);
        assert_eq!(ranked.quality, None);
        assert_eq!(ranked.fit, Some(80.0));
    }

    #[test]
    fn nothing_on_either_axis_is_no_rank_at_all() {
        assert_eq!(rank_from_axes(None, None, &FactorWeights::DEFAULT), None);
    }

    #[test]
    fn an_offer_the_model_knows_nothing_about_has_no_card() {
        let mut fixture = Fixture::section_9();
        fixture.features = PostingFeatures::new("pst_01SECTION9", 3);
        fixture.profile = ProfileSnapshot::empty();
        fixture.prefs = Prefs::default();
        fixture.market = MarketData::default();
        fixture.posting.money = None;
        fixture.posting.posted_at = None;
        fixture.posting.work_mode = WorkMode::Unknown;
        fixture.posting.description_text = String::new();
        fixture.posting.locations = Vec::new();
        assert_eq!(score(&fixture.inputs()), None);
    }

    #[test]
    fn the_card_holds_every_factor_it_could_compute() {
        let fixture = Fixture::section_9();
        let card = score(&fixture.inputs()).expect("a card");
        assert_eq!(card.factors.len(), 11);
        assert_eq!(card.model_version, MODEL_VERSION);
        let fit = card.fit.expect("a fit");
        assert!(fit > 55.0 && fit < 70.0, "{fit}");
        let quality = card.quality.expect("a quality");
        assert!(quality > 50.0 && quality < 80.0, "{quality}");
        // F3 has no embeddings here and the posting asks for no study: nine of
        // eleven factors answered, and the card says so.
        assert_eq!(card.factors.iter().filter(|f| f.score.is_some()).count(), 9);
        assert_eq!(card.confidence, Confidence::Medium);
        assert_eq!(card.verdict, Verdict::from_score(card.rank));
    }

    #[test]
    fn a_missing_factor_hands_its_weight_to_the_ones_that_answered() {
        let fixture = Fixture::section_9();
        let card = score(&fixture.inputs()).expect("a card");
        let semantic = card.factor(Factor::Semantic).expect("F3 is reported");
        assert_eq!(semantic.score, None);
        assert_eq!(semantic.effective_weight, 0.0);
        assert_eq!(
            semantic.weight, 0.15,
            "the configured weight is still shown"
        );

        for axis in [Axis::Fit, Axis::Quality] {
            let share: f32 = card
                .factors
                .iter()
                .filter(|factor| factor.factor.axis() == axis)
                .map(|factor| factor.effective_weight)
                .sum();
            assert!((share - 1.0).abs() < 1e-4, "{axis:?} {share}");
        }
    }

    #[test]
    fn a_card_that_could_not_see_much_is_a_lead() {
        let fixture = Fixture::section_9();
        let card = score(&fixture.inputs()).expect("a card");
        assert!(!card.is_lead(), "ten of eleven factors is not a lead");

        let mut blind = Fixture::section_9();
        blind.features = PostingFeatures::new("pst_01SECTION9", 3);
        blind.profile = ProfileSnapshot::empty();
        blind.market = MarketData::default();
        let card = score(&blind.inputs()).expect("a card");
        assert!(card.is_lead(), "{:?}", card.confidence);
    }

    #[test]
    fn a_card_id_is_a_prefixed_ulid() {
        let id = new_card_id();
        assert!(id.starts_with("sc_"), "{id}");
        assert_eq!(id.len(), 3 + 26, "{id}");
        assert_ne!(id, new_card_id());
    }

    #[test]
    fn a_stored_row_carries_the_versions_it_was_computed_for() {
        let fixture = Fixture::section_9();
        let card = score(&fixture.inputs()).expect("a card");
        let row = ScoreCardRow::new(
            "sc_01",
            "pst_01SECTION9",
            CardVersions {
                profile_version: 3,
                prefs_version: 2,
                model_version: MODEL_VERSION,
            },
            card,
            GateReport {
                results: Vec::new(),
            },
            "2026-10-16T00:00:00Z",
        );
        assert_eq!(
            row.versions(),
            CardVersions {
                profile_version: 3,
                prefs_version: 2,
                model_version: MODEL_VERSION,
            }
        );
        assert_eq!(row.gates.verdict(), GateVerdict::Pass);
        assert_eq!(default_weights(&Prefs::default()), FactorWeights::DEFAULT);
    }
}
