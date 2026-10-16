//! Tailoring use case (S7.2, FR-69/FR-70, AD-14/AD-15/AD-17).
//!
//! A tailoring run drafts a CV and a cover letter for one Offer. The drafts
//! themselves come from the `tailor_application` agent task, so this layer owns
//! the two things the core must guarantee about them:
//!
//! * **the language** — FR-70: the Offer's language when it is ES or EN (the
//!   user's override wins), otherwise EN with a reason the studio can show;
//! * **the provenance** — a draft is agent-written text that read the posting, so
//!   its row is stored `untrusted_origin = true` (ADR-010) and is never rendered
//!   as anything but data.
//!
//! Language selection itself is `boba_domain::documents::Lang::for_offer`; the
//! drafting, reviewing and verifying around this step are S7.3–S7.7.

use boba_domain::documents::{
    CoverLetterDocument, CvDocument, DocumentKind, LanguageChoice, NewDocument, TemplateId,
    new_document_id,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ports::repo::documents::{DocumentsRepo, DocumentsRepoError};

/// The two drafts a tailoring run wrote for one Offer (FR-69).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drafts {
    pub cv: CvDocument,
    pub cover: CoverLetterDocument,
}

/// The templates a run drafts into. The studio reads the user's choices from
/// `RenderPort::templates`; nothing here picks a default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftTemplates {
    pub cv: TemplateId,
    pub cover: TemplateId,
}

/// The two stored documents of a tailoring run, as the payload the tailor
/// surface returns to the studio (AD-3: typed, translated by the UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SavedDrafts {
    /// The draft CV's document id.
    pub cv_id: String,
    /// The draft cover letter's document id.
    pub cover_id: String,
    /// The Offer's language, set only when it was neither ES nor EN and the run
    /// drafted in EN anyway (FR-70). The studio shows it as the reason for the
    /// language it did not expect; `None` means there is nothing to explain. The
    /// same value is stored on both documents, so a reload keeps the notice.
    pub fallback_from: Option<String>,
}

/// Why a run could not store its drafts.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TailorError {
    #[error(transparent)]
    Repo(#[from] DocumentsRepoError),
    /// The document model did not serialize. An internal fault, never something
    /// the user or the agent can cause.
    #[error("document model could not be serialized: {0}")]
    Serialize(String),
}

/// Store both drafts as documents of `application_id` (AD-16).
///
/// `language` is the choice the run drafted in: it is stamped onto both drafts
/// here, so a draft can never disagree with its own row, and both rows are
/// written `untrusted_origin = true` (ADR-010, AC7). Its `fallback_from` comes
/// back in [`SavedDrafts`] so the studio can explain an EN draft of a posting in
/// some other language (FR-70). A failure on the second insert leaves the first
/// document stored: the studio regenerates, which bumps `version` (S7.1).
pub fn save_drafts(
    repo: &dyn DocumentsRepo,
    application_id: &str,
    language: &LanguageChoice,
    templates: &DraftTemplates,
    drafts: &Drafts,
    created_at: &str,
) -> Result<SavedDrafts, TailorError> {
    let lang = language.lang;
    let save = |kind: DocumentKind,
                template: &TemplateId,
                model_json: String|
     -> Result<String, TailorError> {
        let id = new_document_id();
        repo.insert(&NewDocument {
            id: id.clone(),
            application_id: application_id.to_string(),
            kind,
            lang,
            template: template.clone(),
            model_json,
            // Stored with the row, so the studio's notice survives a reload.
            language_fallback_from: language.fallback_from.clone(),
            // Agent-written text that read the posting: untrusted (ADR-010, AC7).
            untrusted_origin: true,
            created_at: created_at.to_string(),
        })?;
        Ok(id)
    };

    let cv_id = save(
        DocumentKind::Cv,
        &templates.cv,
        json(&drafts.cv.clone().with_lang(lang))?,
    )?;
    let cover_id = save(
        DocumentKind::CoverLetter,
        &templates.cover,
        json(&drafts.cover.clone().with_lang(lang))?,
    )?;
    Ok(SavedDrafts {
        cv_id,
        cover_id,
        fallback_from: language.fallback_from.clone(),
    })
}

fn json(model: &impl serde::Serialize) -> Result<String, TailorError> {
    serde_json::to_string(model).map_err(|e| TailorError::Serialize(e.to_string()))
}

#[cfg(test)]
mod tests {
    use boba_domain::documents::{Claim, Contact, Lang, Link, SCHEMA_VERSION, Section};

    use super::*;
    use crate::ports::repo::documents::fake::FakeDocumentsRepo;

    fn contact() -> Contact {
        Contact {
            full_name: "Ada Lovelace".into(),
            headline: None,
            email: Some("ada@example.com".into()),
            phone: None,
            location: None,
            links: vec![Link {
                label: "GitHub".into(),
                url: "github.com/ada".into(),
            }],
        }
    }

