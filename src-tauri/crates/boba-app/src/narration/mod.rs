//! Explanation narration (S6.5, FR-59, AD-5, ADR-010).
//!
//! The `narrate_scores` run reads a ScoreCard and writes two to four warm,
//! honest sentences about it. This use case decides what the app keeps:
//!
//! * the text must be **grounded** in the card ([`boba_domain::scoring::
//!   narration::check`]) — every number and every skill it names already exists
//!   in the factor data, so the coach can be encouraging without being
//!   inventive;
//! * it is stored **against the card** it explains, never against the Offer at
//!   large: a re-score writes a new card, and the old narration stays with the
//!   numbers it was checked against (AD-8);
//! * it is **untrusted** text (ADR-010). It is always rendered as plain text;
//!   the UI never parses it as markup.
//!
//! There is no invented fallback: when no narration exists the detail screen
//! says so rather than showing a template that pretends the agent wrote it.

use std::sync::Arc;

use boba_domain::scoring::narration::{self, NarrationViolation, ScoreNarration, new_narration_id};

use crate::ports::repo::scoring::{ScoringRepo, ScoringRepoError};

/// The narration as the Offer detail shows it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct NarrationView {
    /// The plain text the run wrote. Untrusted (ADR-010): render as text.
    pub text: String,
    /// BCP-47 UI language the text is written in.
    pub lang: String,
    /// ADR-010: always `true` for agent-written text. Travels over IPC so the
    /// screen can mark it as the agent's words, not the core's.
    pub untrusted_origin: bool,
}

/// A failure the UI shows through a `message_key` (AD-3).
#[derive(Debug, Clone, thiserror::Error)]
pub enum NarrationError {
    /// The card or the narration could not be read or written.
    #[error(transparent)]
    Store(#[from] ScoringRepoError),
    /// There is no ScoreCard to explain yet.
    #[error("the Offer has no ScoreCard to explain")]
    NoCard,
    /// The text failed FR-59's grounding check.
    #[error(transparent)]
    Violation(#[from] NarrationViolation),
    /// The text was asked for in no language at all.
    #[error("the narration was requested for no language")]
    Language,
}

impl NarrationError {
    /// The i18n key the UI shows (AD-3: keys cross IPC, never raw text).
    pub fn message_key(&self) -> &'static str {
        match self {
            Self::Store(_) => "errors.narration.store",
            Self::NoCard => "errors.narration.no_card",
            Self::Violation(NarrationViolation::Sentences(_))
            | Self::Violation(NarrationViolation::Number(_))
            | Self::Violation(NarrationViolation::Term(_))
            | Self::Violation(NarrationViolation::Empty) => "errors.narration.ungrounded",
            Self::Language => "errors.narration.language",
        }
    }
}

/// Narrations over the scoring owner's table (AD-16).
pub struct Narration {
    scoring: Arc<dyn ScoringRepo>,
}

impl Narration {
    /// The use case over the mounted repo.
    pub fn new(scoring: Arc<dyn ScoringRepo>) -> Self {
        Self { scoring }
    }

    /// The newest narration for one Offer's **latest card** in one UI
    /// language, `None` when the agent has not written one for that card (the
    /// screen says so; it does not invent one).
    ///
    /// A re-score writes a new card, so the text written for the old one is no
    /// longer shown: it described numbers that are not on screen any more
    /// (AD-8).
    pub fn latest(
        &self,
        posting_id: &str,
        lang: &str,
    ) -> Result<Option<NarrationView>, NarrationError> {
        let lang = language(lang)?;
        let Some(card) = self.scoring.latest_card(posting_id)? else {
            return Ok(None);
        };
        Ok(self
            .scoring
            .latest_narration(posting_id, &card.id, lang)?
            .map(view))
    }

