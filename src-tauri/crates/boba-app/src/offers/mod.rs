//! The Inbox read model (S6.4, FR-63, FR-64).
//!
//! A use case over two ports — the posting owner ([`IngestionRepo`], `posting`
//! and `posting_source`) and the scoring owner ([`ScoringRepo`], `score_card`) —
//! that joins them into **one card per Offer**: the posting as S4.2 normalised
//! it, the newest ScoreCard it has (S6.3), the Sources dedup merged into it
//! (AD-19), the badges its Gates earned (S6.1) and the Verdict that follows from
//! it.
//!
//! What is decided here and what is not:
//!
//! * **Here:** what a card *is*. The Verdict and Confidence are the card's own
//!   (never recomputed), a `VETO` carries its Gate and its quote (FR-54), a Low
//!   Confidence card is a `lead` (FR-57), and the badge ids are derived from the
//!   Gate report. Nothing is imputed: an Offer with no card has no scores, no
//!   Verdict and no confidence at all (AD-8).
//! * **In the screen:** the **sections** (Strong · Good · Moderate · Weak/Poor ·
//!   Closing soon · Excluded), the **filters** (Source, work mode, language,
//!   salary known, Verdict) and the **sort** (date, Fit, Quality). Those are
//!   ways of showing the same list, so they live where the list is drawn —
//!   `src/features/inbox/list` — and not twice, in two languages.
//!
//! The posting side is read a page at a time ([`OffersInbox::cards`]): the
//! screen says how much of the list it wants. The card and its Sources are two
//! reads per Offer, which a local SQLite answers without a network round-trip;
//! a joined query would be a second owner of the scoring table for no gain.

use std::sync::Arc;

use boba_domain::apply::channel;
use boba_domain::posting::{Offer, PayPeriod, Provenance};
use boba_domain::scoring::factors::Axis;
use boba_domain::scoring::gates::{
    Confidence as GateConfidence, Evidence, GateKind, GateReason, GateReport,
    Verdict as GateVerdict,
};
use boba_domain::scoring::rank::ScoreCardRow;

use crate::apply::manual::ChannelView;
use crate::narration::{Narration, NarrationError, NarrationView};
use crate::ports::repo::ingestion::{IngestionRepo, IngestionRepoError};
use crate::ports::repo::scoring::{ScoringRepo, ScoringRepoError};

/// One badge on an Offer card (FR-60, FR-64): a stable id the UI resolves to a
/// label in the active language. The Gate-driven badges are decided here because
/// only the core holds the Gate report (S6.1); the ghost and recruitment-fraud
/// badges S6.8 owns are appended to the same list when that slice lands.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct BadgeView {
    /// Dotted id, e.g. `closing_soon`, `lead`, `below_floor`.
    pub id: String,
    /// The Gate it came from, when it came from one (`eligibility` … `deadline`).
    pub gate: Option<String>,
}

/// Why an Offer sits in Excluded (FR-54, EXPERIENCE §9.11): the Gate that vetoed
/// it, its reason, and the posting's own words when the Gate found any.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct ExclusionView {
    /// The Gate that vetoed: `eligibility` … `deadline`.
    pub gate: String,
    /// The Gate's reason id, e.g. `requirement_not_met`, `below_floor`.
    pub reason: String,
    /// The quoted span, plain text: it is the posting's untrusted words (AD-7)
    /// and is rendered as text, never as markup.
    pub quote: Option<String>,
}

/// One Offer as the Inbox lists it (FR-63/FR-64).
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct OfferCardView {
    pub posting_id: String,
    pub title: String,
    /// Empty when no source named one — never invented.
    pub company: String,
    pub locations: Vec<String>,
    /// `remote` | `hybrid` | `onsite` | `unknown`.
    pub work_mode: String,
    /// BCP-47 content language of the posting.
    pub lang: String,
    /// Pay in minor units with its code and period, so the UI formats it
    /// (AD-17). A pay amount is nowhere near 2^53 minor units, so a JS number is
    /// exact here and specta refuses an `i64`.
    pub amount_minor: Option<f64>,
    pub currency: Option<String>,
    /// `year` | `month` | `hour`.
    pub pay_period: Option<String>,
    /// The Sources this Offer was seen on, each once (AD-19: dedup merges
    /// Offers, it never deletes provenance).
    pub sources: Vec<String>,
    /// UTC RFC 3339 (AD-17). The UI computes the age from these with the user's
    /// own clock, which is the only one that makes "2d" honest.
    pub first_seen: String,
    pub posted_at: Option<String>,
    pub deadline: Option<String>,
    /// The Fit axis (FR-55), `None` when the Offer has no card.
    pub fit: Option<f32>,
    /// The Quality axis (FR-56), `None` when the Offer has no card.
    pub quality: Option<f32>,
    /// The Rank (FR-57), `None` when the Offer has no card.
    pub rank: Option<f32>,
    /// `strong` | `good` | `moderate` | `weak` | `poor`; `None` when the Offer
    /// has no card.
    pub verdict: Option<String>,
    /// `low` | `medium` | `high`; `None` when the Offer has no card.
    pub confidence: Option<String>,
    /// FR-57: a card the model could barely see is a **lead**, not a judgement.
    /// The UI shows its Verdict dashed, as "Lead".
    pub lead: bool,
    /// FR-60/FR-64 badges, in Gate order.
    pub badges: Vec<BadgeView>,
    /// The Gate that vetoed this Offer (FR-54), `None` when none did.
    pub exclusion: Option<ExclusionView>,
    /// FR-52/§9.3: the Deadline Gate says the posting closes soon, so the card
    /// belongs in the Closing soon section as well as in its band.
    pub closing_soon: bool,
}

/// A failure the screen shows through a `message_key` (AD-3).
#[derive(Debug, Clone, thiserror::Error)]
pub enum OffersError {
    /// The postings or their Sources could not be read.
    #[error("could not read the Offers")]
    Postings(#[from] IngestionRepoError),
    /// A ScoreCard could not be read.
    #[error("could not read the Scores")]
    Scores(#[from] ScoringRepoError),
    /// The narration bound to that card could not be read.
    #[error("could not read the narration")]
    Narration(#[from] NarrationError),
    /// No Offer has this id (a stale link, or one already deleted).
    #[error("no Offer has that id")]
    NotFound,
}

impl OffersError {
    /// The i18n key the UI shows (AD-3: keys cross IPC, never raw text).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Postings(_) => "errors.offers.inbox",
            Self::Scores(_) => "errors.scoring.store",
            Self::Narration(error) => error.message_key(),
            Self::NotFound => "errors.offers.not_found",
        }
    }
}

/// The Inbox: every Offer with its newest ScoreCard, as a card.
pub struct OffersInbox {
    postings: Arc<dyn IngestionRepo>,
    scoring: Arc<dyn ScoringRepo>,
}

impl OffersInbox {
    /// The read model over the posting owner and the scoring owner (ADR-012:
    /// both arrive as mounted ports).
    pub fn new(postings: Arc<dyn IngestionRepo>, scoring: Arc<dyn ScoringRepo>) -> Self {
        Self { postings, scoring }
    }