    fn drafts(lang: Lang) -> Drafts {
        Drafts {
            cv: CvDocument {
                schema_version: SCHEMA_VERSION,
                lang,
                contact: contact(),
                summary: None,
                sections: vec![Section {
                    heading: "Experience".into(),
                    items: vec![Claim {
                        text: "Shipped an API".into(),
                        fact_ids: vec!["fact_1".into()],
                    }],
                }],
                declared_gaps: vec!["Kubernetes".into()],
            },
            cover: CoverLetterDocument {
                schema_version: SCHEMA_VERSION,
                lang,
                contact: contact(),
                company: Some("Acme".into()),
                recipient: None,
                date: None,
                salutation: None,
                paragraphs: vec![Claim {
                    text: "I would like to apply.".into(),
                    fact_ids: vec!["fact_2".into()],
                }],
                closing: None,
                declared_gaps: vec![],
            },
        }
    }

    fn templates() -> DraftTemplates {
        DraftTemplates {
            cv: TemplateId::new("cv-classic"),
            cover: TemplateId::new("cover-classic"),
        }
    }

    // AC1: a Spanish Offer drafts both documents in Spanish, and the drafts are
    // stored in the language the run chose, whatever the agent left on them.
    #[test]
    fn both_drafts_are_stored_in_the_chosen_language() {
        let choice = Lang::for_offer("es-MX", None);
        assert_eq!(
            choice,
            LanguageChoice {
                lang: Lang::Es,
                fallback_from: None
            }
        );

        let repo = FakeDocumentsRepo::default();
        let saved = save_drafts(
            &repo,
            "app_1",
            &choice,
            &templates(),
            &drafts(Lang::En), // the agent answered in the wrong language
            "2026-10-16T09:00:00Z",
        )
        .expect("save");

        let rows = repo.rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, DocumentKind::Cv);
        assert_eq!(rows[1].kind, DocumentKind::CoverLetter);
        for row in &rows {
            assert_eq!(row.lang, Lang::Es, "both drafts follow the Offer");
            assert_eq!(row.application_id, "app_1");
            assert_eq!(row.created_at, "2026-10-16T09:00:00Z");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&row.model_json).expect("json")["lang"],
                "es",
                "the stored model agrees with its row"
            );
        }
        assert_eq!(rows[0].template.as_str(), "cv-classic");
        assert_eq!(rows[1].template.as_str(), "cover-classic");
        assert_eq!(rows[0].id, saved.cv_id);
        assert_eq!(rows[1].id, saved.cover_id);
        assert_eq!(
            saved.fallback_from, None,
            "the Offer's own language needs no notice"
        );
    }

    // AC2/AC7: every claim carries its facts, gaps are declared, and an
    // agent-written draft is stored untrusted (ADR-010).
    #[test]
    fn stored_drafts_carry_fact_ids_gaps_and_untrusted_origin() {
        let repo = FakeDocumentsRepo::default();
        save_drafts(
            &repo,
            "app_1",
            &Lang::for_offer("en", None),
            &templates(),
            &drafts(Lang::En),
            "2026-10-16T09:00:00Z",
        )
        .expect("save");

        let rows = repo.rows();
        assert!(rows.iter().all(|row| row.untrusted_origin));
        let cv: serde_json::Value =
            serde_json::from_str(&rows[0].model_json).expect("the stored CV is JSON");
        assert_eq!(cv["sections"][0]["items"][0]["fact_ids"][0], "fact_1");
        assert_eq!(cv["declared_gaps"][0], "Kubernetes");
    }

    // AC1: an override needs no notice, a fallback hands its reason back to the
    // caller, and a draft of the Offer's own language carries neither.
    #[test]
    fn an_override_is_drafted_and_a_fallback_reaches_the_caller() {
        let repo = FakeDocumentsRepo::default();

        let override_choice = Lang::for_offer("es", Some(Lang::En));
        let saved = save_drafts(
            &repo,
            "app_1",
            &override_choice,
            &templates(),
            &drafts(Lang::En),
            "t",
        )
        .expect("save");
        assert_eq!(saved.fallback_from, None, "an override is not a fallback");
        assert_eq!(repo.rows()[0].lang, Lang::En);
        assert_eq!(
            repo.rows()[0].language_fallback_from,
            None,
            "an override leaves nothing to explain"
        );

        let saved = save_drafts(
            &repo,
            "app_2",
            &Lang::for_offer("de", None),
            &templates(),
            &drafts(Lang::En),
            "t",
        )
        .expect("save");
        assert_eq!(
            saved.fallback_from.as_deref(),
            Some("de"),
            "the studio is told why an EN draft was written for a German posting"
        );
        assert!(
            repo.rows()
                .iter()
                .filter(|row| row.application_id == "app_2")
                .all(|row| row.language_fallback_from.as_deref() == Some("de")),
            "and the reason is stored with both documents"
        );
    }

    #[test]
    fn a_failed_insert_reaches_the_caller() {
        let repo = FakeDocumentsRepo::failing();
        let error = save_drafts(
            &repo,
            "app_1",
            &Lang::for_offer("en", None),
            &templates(),
            &drafts(Lang::En),
            "t",
        )
        .expect_err("the store refused the row");
        assert!(matches!(error, TailorError::Repo(_)));
        assert!(repo.rows().is_empty());
    }
}