    /// Check and store the narration a run proposed (FR-59, AD-5).
    ///
    /// The latest card is the one being explained: the run was served that card
    /// by `get_score_card`, and a stale proposal cannot silently attach itself
    /// to a newer score. A text that fails grounding is refused whole — the run
    /// is asked to write it again — and nothing is stored.
    pub fn save(
        &self,
        posting_id: &str,
        lang: &str,
        text: &str,
        now: &str,
    ) -> Result<NarrationView, NarrationError> {
        let lang = language(lang)?;
        let card = self
            .scoring
            .latest_card(posting_id)?
            .ok_or(NarrationError::NoCard)?;
        narration::check(text, &card.card)?;
        let row = ScoreNarration {
            id: new_narration_id(),
            posting_id: posting_id.to_string(),
            score_card_id: card.id.clone(),
            lang: lang.to_string(),
            text: text.trim().to_string(),
            // ADR-010: narration is written from third-party content.
            untrusted_origin: true,
            created_at: now.to_string(),
        };
        self.scoring.insert_narration(&row)?;
        Ok(view(row))
    }
}

fn view(row: ScoreNarration) -> NarrationView {
    NarrationView {
        text: row.text,
        lang: row.lang,
        untrusted_origin: row.untrusted_origin,
    }
}

/// A language id the catalogues can serve: `en` or `es`, from the UI. Anything
/// else is refused rather than stored as a narration nothing can show.
fn language(lang: &str) -> Result<&'static str, NarrationError> {
    let lang = lang.trim().to_lowercase();
    match lang.as_str() {
        "en" => Ok("en"),
        "es" => Ok("es"),
        _ => Err(NarrationError::Language),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::repo::scoring::ScoringRepo;
    use crate::scoring::testing::FakeScoringRepo;
    use boba_domain::scoring::factors::{Factor, FactorScore, FactorWeights};
    use boba_domain::scoring::gates::{Confidence, GateReport};
    use boba_domain::scoring::rank::{
        CardVersions, MODEL_VERSION, ScoreCardRow, card as build_card,
    };

    const POSTING: &str = "pst_01NARRATION";

    fn scored_row() -> ScoreCardRow {
        let mut factors = Vec::new();
        for factor in Factor::ALL {
            let mut score = FactorScore::scored(factor, 80.0, 0.1, Confidence::High, Vec::new());
            if factor == Factor::Skills {
                score.matched = vec!["dbt".to_string(), "Airflow".to_string()];
                score.missing = vec!["Kubernetes".to_string()];
            }
            factors.push(score);
        }
        let card = build_card(MODEL_VERSION, factors, &FactorWeights::DEFAULT).expect("a card");
        ScoreCardRow::new(
            "sc_01",
            POSTING,
            CardVersions {
                profile_version: 1,
                prefs_version: 1,
                model_version: MODEL_VERSION,
            },
            card,
            GateReport {
                results: Vec::new(),
            },
            "2026-10-16T00:00:00Z",
        )
    }

    #[test]
    fn save_stores_a_grounded_narration_for_the_language() {
        let repo = Arc::new(FakeScoringRepo::default());
        repo.insert_card(&scored_row()).expect("a card");
        let use_case = Narration::new(repo.clone());

        let text = "You match most of the profile, with dbt and Airflow standing out. \
                    Kubernetes is the one skill missing.";
        let saved = use_case
            .save(POSTING, "en", text, "2026-10-16T01:00:00Z")
            .expect("saved");
        assert_eq!(saved.lang, "en");
        assert!(saved.untrusted_origin);
        assert_eq!(saved.text, text);

        let stored = repo
            .latest_narration(POSTING, "sc_01", "en")
            .expect("read")
            .expect("a row");
        assert_eq!(stored.score_card_id, "sc_01");
        assert_eq!(stored.created_at, "2026-10-16T01:00:00Z");
    }

    #[test]
    fn save_refuses_an_ungrounded_narration_and_stores_nothing() {
        let repo = Arc::new(FakeScoringRepo::default());
        repo.insert_card(&scored_row()).expect("a card");
        let use_case = Narration::new(repo.clone());

        let text = "You match 97 of 99 points here. dbt and Airflow stand out.";
        let error = use_case
            .save(POSTING, "en", text, "2026-10-16T01:00:00Z")
            .expect_err("refused");
        assert_eq!(error.message_key(), "errors.narration.ungrounded");
        assert_eq!(
            repo.latest_narration(POSTING, "sc_01", "en").expect("read"),
            None
        );
    }

    #[test]
    fn save_refuses_when_the_offer_has_no_card() {
        let repo = Arc::new(FakeScoringRepo::default());
        let use_case = Narration::new(repo);
        let error = use_case
            .save(POSTING, "en", "Two sentences. Of explanation.", "t")
            .expect_err("no card");
        assert_eq!(error.message_key(), "errors.narration.no_card");
    }

    #[test]
    fn latest_returns_the_newest_text_for_one_language() {
        let repo = Arc::new(FakeScoringRepo::default());
        repo.insert_card(&scored_row()).expect("a card");
        let use_case = Narration::new(repo.clone());

        use_case
            .save(
                POSTING,
                "en",
                "First text here. Second sentence.",
                "2026-10-16T01:00:00Z",
            )
            .expect("first");
        use_case
            .save(
                POSTING,
                "en",
                "Newer text here. Second sentence.",
                "2026-10-16T02:00:00Z",
            )
            .expect("second");

        let latest = use_case
            .latest(POSTING, "en")
            .expect("read")
            .expect("a row");
        assert_eq!(latest.text, "Newer text here. Second sentence.");
        assert_eq!(use_case.latest(POSTING, "es").expect("read"), None);
    }

    #[test]
    fn an_unsupported_language_is_refused() {
        let repo = Arc::new(FakeScoringRepo::default());
        let use_case = Narration::new(repo);
        let error = use_case.latest(POSTING, "fr").expect_err("refused");
        assert_eq!(error.message_key(), "errors.narration.language");
    }

    #[test]
    fn latest_ignores_a_narration_written_for_an_older_card() {
        let repo = Arc::new(FakeScoringRepo::default());
        repo.insert_card(&scored_row()).expect("a card");
        let use_case = Narration::new(repo.clone());
        use_case
            .save(
                POSTING,
                "en",
                "You match dbt and Airflow. Kubernetes is missing.",
                "2026-10-16T01:00:00Z",
            )
            .expect("a narration for the first card");

        let mut newer = scored_row();
        newer.id = "sc_02".to_string();
        newer.prefs_version = 2;
        newer.created_at = "2026-10-17T00:00:00Z".to_string();
        repo.insert_card(&newer).expect("the re-score");

        assert_eq!(
            use_case.latest(POSTING, "en").expect("read"),
            None,
            "the old text must not describe the new card"
        );
    }
}