    /// One page of cards, newest first (AC1/AC2).
    pub fn cards(&self, limit: usize, offset: usize) -> Result<Vec<OfferCardView>, OffersError> {
        let offers = self.postings.offers(limit, offset)?;
        // The Sources and the newest card of the whole page, in one read each
        // (S5.2: the N+1 follow-up — two queries per page, not two per Offer).
        let ids: Vec<String> = offers.iter().map(|offer| offer.id.clone()).collect();
        let sources = self.postings.offer_sources_many(&ids)?;
        let cards = self.scoring.latest_cards(&ids)?;
        Ok(offers
            .into_iter()
            .map(|offer| {
                let sources = sources.get(&offer.id).cloned().unwrap_or_default();
                let card = cards.get(&offer.id);
                card_view(&offer, card, sources)
            })
            .collect())
    }

    /// One Offer in full (FR-65, EXPERIENCE §9.4): the sanitised posting, the
    /// Gates with their quotes, the factor breakdown, the Sources and the other
    /// places dedup merged into it, the Apply Channels and — when the agent has
    /// written one — the Explanation in the UI language `lang`.
    ///
    /// A missing narration is `None`, never a template (FR-59 is the agent's
    /// words, and inventing them would defeat the point of checking them).
    pub fn detail(&self, posting_id: &str, lang: &str) -> Result<OfferDetailView, OffersError> {
        let offer = self
            .postings
            .get_offer(posting_id)?
            .ok_or(OffersError::NotFound)?;
        let sources = self.postings.offer_sources(posting_id)?;
        let provenance = self.postings.provenance(posting_id)?;
        let card = self.scoring.latest_card(posting_id)?;
        let narration = Narration::new(self.scoring.clone()).latest(posting_id, lang)?;
        Ok(detail_view(
            &offer,
            card.as_ref(),
            sources,
            provenance,
            narration,
        ))
    }
}

/// One Gate as the detail shows it (FR-65, EXPERIENCE §9.4): the verdict in
/// words and icons, and the posting's own quote when the Gate read one.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct GateView {
    /// `eligibility` … `deadline`.
    pub gate: String,
    /// `pass` | `flag` | `veto`.
    pub verdict: String,
    /// The reason id, e.g. `requirement_not_met`, `below_floor`.
    pub reason: String,
    /// `low` | `medium` | `high`.
    pub confidence: String,
    /// The posting's own words, rendered as text (AD-7).
    pub quote: Option<String>,
    /// The structured field the Gate read, when it read one instead of words.
    pub field: Option<String>,
}

/// One piece of evidence (FR-53): a quoted span or a structured field name.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct EvidenceView {
    /// `quote` | `field`.
    pub kind: String,
    /// The quote's text, or the field's name.
    pub text: String,
}

/// One factor of the breakdown (FR-55/FR-56, EXPERIENCE §9.4): its score, its
/// share, what it found for and against, and where it read it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct FactorView {
    /// The factor id (`skills` … `alignment`).
    pub factor: String,
    /// `fit` | `quality` — which group the axis scrolls to (AC3).
    pub axis: String,
    pub score: Option<f32>,
    pub weight: f32,
    pub effective_weight: f32,
    pub matched: Vec<String>,
    pub missing: Vec<String>,
    pub evidence: Vec<EvidenceView>,
    /// `low` | `medium` | `high`.
    pub confidence: String,
}

/// Another place the same Offer was seen: one provenance row dedup merged into
/// the posting (AD-19). It is shown, never deleted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct DuplicateView {
    pub source: String,
    pub external_id: String,
    pub url: String,
    pub fetched_at: String,
}

/// One Offer in full (FR-65). Everything here is the core's; the screen decides
/// only how to draw it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, specta::Type)]
pub struct OfferDetailView {
    pub posting_id: String,
    pub title: String,
    pub company: String,
    pub locations: Vec<String>,
    pub work_mode: String,
    /// BCP-47 content language of the posting.
    pub lang: String,
    pub amount_minor: Option<f64>,
    pub currency: Option<String>,
    pub pay_period: Option<String>,
    pub first_seen: String,
    pub posted_at: Option<String>,
    pub deadline: Option<String>,
    /// Sanitised HTML (AD-7) — the only form the detail may render, and it is
    /// rendered without loading anything remote.
    pub description_html: String,
    pub requirements: Vec<String>,
    /// The posting's own URL (https-only since S4.5/S4.6).
    pub apply_url: String,
    pub apply_email: Option<String>,
    pub sources: Vec<String>,
    /// The other places this Offer was seen (AD-19: dedup merges, never drops).
    pub duplicates: Vec<DuplicateView>,
    /// The Apply Channels the posting allows (FR-81), in the apply bar's order.
    pub channels: Vec<ChannelView>,
    pub fit: Option<f32>,
    pub quality: Option<f32>,
    pub rank: Option<f32>,
    pub verdict: Option<String>,
    pub confidence: Option<String>,
    pub lead: bool,
    /// The five Gates in report order (FR-53), empty when the Offer has no card.
    pub gates: Vec<GateView>,
    /// The eleven factors, Fit then Quality (FR-55/FR-56).
    pub factors: Vec<FactorView>,
    /// FR-59's Explanation in the requested UI language, `None` when the agent
    /// has not written one.
    pub narration: Option<NarrationView>,
}

