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
    CoverLetterDocument, CvDocument, DocumentKind, Lang, LanguageChoice, NewDocument,
    SCHEMA_VERSION, TemplateId, new_document_id,
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
    /// A `save_document_draft` payload this build cannot store: not an object,
    /// not a draft of either kind, a newer schema, or an unknown language. The
    /// run is told, so a model that misreads the contract retries instead of
    /// quietly storing nothing.
    #[error("this draft is not one the core can store: {0}")]
    Draft(String),
    /// The run tried to store a draft for another application than the one it
    /// was started for (ADR-010: a run disposes only of its own job).
    #[error("this run was started for `{run}`, but its draft names `{payload}`")]
    WrongApplication { run: String, payload: String },
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

/// What one stored agent draft was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedDraft {
    pub id: String,
    pub kind: DocumentKind,
    pub version: i64,
    pub lang: Lang,
}

/// Store one `save_document_draft` call (S7.7): the tailor run writes the CV,
/// then the cover letter, one call each.
///
/// The payload is refused whole unless it is a draft this build can store — an
/// object naming this run's application, a known schema version and language,
/// and one document shape. A claim-bearing item with no `fact_ids` fails to
/// deserialize, so the no-fabrication rule is enforced here as well as in the
/// validator: a run that invents a sentence without a fact stores nothing and
/// reads the refusal back.
///
/// The stored row is `untrusted_origin = true` (ADR-010: agent text that read
/// the posting). `offer_lang` is the Offer's content language, used for the same
/// notice `Lang::for_offer` records: a draft in EN for a posting in another
/// language explains itself in the studio (FR-70).
pub fn save_agent_draft(
    repo: &dyn DocumentsRepo,
    application_id: &str,
    args: &serde_json::Value,
    offer_lang: Option<&str>,
    created_at: &str,
) -> Result<SavedDraft, TailorError> {
    let object = args
        .as_object()
        .ok_or_else(|| TailorError::Draft("the arguments must be an object".to_string()))?;
    for key in object.keys() {
        if key != "app_id" && key != "doc" {
            return Err(TailorError::Draft(format!("unknown field `{key}`")));
        }
    }
    let payload_app = object
        .get("app_id")
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| TailorError::Draft("`app_id` must be a non-empty string".to_string()))?;
    if payload_app != application_id {
        return Err(TailorError::WrongApplication {
            run: application_id.to_string(),
            payload: payload_app.to_string(),
        });
    }
    let doc = object
        .get("doc")
        .ok_or_else(|| TailorError::Draft("`doc` is required".to_string()))?;

    // The two shapes are told apart by the field only one of them has: a CV has
    // `sections`, a cover letter `paragraphs`.
    let (kind, lang, model_json) = if doc.get("paragraphs").is_some() {
        let model: CoverLetterDocument = serde_json::from_value(doc.clone())
            .map_err(|error| TailorError::Draft(error.to_string()))?;
        let json = serialized(&model, model.schema_version, model.lang)?;
        (DocumentKind::CoverLetter, model.lang, json)
    } else {
        let model: CvDocument = serde_json::from_value(doc.clone())
            .map_err(|error| TailorError::Draft(error.to_string()))?;
        let json = serialized(&model, model.schema_version, model.lang)?;
        (DocumentKind::Cv, model.lang, json)
    };

    // The run writes in the language the core chose for this job (FR-70); a
    // draft in the other language is a contract violation, not a preference.
    let choice = Lang::for_offer(offer_lang.unwrap_or(lang.code()), None);
    if choice.lang != lang {
        return Err(TailorError::Draft(format!(
            "this draft is {}, but the job's language is {}",
            lang.code(),
            choice.lang.code()
        )));
    }

    let id = new_document_id();
    let version = repo.insert(&NewDocument {
        id: id.clone(),
        application_id: application_id.to_string(),
        kind,
        lang,
        // No preference for a template exists yet (S3.x): a run drafts into the
        // default of its kind, which is what the studio offers first.
        template: TemplateId::new(match kind {
            DocumentKind::Cv => "cv-classic",
            DocumentKind::CoverLetter => "cover-classic",
        }),
        model_json,
        language_fallback_from: choice.fallback_from,
        // Agent-written text that read the posting (ADR-010).
        untrusted_origin: true,
        created_at: created_at.to_string(),
    })?;
    Ok(SavedDraft {
        id,
        kind,
        version,
        lang,
    })
}

