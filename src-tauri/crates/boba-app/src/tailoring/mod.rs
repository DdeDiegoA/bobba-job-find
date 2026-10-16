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
    CoverLetterDocument, CvDocument, DocumentKind, Lang, NewDocument, TemplateId, new_document_id,
};

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

/// The ids of the two stored documents (the CV, then the cover letter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedDrafts {
    pub cv_id: String,
    pub cover_id: String,
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
/// `lang` is stamped onto both drafts here, so the language the run chose is the
/// language that is stored — a draft can never disagree with its own row — and
/// both rows are written `untrusted_origin = true` (ADR-010, AC7). A failure on
/// the second insert leaves the first document stored: the studio regenerates,
/// which bumps `version` (S7.1).
pub fn save_drafts(
    repo: &dyn DocumentsRepo,
    application_id: &str,
    lang: Lang,
    templates: &DraftTemplates,
    drafts: &Drafts,
    created_at: &str,
) -> Result<SavedDrafts, TailorError> {
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
    Ok(SavedDrafts { cv_id, cover_id })
}

fn json(model: &impl serde::Serialize) -> Result<String, TailorError> {
    serde_json::to_string(model).map_err(|e| TailorError::Serialize(e.to_string()))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use boba_domain::documents::{
        Claim, Contact, DocumentRecord, LanguageChoice, Link, SCHEMA_VERSION, Section,
    };

    use super::*;

    /// An in-memory `DocumentsRepo` (test-only). The real SQL and the adapter are
    /// tested against an encrypted database in `boba-store`.
    #[derive(Default)]
    struct FakeDocumentsRepo {
        rows: Mutex<Vec<NewDocument>>,
        /// When set, `insert` fails the way a full disk would.
        fail: bool,
    }

    impl FakeDocumentsRepo {
        fn rows(&self) -> Vec<NewDocument> {
            self.rows.lock().unwrap().clone()
        }
    }

    impl DocumentsRepo for FakeDocumentsRepo {
        fn insert(&self, new: &NewDocument) -> Result<i64, DocumentsRepoError> {
            if self.fail {
                return Err(DocumentsRepoError::new("disk gone"));
            }
            self.rows.lock().unwrap().push(new.clone());
            Ok(1)
        }

        fn get(&self, _id: &str) -> Result<Option<DocumentRecord>, DocumentsRepoError> {
            Ok(None)
        }

        fn list_for_application(
            &self,
            _application_id: &str,
        ) -> Result<Vec<DocumentRecord>, DocumentsRepoError> {
            Ok(vec![])
        }

        fn latest(
            &self,
            _application_id: &str,
            _kind: DocumentKind,
        ) -> Result<Option<DocumentRecord>, DocumentsRepoError> {
            Ok(None)
        }

        fn set_pdf(&self, _: &str, _: &[u8], _: &str) -> Result<(), DocumentsRepoError> {
            Ok(())
        }

        fn set_docx(&self, _: &str, _: &[u8], _: &str) -> Result<(), DocumentsRepoError> {
            Ok(())
        }

        fn mark_verified(&self, _: &str, _: &str, _: &str) -> Result<(), DocumentsRepoError> {
            Ok(())
        }

        fn pdf(&self, _: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError> {
            Ok(None)
        }

        fn docx(&self, _: &str) -> Result<Option<Vec<u8>>, DocumentsRepoError> {
            Ok(None)
        }
    }

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
            choice.lang,
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
    }

    // AC2/AC7: every claim carries its facts, gaps are declared, and an
    // agent-written draft is stored untrusted (ADR-010).
    #[test]
    fn stored_drafts_carry_fact_ids_gaps_and_untrusted_origin() {
        let repo = FakeDocumentsRepo::default();
        save_drafts(
            &repo,
            "app_1",
            Lang::En,
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

    #[test]
    fn an_override_is_drafted_and_a_fallback_keeps_its_reason() {
        assert_eq!(Lang::for_offer("es", Some(Lang::En)).lang, Lang::En);
        let fallback = Lang::for_offer("de", None);
        assert_eq!(fallback.lang, Lang::En);
        assert_eq!(fallback.fallback_from.as_deref(), Some("de"));
    }

    #[test]
    fn a_failed_insert_reaches_the_caller() {
        let repo = FakeDocumentsRepo {
            fail: true,
            ..Default::default()
        };
        let error = save_drafts(
            &repo,
            "app_1",
            Lang::En,
            &templates(),
            &drafts(Lang::En),
            "t",
        )
        .expect_err("the store refused the row");
        assert!(matches!(error, TailorError::Repo(_)));
        assert!(repo.rows().is_empty());
    }
}