/// One Offer in full, from the pieces the use case read (FR-65).
pub fn detail_view(
    offer: &Offer,
    card: Option<&ScoreCardRow>,
    sources: Vec<String>,
    provenance: Vec<Provenance>,
    narration: Option<NarrationView>,
) -> OfferDetailView {
    let money = offer.money.as_ref();
    let own = card.map(|row| &row.card);
    let gates = card
        .map(|row| row.gates.results.iter().map(gate_view).collect())
        .unwrap_or_default();
    let factors = own
        .map(|card| card.factors.iter().map(factor_view).collect())
        .unwrap_or_default();
    OfferDetailView {
        posting_id: offer.id.clone(),
        title: offer.title.clone(),
        company: offer.company.clone(),
        locations: offer.locations.clone(),
        work_mode: offer.work_mode.as_str().to_string(),
        lang: offer.lang.clone(),
        amount_minor: money.map(|money| money.amount_minor as f64),
        currency: money.map(|money| money.currency.clone()),
        pay_period: money.map(|money| pay_period(money.period).to_string()),
        first_seen: offer.first_seen.clone(),
        posted_at: offer.posted_at.clone(),
        deadline: offer.deadline.clone(),
        description_html: offer.description_html.clone(),
        requirements: offer.requirements.clone(),
        apply_url: offer.apply.url.clone(),
        apply_email: offer.apply.email.clone(),
        sources,
        duplicates: duplicates(offer, provenance),
        channels: channel::detect(offer)
            .into_iter()
            .map(ChannelView::from)
            .collect(),
        fit: own.and_then(|card| card.fit),
        quality: own.and_then(|card| card.quality),
        rank: own.map(|card| card.rank),
        verdict: own.map(|card| card.verdict.as_str().to_string()),
        confidence: own.map(|card| confidence(card.confidence).to_string()),
        lead: own.is_some_and(|card| card.is_lead()),
        gates,
        factors,
        narration,
    }
}

/// The provenance rows that came from somewhere other than the Offer's own
/// source: the copies dedup merged into this posting (AD-19).
fn duplicates(offer: &Offer, provenance: Vec<Provenance>) -> Vec<DuplicateView> {
    provenance
        .into_iter()
        .filter(|row| !(row.source_id == offer.source_id && row.external_id == offer.external_id))
        .map(|row| DuplicateView {
            source: row.source_id,
            external_id: row.external_id,
            url: row.url,
            fetched_at: row.fetched_at,
        })
        .collect()
}

fn gate_view(result: &boba_domain::scoring::gates::GateResult) -> GateView {
    let (quote, field) = match &result.evidence {
        Some(Evidence::Quoted(span)) => (Some(span.text.clone()), None),
        Some(Evidence::Field(name)) => (None, Some(name.clone())),
        None => (None, None),
    };
    GateView {
        gate: result.gate.as_str().to_string(),
        verdict: match result.verdict {
            GateVerdict::Pass => "pass",
            GateVerdict::Flag => "flag",
            GateVerdict::Veto => "veto",
        }
        .to_string(),
        reason: reason_id(result.reason).to_string(),
        confidence: gate_confidence(result.confidence).to_string(),
        quote,
        field,
    }
}

fn gate_confidence(confidence: GateConfidence) -> &'static str {
    match confidence {
        GateConfidence::Low => "low",
        GateConfidence::Medium => "medium",
        GateConfidence::High => "high",
    }
}

fn factor_view(score: &boba_domain::scoring::factors::FactorScore) -> FactorView {
    FactorView {
        factor: score.factor.as_str().to_string(),
        axis: match score.factor.axis() {
            Axis::Fit => "fit",
            Axis::Quality => "quality",
        }
        .to_string(),
        score: score.score,
        weight: score.weight,
        effective_weight: score.effective_weight,
        matched: score.matched.clone(),
        missing: score.missing.clone(),
        evidence: score
            .evidence
            .iter()
            .map(|evidence| match evidence {
                Evidence::Quoted(span) => EvidenceView {
                    kind: "quote".to_string(),
                    text: span.text.clone(),
                },
                Evidence::Field(name) => EvidenceView {
                    kind: "field".to_string(),
                    text: name.clone(),
                },
            })
            .collect(),
        confidence: gate_confidence(score.confidence).to_string(),
    }
}

/// One Offer as a card (FR-64): what the core knows, and nothing it does not.
pub fn card_view(
    offer: &Offer,
    card: Option<&ScoreCardRow>,
    sources: Vec<String>,
) -> OfferCardView {
    let money = offer.money.as_ref();
    OfferCardView {
        posting_id: offer.id.clone(),
        title: offer.title.clone(),
        company: offer.company.clone(),
        locations: offer.locations.clone(),
        work_mode: offer.work_mode.as_str().to_string(),
        lang: offer.lang.clone(),
        amount_minor: money.map(|money| money.amount_minor as f64),
        currency: money.map(|money| money.currency.clone()),
        pay_period: money.map(|money| pay_period(money.period).to_string()),
        sources,
        first_seen: offer.first_seen.clone(),
        posted_at: offer.posted_at.clone(),
        deadline: offer.deadline.clone(),
        fit: card.and_then(|row| row.card.fit),
        quality: card.and_then(|row| row.card.quality),
        rank: card.map(|row| row.card.rank),
        verdict: card.map(|row| row.card.verdict.as_str().to_string()),
        confidence: card.map(|row| confidence(row.card.confidence).to_string()),
        lead: card.is_some_and(|row| row.card.is_lead()),
        badges: badges(card),
        exclusion: card.and_then(|row| exclusion(&row.gates)),
        closing_soon: card.is_some_and(|row| {
            row.gates
                .gate(GateKind::Deadline)
                .is_some_and(|result| result.reason == GateReason::ClosingSoon)
        }),
    }
}

/// The stored form of a pay period (AD-17); the UI formats it.
fn pay_period(period: PayPeriod) -> &'static str {
    period.as_str()
}

/// The card's own confidence, as stored.
fn confidence(confidence: GateConfidence) -> &'static str {
    match confidence {
        GateConfidence::Low => "low",
        GateConfidence::Medium => "medium",
        GateConfidence::High => "high",
    }
}

/// FR-54: the one Gate that vetoed this Offer, with its quote. A `VETO` is never
/// silently dropped — the Excluded section shows it with the words it read.
fn exclusion(gates: &GateReport) -> Option<ExclusionView> {
    let result = gates
        .results
        .iter()
        .find(|result| result.verdict == GateVerdict::Veto)?;
    Some(ExclusionView {
        gate: result.gate.as_str().to_string(),
        reason: reason_id(result.reason),
        quote: match &result.evidence {
            // The posting's own words: untrusted text, shown as text (AD-7).
            Some(Evidence::Quoted(span)) => Some(span.text.clone()),
            Some(Evidence::Field(_)) | None => None,
        },
    })
}