/// The draft as it is stored, after the schema check: a build that cannot read
/// the draft it was handed must not write it.
fn serialized(
    model: &impl serde::Serialize,
    schema_version: u32,
    _lang: Lang,
) -> Result<String, TailorError> {
    if schema_version != SCHEMA_VERSION {
        return Err(TailorError::Draft(format!(
            "schema version {schema_version} is not {SCHEMA_VERSION}"
        )));
    }
    serde_json::to_string(model).map_err(|error| TailorError::Serialize(error.to_string()))
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

    fn draft_call(app_id: &str, doc: serde_json::Value) -> serde_json::Value {
        serde_json::json!({ "app_id": app_id, "doc": doc })
    }

    fn cv_payload(lang: &str) -> serde_json::Value {
        serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "lang": lang,
            "contact": {"full_name": "Ada Lovelace"},
            "summary": {"text": "Backend developer.", "fact_ids": ["fact_1"]},
            "sections": [{"heading": "Experience",
                          "items": [{"text": "Shipped an API.", "fact_ids": ["fact_2"]}]}],
            "declared_gaps": ["Kubernetes"]
        })
    }

    fn cover_payload(lang: &str) -> serde_json::Value {
        serde_json::json!({
            "schema_version": SCHEMA_VERSION,
            "lang": lang,
            "contact": {"full_name": "Ada Lovelace"},
            "company": "Acme",
            "paragraphs": [{"text": "I would like to apply.", "fact_ids": ["fact_1"]}],
            "closing": "Sincerely, Ada"
        })
    }

    // AC1 (S7.7): the two `save_document_draft` calls of a run are stored as the
    // application's next CV and cover-letter versions, untrusted (ADR-010).
    #[test]
    fn an_agent_draft_is_stored_as_the_next_version_and_is_untrusted() {
        let repo = FakeDocumentsRepo::default();
        save_drafts(
            &repo,
            "app_1",
            &Lang::for_offer("en", None),
            &templates(),
            &drafts(Lang::En),
            "t1",
        )
        .expect("the run's first pass");

        let cv = save_agent_draft(
            &repo,
            "app_1",
            &draft_call("app_1", cv_payload("en")),
            Some("en"),
            "t2",
        )
        .expect("the CV");
        assert_eq!(cv.kind, DocumentKind::Cv);
        assert_eq!(cv.version, 2, "a regeneration is the next version");
        let cover = save_agent_draft(
            &repo,
            "app_1",
            &draft_call("app_1", cover_payload("en")),
            Some("en"),
            "t2",
        )
        .expect("the cover letter");
        assert_eq!(cover.kind, DocumentKind::CoverLetter);
        assert_eq!(cover.version, 2);

        let rows = repo.rows();
        assert_eq!(rows.len(), 4);
        assert!(rows[2..].iter().all(|row| row.untrusted_origin));
        let model: CvDocument = serde_json::from_str(&rows[2].model_json).expect("json");
        assert_eq!(
            model.sections[0].items[0].fact_ids,
            vec!["fact_2".to_string()]
        );
    }

    // AD-15: a draft whose claim names no fact is refused before it is stored,
    // so a run reads the refusal back and retries instead of half-storing.
    #[test]
    fn a_draft_whose_claim_names_no_fact_is_refused_whole() {
        let repo = FakeDocumentsRepo::default();
        let mut payload = cv_payload("en");
        payload["sections"][0]["items"][0]
            .as_object_mut()
            .expect("the item")
            .remove("fact_ids");

        let error = save_agent_draft(
            &repo,
            "app_1",
            &draft_call("app_1", payload),
            Some("en"),
            "t",
        )
        .expect_err("no fact, no draft");
        assert!(matches!(error, TailorError::Draft(_)), "{error:?}");
        assert!(repo.rows().is_empty(), "nothing was stored");
    }

    // ADR-010: a run disposes only of its own job.
    #[test]
    fn a_draft_for_another_application_is_refused() {
        let repo = FakeDocumentsRepo::default();
        let error = save_agent_draft(
            &repo,
            "app_1",
            &draft_call("app_2", cv_payload("en")),
            Some("en"),
            "t",
        )
        .expect_err("not this run's application");
        assert_eq!(
            error,
            TailorError::WrongApplication {
                run: "app_1".to_string(),
                payload: "app_2".to_string()
            }
        );
        assert!(repo.rows().is_empty());
    }

    // A newer schema, or a language this build does not write, is refused.
    #[test]
    fn a_newer_schema_or_unknown_language_is_refused() {
        let repo = FakeDocumentsRepo::default();
        let mut newer = cv_payload("en");
        newer["schema_version"] = serde_json::json!(SCHEMA_VERSION + 1);
        assert!(matches!(
            save_agent_draft(&repo, "app_1", &draft_call("app_1", newer), Some("en"), "t"),
            Err(TailorError::Draft(_))
        ));
        let mut unknown = cv_payload("de");
        unknown["lang"] = serde_json::json!("de");
        assert!(matches!(
            save_agent_draft(
                &repo,
                "app_1",
                &draft_call("app_1", unknown),
                Some("de"),
                "t"
            ),
            Err(TailorError::Draft(_))
        ));
        assert!(repo.rows().is_empty());
    }

    // FR-70: an EN draft of a German posting carries the reason the studio shows.
    #[test]
    fn a_fallback_language_is_recorded_with_the_draft() {
        let repo = FakeDocumentsRepo::default();
        save_agent_draft(
            &repo,
            "app_1",
            &draft_call("app_1", cv_payload("en")),
            Some("de"),
            "t",
        )
        .expect("the CV");
        assert_eq!(repo.rows()[0].language_fallback_from.as_deref(), Some("de"));

        // A posting in the draft's own language has nothing to explain.
        save_agent_draft(
            &repo,
            "app_1",
            &draft_call("app_1", cover_payload("en")),
            Some("en"),
            "t",
        )
        .expect("the cover letter");
        assert_eq!(repo.rows()[1].language_fallback_from, None);
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