/// The badges a card shows (FR-60/FR-64): one per Gate that told the user
/// something, plus FR-57's Lead. Ordered by Gate, deduplicated.
fn badges(card: Option<&ScoreCardRow>) -> Vec<BadgeView> {
    let Some(card) = card else {
        return Vec::new();
    };
    let mut badges: Vec<BadgeView> = Vec::new();
    for result in &card.gates.results {
        // A Gate that passed says nothing worth a badge.
        if result.verdict == GateVerdict::Pass {
            continue;
        }
        let id = badge_id(result.gate, result.reason);
        if badges.iter().any(|badge| badge.id == id) {
            continue;
        }
        badges.push(BadgeView {
            id: id.to_string(),
            gate: Some(result.gate.as_str().to_string()),
        });
    }
    // FR-57: the card could barely see the Offer, so it is a lead. The Verdict
    // itself is also drawn as "Lead"; the badge is what a filtered or collapsed
    // list reads.
    if card.card.is_lead() {
        badges.push(BadgeView {
            id: "lead".to_string(),
            gate: None,
        });
    }
    badges
}

/// One badge id per (Gate, reason) the UI has a label for.
fn badge_id(gate: GateKind, reason: GateReason) -> &'static str {
    use GateReason as R;
    match (gate, reason) {
        (GateKind::Eligibility, R::RequirementNotMet) => "eligibility_not_met",
        (GateKind::Eligibility, R::Unverified) => "eligibility_unverified",
        (GateKind::Language, R::BelowRequiredLevel) => "language_level",
        (GateKind::Language, R::Unverified) => "language_unverified",
        (GateKind::Location, R::RequirementNotMet) => "location_not_met",
        (GateKind::Location, R::Unverified) => "location_unverified",
        (GateKind::Compensation, R::BelowFloor) => "below_floor",
        (GateKind::Compensation, R::Unverified) => "compensation_unverified",
        (GateKind::Deadline, R::ClosingSoon) => "closing_soon",
        (GateKind::Deadline, R::Expired) => "expired",
        // A Gate can only report the reasons its own kind reaches; anything else
        // is a flag the UI shows as a plain "check this" badge.
        _ => "check",
    }
}

/// The Gate reason's id, for the Excluded card's line (FR-54). Spelled out
/// rather than derived from serde, so a reason the UI has no label for is a
/// compile error here instead of an empty line on the screen.
fn reason_id(reason: GateReason) -> String {
    use GateReason as R;
    let id = match reason {
        R::Satisfied => "satisfied",
        R::Unverified => "unverified",
        R::RequirementNotMet => "requirement_not_met",
        R::BelowRequiredLevel => "below_required_level",
        R::BelowFloor => "below_floor",
        R::ClosingSoon => "closing_soon",
        R::Expired => "expired",
    };
    id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use boba_domain::posting::{ApplyHints, Money, WorkMode};
    use boba_domain::scoring::gates::{QuotedSpan, Verdict as Veto};
    use boba_domain::scoring::narration::ScoreNarration;
    use boba_domain::scoring::rank::{CardVersions, ScoreCard, Verdict as Band};

    /// An in-memory `ScoringRepo`: this module only reads the newest card, so
    /// the fake stores rows and hands back the last one written for a posting —
    /// the store's `ORDER BY created_at DESC` rule.
    #[derive(Default)]
    struct FakeScoringRepo {
        rows: Mutex<Vec<ScoreCardRow>>,
        narrations: Mutex<Vec<ScoreNarration>>,
    }

    impl FakeScoringRepo {
        fn insert_card(&self, row: &ScoreCardRow) -> Result<(), ScoringRepoError> {
            self.rows.lock().unwrap().push(row.clone());
            Ok(())
        }
    }

    impl ScoringRepo for FakeScoringRepo {
        fn insert_card(&self, row: &ScoreCardRow) -> Result<(), ScoringRepoError> {
            FakeScoringRepo::insert_card(self, row)
        }

        fn card(
            &self,
            posting_id: &str,
            versions: CardVersions,
        ) -> Result<Option<ScoreCardRow>, ScoringRepoError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .find(|row| row.posting_id == posting_id && row.versions() == versions)
                .cloned())
        }

        fn latest_card(&self, posting_id: &str) -> Result<Option<ScoreCardRow>, ScoringRepoError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .rfind(|row| row.posting_id == posting_id)
                .cloned())
        }

        fn cards_for(&self, posting_id: &str) -> Result<Vec<ScoreCardRow>, ScoringRepoError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|row| row.posting_id == posting_id)
                .cloned()
                .collect())
        }

        fn latest_narration(
            &self,
            posting_id: &str,
            score_card_id: &str,
            lang: &str,
        ) -> Result<Option<ScoreNarration>, ScoringRepoError> {
            Ok(self
                .narrations
                .lock()
                .unwrap()
                .iter()
                .filter(|row| {
                    row.posting_id == posting_id
                        && row.score_card_id == score_card_id
                        && row.lang == lang
                })
                .max_by(|left, right| left.created_at.cmp(&right.created_at))
                .cloned())
        }

        fn insert_narration(&self, row: &ScoreNarration) -> Result<(), ScoringRepoError> {
            self.narrations.lock().unwrap().push(row.clone());
            Ok(())
        }
    }

    /// A posting repo whose single-Offer reads panic: the Inbox must read a
    /// page with the batched methods (S5.2's N+1 follow-up), and a regression to
    /// per-Offer reads fails loudly instead of only being slow.
    struct BatchedPostings {
        offers: Vec<Offer>,
    }

    impl IngestionRepo for BatchedPostings {
        fn insert_offer(
            &self,
            _offer: &Offer,
            _provenance: &Provenance,
        ) -> Result<(), IngestionRepoError> {
            unreachable!("the read model does not write")
        }

        fn get_offer(&self, _id: &str) -> Result<Option<Offer>, IngestionRepoError> {
            unreachable!("the read model reads a page")
        }

        fn offers(&self, limit: usize, offset: usize) -> Result<Vec<Offer>, IngestionRepoError> {
            Ok(self
                .offers
                .iter()
                .skip(offset)
                .take(limit)
                .cloned()
                .collect())
        }

        fn offer_sources(&self, _posting_id: &str) -> Result<Vec<String>, IngestionRepoError> {
            panic!("the Inbox must not read Sources one Offer at a time")
        }

        fn offer_sources_many(
            &self,
            posting_ids: &[String],
        ) -> Result<std::collections::HashMap<String, Vec<String>>, IngestionRepoError> {
            Ok(posting_ids
                .iter()
                .map(|id| (id.clone(), vec!["remotive".to_string()]))
                .collect())
        }

        fn provenance(&self, _posting_id: &str) -> Result<Vec<Provenance>, IngestionRepoError> {
            unreachable!("the card does not read provenance")
        }

        fn dedup_candidates(
            &self,
            _probe: &boba_domain::dedup::DedupProbe,
        ) -> Result<Vec<Offer>, IngestionRepoError> {
            unreachable!("the read model does not dedup")
        }

        fn merge_provenance(
            &self,
            _posting_id: &str,
            _provenance: &Provenance,
            _now: &str,
        ) -> Result<(), IngestionRepoError> {
            unreachable!("the read model does not write")
        }

        fn features(
            &self,
            _posting_id: &str,
            _profile_version: u32,
        ) -> Result<Option<boba_domain::posting_features::PostingFeatures>, IngestionRepoError>
        {
            unreachable!("the card does not read features")
        }

        fn features_agent_done(
            &self,
            _posting_id: &str,
            _profile_version: u32,
        ) -> Result<bool, IngestionRepoError> {
            unreachable!("the card does not read features")
        }

        fn save_deterministic_features(
            &self,
            _features: &boba_domain::posting_features::PostingFeatures,
            _now: &str,
        ) -> Result<(), IngestionRepoError> {
            unreachable!("the read model does not write")
        }

        fn save_agent_features(
            &self,
            _features: &boba_domain::posting_features::PostingFeatures,
            _now: &str,
        ) -> Result<(), IngestionRepoError> {
            unreachable!("the read model does not write")
        }

        fn features_pending(
            &self,
            _profile_version: u32,
            _limit: usize,
        ) -> Result<Vec<boba_domain::posting_features::PendingPosting>, IngestionRepoError>
        {
            unreachable!("the card does not read features")
        }

        fn lifecycle(
            &self,
            _posting_id: &str,
        ) -> Result<Option<boba_domain::posting::lifecycle::PostingLifecycle>, IngestionRepoError>
        {
            unreachable!("the card does not read lifecycle")
        }

        fn save_lifecycle(
            &self,
            _lifecycle: &boba_domain::posting::lifecycle::PostingLifecycle,
        ) -> Result<(), IngestionRepoError> {
            unreachable!("the read model does not write")
        }

        fn postings_missing_from_run(
            &self,
            _source_ids: &[String],
            _before: &str,
        ) -> Result<Vec<Offer>, IngestionRepoError> {
            unreachable!("the read model does not sweep")
        }

        fn postings_seen_by_run(
            &self,
            _source_ids: &[String],
            _since: &str,
        ) -> Result<Vec<Offer>, IngestionRepoError> {
            unreachable!("the read model does not sweep")
        }

        fn possibly_closed_with_applications(
            &self,
        ) -> Result<Vec<boba_domain::posting::lifecycle::ClosedWarning>, IngestionRepoError>
        {
            unreachable!("the read model does not sweep")
        }
    }

    /// The same rule for the card side: newest cards are read in one call.
    #[derive(Default)]
    struct BatchedScoring {
        rows: Mutex<Vec<ScoreCardRow>>,
    }

    impl ScoringRepo for BatchedScoring {
        fn insert_card(&self, _row: &ScoreCardRow) -> Result<(), ScoringRepoError> {
            unreachable!("the read model does not write")
        }

        fn card(
            &self,
            _posting_id: &str,
            _versions: CardVersions,
        ) -> Result<Option<ScoreCardRow>, ScoringRepoError> {
            unreachable!("the read model reads the newest card")
        }

        fn latest_card(&self, _posting_id: &str) -> Result<Option<ScoreCardRow>, ScoringRepoError> {
            panic!("the Inbox must not read cards one Offer at a time")
        }

        fn latest_cards(
            &self,
            posting_ids: &[String],
        ) -> Result<std::collections::HashMap<String, ScoreCardRow>, ScoringRepoError> {
            Ok(self
                .rows
                .lock()
                .unwrap()
                .iter()
                .filter(|row| posting_ids.contains(&row.posting_id))
                .map(|row| (row.posting_id.clone(), row.clone()))
                .collect())
        }

        fn cards_for(&self, _posting_id: &str) -> Result<Vec<ScoreCardRow>, ScoringRepoError> {
            unreachable!("the card does not read history")
        }

        fn latest_narration(
            &self,
            _posting_id: &str,
            _score_card_id: &str,
            _lang: &str,
        ) -> Result<Option<ScoreNarration>, ScoringRepoError> {
            unreachable!("the card does not read narration")
        }

        fn insert_narration(&self, _row: &ScoreNarration) -> Result<(), ScoringRepoError> {
            unreachable!("the read model does not write")
        }
    }

    // AC1/AC2 (S5.2 follow-up): one page is two reads, whatever its size.
    #[test]
    fn a_page_of_cards_reads_its_sources_and_scores_in_one_call_each() {
        let mut second = offer();
        second.id = "pst_02".to_string();
        let postings = BatchedPostings {
            offers: vec![offer(), second],
        };
        let scoring = BatchedScoring::default();
        let row = card(Vec::new(), GateConfidence::Medium, Some(81.3));
        scoring.rows.lock().unwrap().push(row);

        let inbox = OffersInbox::new(Arc::new(postings), Arc::new(scoring));
        let cards = inbox.cards(500, 0).expect("read");

        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].sources, vec!["remotive".to_string()]);
        assert_eq!(cards[0].fit, Some(81.3));
        assert_eq!(cards[1].fit, None, "an unscored Offer has no scores");
    }

    fn offer() -> Offer {
        Offer {
            id: "pst_01".to_string(),
            source_id: "greenhouse".to_string(),
            external_id: "gh-42".to_string(),
            title: "Senior Data Engineer".to_string(),
            company: "Boba Labs".to_string(),
            locations: vec!["Remote, Spain".to_string()],
            work_mode: WorkMode::Remote,
            money: Some(Money {
                amount_minor: 5_500_000,
                currency: "EUR".to_string(),
                period: PayPeriod::Year,
            }),
            posted_at: Some("2026-10-04T00:00:00Z".to_string()),
            deadline: Some("2026-10-20T00:00:00Z".to_string()),
            first_seen: "2026-10-04T00:00:00Z".to_string(),
            last_seen: "2026-10-16T09:00:00Z".to_string(),
            lang: "en".to_string(),
            description_html: "<p>dbt</p>".to_string(),
            description_text: "dbt and Airflow".to_string(),
            requirements: vec!["dbt".to_string()],
            apply: ApplyHints {
                url: "https://boards.example.com/boba/gh-42".to_string(),
                email: None,
            },
        }
    }

    fn card(gates: Vec<GateResult>, confidence: GateConfidence, fit: Option<f32>) -> ScoreCardRow {
        ScoreCardRow::new(
            "sc_01",
            "pst_01",
            CardVersions {
                profile_version: 3,
                prefs_version: 2,
                model_version: 1,
            },
            ScoreCard {
                model_version: 1,
                factors: Vec::new(),
                fit,
                quality: Some(71.0),
                rank: 78.0,
                verdict: Band::Strong,
                confidence,
            },
            GateReport { results: gates },
            "2026-10-16T09:00:00Z",
        )
    }

    use boba_domain::scoring::gates::GateResult;

    fn gate(
        gate: GateKind,
        verdict: Veto,
        reason: GateReason,
        evidence: Option<Evidence>,
    ) -> GateResult {
        GateResult {
            gate,
            verdict,
            reason,
            evidence,
            confidence: GateConfidence::Medium,
        }
    }

    // AC2: the card carries what FR-64 lists, in the core's own vocabulary.
    #[test]
    fn a_card_carries_the_offer_the_scores_and_the_sources() {
        let row = card(Vec::new(), GateConfidence::Medium, Some(81.3));
        let view = card_view(
            &offer(),
            Some(&row),
            vec!["greenhouse".to_string(), "rss".to_string()],
        );

        assert_eq!(view.posting_id, "pst_01");
        assert_eq!(view.title, "Senior Data Engineer");
        assert_eq!(view.company, "Boba Labs");
        assert_eq!(view.locations, vec!["Remote, Spain".to_string()]);
        assert_eq!(view.work_mode, "remote");
        assert_eq!(view.lang, "en");
        assert_eq!(view.amount_minor, Some(5_500_000.0));
        assert_eq!(view.currency.as_deref(), Some("EUR"));
        assert_eq!(view.pay_period.as_deref(), Some("year"));
        assert_eq!(view.sources, vec!["greenhouse", "rss"]);
        assert_eq!(view.fit, Some(81.3));
        assert_eq!(view.quality, Some(71.0));
        assert_eq!(view.rank, Some(78.0));
        assert_eq!(view.verdict.as_deref(), Some("strong"));
        assert_eq!(view.confidence.as_deref(), Some("medium"));
        assert!(!view.lead);
        assert!(view.badges.is_empty());
        assert!(view.exclusion.is_none());
        assert!(!view.closing_soon);
        // The age is the UI's to compute (AD-17: the user's own clock).
        assert_eq!(view.posted_at.as_deref(), Some("2026-10-04T00:00:00Z"));
    }

    // AC1/AC2: an Offer nobody has scored yet is not an error and carries no band.
    #[test]
    fn an_unscored_offer_has_no_scores_and_no_verdict() {
        let view = card_view(&offer(), None, vec!["manual".to_string()]);
        assert_eq!(view.fit, None);
        assert_eq!(view.quality, None);
        assert_eq!(view.rank, None);
        assert_eq!(view.verdict, None);
        assert_eq!(view.confidence, None);
        assert!(!view.lead);
        assert!(view.badges.is_empty());
        assert!(view.exclusion.is_none());
        assert!(!view.closing_soon);
    }

    // AC2: Low Confidence is a lead (FR-57) — the UI's dashed "Lead" chip.
    #[test]
    fn a_low_confidence_card_is_a_lead() {
        let row = card(vec![], GateConfidence::Low, Some(70.0));
        let view = card_view(&offer(), Some(&row), Vec::new());
        assert!(view.lead);
        assert_eq!(view.badges.len(), 1);
        assert_eq!(view.badges[0].id, "lead");
        assert_eq!(view.badges[0].gate, None);
        // The lead badge never replaces the band: FR-64 shows both.
        assert_eq!(view.verdict.as_deref(), Some("strong"));
    }

    // AC2/§9.11: a VETO is shown with its quote and its reason, never dropped.
    #[test]
    fn a_vetoed_offer_carries_its_exclusion_and_quote() {
        let row = card(
            vec![gate(
                GateKind::Eligibility,
                Veto::Veto,
                GateReason::RequirementNotMet,
                Some(Evidence::Quoted(QuotedSpan {
                    text: "Must be authorized to work in the US.".to_string(),
                    start: 0,
                    end: 40,
                })),
            )],
            GateConfidence::High,
            Some(40.0),
        );
        let view = card_view(&offer(), Some(&row), Vec::new());
        let exclusion = view.exclusion.expect("a veto");
        assert_eq!(exclusion.gate, "eligibility");
        assert_eq!(exclusion.reason, "requirement_not_met");
        assert_eq!(
            exclusion.quote.as_deref(),
            Some("Must be authorized to work in the US.")
        );
        assert_eq!(view.badges.len(), 1);
        assert_eq!(view.badges[0].id, "eligibility_not_met");
        assert_eq!(view.badges[0].gate.as_deref(), Some("eligibility"));
    }

    // A structured field is not a quote: the Excluded card shows the reason only.
    #[test]
    fn a_field_evidence_veto_has_no_quote() {
        let row = card(
            vec![gate(
                GateKind::Compensation,
                Veto::Veto,
                GateReason::BelowFloor,
                Some(Evidence::Field("money".to_string())),
            )],
            GateConfidence::High,
            Some(40.0),
        );
        let exclusion = card_view(&offer(), Some(&row), Vec::new())
            .exclusion
            .expect("a veto");
        assert_eq!(exclusion.reason, "below_floor");
        assert_eq!(exclusion.quote, None);
    }

    // AC1: Closing soon is a Gate decision (FR-52), so the section is populated
    // by the same report the band comes from.
    #[test]
    fn the_deadline_gate_marks_an_offer_closing_soon() {
        let row = card(
            vec![gate(
                GateKind::Deadline,
                Veto::Flag,
                GateReason::ClosingSoon,
                Some(Evidence::Field("deadline".to_string())),
            )],
            GateConfidence::High,
            Some(75.0),
        );
        let view = card_view(&offer(), Some(&row), Vec::new());
        assert!(view.closing_soon);
        assert_eq!(view.badges.len(), 1);
        assert_eq!(view.badges[0].id, "closing_soon");
        assert!(view.exclusion.is_none(), "a FLAG is not a VETO");
    }

    #[test]
    fn gates_that_passed_leave_no_badge() {
        let row = card(
            vec![
                gate(
                    GateKind::Eligibility,
                    Veto::Pass,
                    GateReason::Satisfied,
                    None,
                ),
                gate(GateKind::Language, Veto::Flag, GateReason::Unverified, None),
                gate(GateKind::Location, Veto::Flag, GateReason::Unverified, None),
                gate(GateKind::Language, Veto::Flag, GateReason::Unverified, None),
            ],
            GateConfidence::Medium,
            Some(60.0),
        );
        let view = card_view(&offer(), Some(&row), Vec::new());
        let ids: Vec<&str> = view.badges.iter().map(|badge| badge.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["language_unverified", "location_unverified"],
            "gate order, no duplicates"
        );
    }

    // An Offer with no pay is not a zero: FR-64 shows salary only when known.
    #[test]
    fn an_offer_without_pay_carries_no_money_at_all() {
        let mut offer = offer();
        offer.money = None;
        let view = card_view(&offer, None, Vec::new());
        assert_eq!(view.amount_minor, None);
        assert_eq!(view.currency, None);
        assert_eq!(view.pay_period, None);
    }

    #[test]
    fn the_confidence_is_the_stored_form_the_ui_translates() {
        assert_eq!(confidence(GateConfidence::Low), "low");
        assert_eq!(confidence(GateConfidence::Medium), "medium");
        assert_eq!(confidence(GateConfidence::High), "high");
    }

    // The read model joins the two owners: the posting comes from ingestion and
    // the score from scoring, and neither is written here.
    #[test]
    fn the_inbox_joins_postings_with_their_newest_card() {
        use crate::ingest::normalize::testing::FakeIngestionRepo;
        use boba_domain::posting::Provenance;

        let postings = Arc::new(FakeIngestionRepo::default());
        let scoring = Arc::new(FakeScoringRepo::default());
        let offer = offer();
        postings
            .insert_offer(
                &offer,
                &Provenance {
                    posting_id: offer.id.clone(),
                    source_id: "greenhouse".to_string(),
                    external_id: offer.external_id.clone(),
                    url: offer.apply.url.clone(),
                    raw: "{}".to_string(),
                    fetched_at: "2026-10-04T00:00:00Z".to_string(),
                },
            )
            .expect("store the posting");
        // A second source for the same posting (AD-19: one Offer, two rows).
        postings
            .merge_provenance(
                &offer.id,
                &Provenance {
                    posting_id: offer.id.clone(),
                    source_id: "rss".to_string(),
                    external_id: "rss-7".to_string(),
                    url: "https://feed.example.com/7".to_string(),
                    raw: "{}".to_string(),
                    fetched_at: "2026-10-05T00:00:00Z".to_string(),
                },
                "2026-10-05T00:00:00Z",
            )
            .expect("merge the second source");
        scoring
            .insert_card(&card(vec![], GateConfidence::High, Some(81.3)))
            .expect("store the card");

        let inbox = OffersInbox::new(postings, scoring);
        let cards = inbox.cards(10, 0).expect("the inbox");
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].posting_id, "pst_01");
        assert_eq!(cards[0].sources, vec!["greenhouse", "rss"]);
        assert_eq!(cards[0].fit, Some(81.3));
        assert_eq!(cards[0].verdict.as_deref(), Some("strong"));
    }

    // A page is a page: the limit and the offset are the caller's.
    #[test]
    fn the_inbox_reads_one_page_at_a_time() {
        use crate::ingest::normalize::testing::FakeIngestionRepo;
        use boba_domain::posting::Provenance;

        let postings = Arc::new(FakeIngestionRepo::default());
        for (index, seen) in ["2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z"]
            .into_iter()
            .enumerate()
        {
            let mut offer = offer();
            offer.id = format!("pst_0{index}");
            offer.first_seen = seen.to_string();
            postings
                .insert_offer(
                    &offer,
                    &Provenance {
                        posting_id: offer.id.clone(),
                        source_id: "greenhouse".to_string(),
                        external_id: offer.external_id.clone(),
                        url: offer.apply.url.clone(),
                        raw: "{}".to_string(),
                        fetched_at: seen.to_string(),
                    },
                )
                .expect("store the posting");
        }
        let inbox = OffersInbox::new(postings, Arc::new(FakeScoringRepo::default()));
        let first = inbox.cards(1, 0).expect("first page");
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].posting_id, "pst_01", "newest first");
        let second = inbox.cards(1, 1).expect("second page");
        assert_eq!(second[0].posting_id, "pst_00");
        // No card yet: the page still renders, with no scores.
        assert_eq!(second[0].verdict, None);
    }

    // --- Offer detail (S6.5, FR-65) ------------------------------------------

    /// A provenance row for one copy of the same Offer.
    fn seen(source: &str, external_id: &str, url: &str) -> Provenance {
        Provenance {
            posting_id: "pst_01".to_string(),
            source_id: source.to_string(),
            external_id: external_id.to_string(),
            url: url.to_string(),
            raw: "{}".to_string(),
            fetched_at: "2026-10-04T00:00:00Z".to_string(),
        }
    }

    /// A card whose factors are the real eleven, so the breakdown is testable.
    fn scored_card(gates: Vec<GateResult>) -> ScoreCardRow {
        use boba_domain::scoring::factors::{Factor, FactorScore, FactorWeights};
        use boba_domain::scoring::gates::Confidence;
        use boba_domain::scoring::rank::{MODEL_VERSION, card as build_card};

        let factors: Vec<FactorScore> = Factor::ALL
            .into_iter()
            .map(|factor| {
                let mut score = FactorScore::scored(
                    factor,
                    80.0,
                    FactorWeights::DEFAULT.of(factor),
                    Confidence::High,
                    vec![Evidence::Quoted(QuotedSpan {
                        text: "at least five years of dbt".to_string(),
                        start: 4,
                        end: 30,
                    })],
                );
                if factor == Factor::Skills {
                    score.matched = vec!["dbt".to_string(), "Airflow".to_string()];
                    score.missing = vec!["Kubernetes".to_string()];
                }
                score
            })
            .collect();
        let card = build_card(MODEL_VERSION, factors, &FactorWeights::DEFAULT).expect("a card");
        ScoreCardRow::new(
            "sc_01",
            "pst_01",
            CardVersions {
                profile_version: 3,
                prefs_version: 2,
                model_version: MODEL_VERSION,
            },
            card,
            GateReport { results: gates },
            "2026-10-16T09:00:00Z",
        )
    }

    #[test]
    fn the_detail_shows_gates_factors_sources_duplicates_and_channels() {
        let gates = vec![
            gate(GateKind::Location, Veto::Pass, GateReason::Satisfied, None),
            gate(
                GateKind::Language,
                Veto::Flag,
                GateReason::Unverified,
                Some(Evidence::Quoted(QuotedSpan {
                    text: "English B2 required".to_string(),
                    start: 0,
                    end: 18,
                })),
            ),
        ];
        let card = scored_card(gates);
        let mut offer = offer();
        offer.apply.email = Some("jobs@boba.example".to_string());
        let view = detail_view(
            &offer,
            Some(&card),
            vec!["greenhouse".to_string(), "rss".to_string()],
            vec![
                seen(
                    "greenhouse",
                    "gh-42",
                    "https://boards.example.com/boba/gh-42",
                ),
                seen("rss", "rss-7", "https://feed.example.com/jobs/7"),
            ],
            Some(NarrationView {
                text: "You match dbt. Kubernetes is missing.".to_string(),
                lang: "en".to_string(),
                untrusted_origin: true,
            }),
        );

        // AC1: the posting and its original link.
        assert_eq!(view.title, "Senior Data Engineer");
        assert_eq!(view.description_html, "<p>dbt</p>");
        assert_eq!(view.apply_url, "https://boards.example.com/boba/gh-42");
        assert_eq!(view.apply_email.as_deref(), Some("jobs@boba.example"));

        // AC1: Gates at the top, with their quotes.
        assert_eq!(view.gates.len(), 2);
        assert_eq!(view.gates[1].gate, "language");
        assert_eq!(view.gates[1].verdict, "flag");
        assert_eq!(view.gates[1].quote.as_deref(), Some("English B2 required"));

        // AC1/AC3: the eleven factors, each with its axis and evidence.
        assert_eq!(view.factors.len(), 11);
        let skills = view
            .factors
            .iter()
            .find(|f| f.factor == "f1_skills")
            .expect("F1");
        assert_eq!(skills.axis, "fit");
        assert_eq!(skills.matched, vec!["dbt", "Airflow"]);
        assert_eq!(skills.missing, vec!["Kubernetes"]);
        assert_eq!(skills.evidence[0].kind, "quote");
        assert!(
            view.factors
                .iter()
                .any(|f| f.factor == "q1_compensation" && f.axis == "quality")
        );

        // AC1: Sources and the copies dedup merged, never the Offer's own row.
        assert_eq!(view.sources, vec!["greenhouse", "rss"]);
        assert_eq!(view.duplicates.len(), 1);
        assert_eq!(view.duplicates[0].source, "rss");

        // AC1: the detected Apply Channels, in order.
        let kinds: Vec<&str> = view
            .channels
            .iter()
            .map(|c| match c {
                ChannelView::EmailApply { .. } => "email_apply",
                ChannelView::AtsForm { .. } => "ats_form",
                ChannelView::Manual { .. } => "manual",
            })
            .collect();
        assert_eq!(kinds, vec!["email_apply", "manual"]);

        // AC2: the narration travels as plain text, marked untrusted.
        let narration = view.narration.expect("a narration");
        assert!(narration.untrusted_origin);
        assert_eq!(narration.lang, "en");
    }

    #[test]
    fn an_unscored_offer_has_no_gates_no_factors_and_no_narration() {
        let view = detail_view(
            &offer(),
            None,
            vec!["greenhouse".to_string()],
            Vec::new(),
            None,
        );
        assert!(view.gates.is_empty());
        assert!(view.factors.is_empty());
        assert_eq!(view.fit, None);
        assert_eq!(view.rank, None);
        assert!(view.narration.is_none());
        // The posting itself is still there.
        assert_eq!(view.description_html, "<p>dbt</p>");
    }

    #[test]
    fn the_service_returns_not_found_for_an_id_no_offer_has() {
        use crate::ingest::normalize::testing::FakeIngestionRepo;

        let inbox = OffersInbox::new(
            Arc::new(FakeIngestionRepo::default()),
            Arc::new(FakeScoringRepo::default()),
        );
        let error = inbox.detail("pst_missing", "en").expect_err("not found");
        assert_eq!(error.message_key(), "errors.offers.not_found");
    }

    #[test]
    fn the_service_reads_the_narration_in_the_requested_language() {
        use crate::ingest::normalize::testing::FakeIngestionRepo;

        let postings = Arc::new(FakeIngestionRepo::default());
        let offer = offer();
        postings
            .insert_offer(&offer, &seen("greenhouse", "gh-42", &offer.apply.url))
            .expect("store the offer");
        let scoring = Arc::new(FakeScoringRepo::default());
        scoring.insert_card(&scored_card(Vec::new())).expect("card");
        scoring
            .insert_narration(&ScoreNarration {
                id: "nrt_1".to_string(),
                posting_id: "pst_01".to_string(),
                score_card_id: "sc_01".to_string(),
                lang: "es".to_string(),
                text: "Encajas con dbt. Falta Kubernetes.".to_string(),
                untrusted_origin: true,
                created_at: "2026-10-16T10:00:00Z".to_string(),
            })
            .expect("narration");

        let inbox = OffersInbox::new(postings, scoring);
        let spanish = inbox.detail("pst_01", "es").expect("detail");
        assert_eq!(
            spanish.narration.expect("a narration").text,
            "Encajas con dbt. Falta Kubernetes."
        );
        let english = inbox.detail("pst_01", "en").expect("detail");
        assert!(
            english.narration.is_none(),
            "no English narration was written"
        );
    }

    /// BUG2 regression (AD-8): a re-score writes a new card, and the narration
    /// written for the old one must not be shown beside the new numbers.
    #[test]
    fn a_rescored_offer_hides_the_narration_written_for_the_old_card() {
        use crate::ingest::normalize::testing::FakeIngestionRepo;
        use crate::narration::Narration;

        let postings = Arc::new(FakeIngestionRepo::default());
        let offer = offer();
        postings
            .insert_offer(&offer, &seen("greenhouse", "gh-42", &offer.apply.url))
            .expect("store the offer");
        let scoring = Arc::new(FakeScoringRepo::default());
        let first = scored_card(Vec::new());
        scoring.insert_card(&first).expect("first card");
        Narration::new(scoring.clone())
            .save(
                "pst_01",
                "en",
                "You match dbt and Airflow. Kubernetes is missing.",
                "2026-10-16T10:00:00Z",
            )
            .expect("narration for the first card");

        let inbox = OffersInbox::new(postings.clone(), scoring.clone());
        assert!(
            inbox
                .detail("pst_01", "en")
                .expect("detail")
                .narration
                .is_some(),
            "the narration matches the card it was written for"
        );

        let mut second = first.clone();
        second.id = "sc_02".to_string();
        second.prefs_version = 9;
        second.created_at = "2026-10-17T10:00:00Z".to_string();
        scoring.insert_card(&second).expect("the re-score");

        assert!(
            inbox
                .detail("pst_01", "en")
                .expect("detail")
                .narration
                .is_none(),
            "the old narration must not describe the new card"
        );

        Narration::new(scoring.clone())
            .save(
                "pst_01",
                "en",
                "You match dbt and Airflow. Kubernetes is missing.",
                "2026-10-17T11:00:00Z",
            )
            .expect("narration for the new card");
        assert!(
            inbox
                .detail("pst_01", "en")
                .expect("detail")
                .narration
                .is_some(),
            "a narration written against the new card shows again"
        );
    }
}
