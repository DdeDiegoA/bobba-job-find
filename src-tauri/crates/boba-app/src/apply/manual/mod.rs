//! The manual (link-out) apply path (S9.2, FR-81/FR-82, EXPERIENCE §9.5).
//!
//! Every Offer has at least one Apply Channel, and when the channel is Manual
//! the user applies on the employer's own page. This module gives that user
//! everything the page asks for, in one read:
//!
//! * **channels** — what the Offer allows, as `boba-domain::apply::channel`
//!   detected it;
//! * **the kit** — the verified PDFs of the application's package (AD-14: only a
//!   verified document is attachable) and the answers the user approved (S7.8),
//!   ready to copy;
//! * **mark applied** — the same status transition the Tracker uses (AD-16): an
//!   event with `source = user`, so the history says who moved the card.

use std::sync::Arc;

use boba_domain::application::{AppStatus, Application, PackageItem};
use boba_domain::apply::channel::{self, ApplyChannel};
use boba_domain::documents::{DocumentModel, DocumentRecord};
use boba_domain::posting::Offer;
use boba_domain::profile::ProfileFact;
use boba_domain::validator;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::applications::{TrackerError, TrackerService, TransitionInput};
use crate::ports::clock::ClockPort;
use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use crate::ports::repo::documents::{DocumentsRepo, DocumentsRepoError};
use crate::ports::repo::profile::{ProfileRepo, ProfileRepoError};

/// One way to apply, as the apply bar shows it (FR-81).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ChannelView {
    /// Write to the posting's apply address.
    EmailApply { address: String },
    /// Fill the employer's hosted ATS form.
    AtsForm { provider: String, url: String },
    /// Open the original page and apply by hand.
    Manual { url: String },
}

impl From<ApplyChannel> for ChannelView {
    fn from(channel: ApplyChannel) -> Self {
        match channel {
            ApplyChannel::EmailApply { address } => ChannelView::EmailApply { address },
            ApplyChannel::AtsForm { provider, url } => ChannelView::AtsForm {
                provider: provider.as_str().to_string(),
                url,
            },
            ApplyChannel::Manual { url } => ChannelView::Manual { url },
        }
    }
}

/// One verified PDF the user can attach by hand (AD-14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct KitDocumentView {
    pub document_id: String,
    /// `cv` or `cover_letter`.
    pub kind: String,
    pub lang: String,
    pub version: u32,
}

/// One screening answer the user approved, ready to copy (S7.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct KitAnswerView {
    pub question: String,
    pub answer: String,
    pub class: String,
}

/// Everything the Manual apply bar shows for one application (AC1/AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ManualView {
    pub application_id: String,
    pub posting_id: String,
    pub title: String,
    pub company: String,
    /// The stored status (`saved`, `drafted`, `applied`…).
    pub status: String,
    /// Email Apply, ATS Form and Manual, in that order (FR-81).
    pub channels: Vec<ChannelView>,
    /// The package's Ready and verified documents; empty until they render.
    pub documents: Vec<KitDocumentView>,
    /// The approved answers, oldest first.
    pub answers: Vec<KitAnswerView>,
    /// Kinds whose newest draft is not Ready and verified (AD-14), so the bar
    /// can say so while a manual apply is still the user's to do.
    pub pending: Vec<String>,
    /// Whether every draft in the package passed the attachable check (AD-14).
    pub ready: bool,
    /// The Session Board this Offer's Source is (`linkedin`, `indeed`), when
    /// Boba can fill its form in the user's own session (S11.2). `None` for
    /// every other Source, so the apply bar knows whether to offer it.
    pub session_board: Option<String>,
    /// An application to the same company and a similar role inside 30 days
    /// (S9.5, FR-92). The bar warns before anything is sent.
    pub duplicate: Option<crate::apply::guard::DuplicateGuardView>,
}

/// One PDF the command writes to disk for the user to attach by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfExport {
    /// Safe on every platform: no separator, no space.
    pub file_name: String,
    pub kind: String,
    pub version: u32,
    pub bytes: Vec<u8>,
}

/// Why the manual path refused an action.
#[derive(Debug, Clone, thiserror::Error)]
pub enum ManualError {
    #[error(transparent)]
    Repo(#[from] ApplyRepoError),
    #[error(transparent)]
    Documents(#[from] DocumentsRepoError),
    #[error(transparent)]
    Tracker(#[from] TrackerError),
    #[error(transparent)]
    Profile(#[from] ProfileRepoError),
    /// A package draft is not Ready and verified, so nothing may be applied
    /// with it yet (AD-14). The kinds name what to fix in the Studio.
    #[error("not Ready and verified: {0:?}")]
    NotReady(Vec<String>),
}

impl ManualError {
    /// The translated message key the UI shows (CH7: `errors.<area>.*`).
    pub fn message_key(&self) -> &'static str {
        match self {
            ManualError::Repo(ApplyRepoError::UnknownApplication(_)) => {
                "errors.apply.unknown_application"
            }
            ManualError::Repo(_) => "errors.apply.store",
            ManualError::Documents(_) | ManualError::Profile(_) => "errors.apply.store",
            ManualError::Tracker(error) => error.message_key(),
            ManualError::NotReady(_) => "errors.apply.not_ready",
        }
    }
}

/// The Manual apply use cases (AC1/AC2) over the apply and document owners.
pub struct ManualService {
    apply: Arc<dyn ApplyRepo>,
    documents: Arc<dyn DocumentsRepo>,
    profile: Arc<dyn ProfileRepo>,
    clock: Arc<dyn ClockPort>,
}

impl ManualService {
    pub fn new(
        apply: Arc<dyn ApplyRepo>,
        documents: Arc<dyn DocumentsRepo>,
        profile: Arc<dyn ProfileRepo>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            apply,
            documents,
            profile,
            clock,
        }
    }

    /// The channels and the copy-ready kit for one application (AC1/AC2).
    pub fn view(
        &self,
        application: &Application,
        offer: &Offer,
    ) -> Result<ManualView, ManualError> {
        let gate = self.gate(application)?;
        let answers = self
            .apply
            .answer_bank_for_application(&application.id)?
            .into_iter()
            .filter(|row| row.is_reusable())
            .filter_map(|row| {
                row.answer.map(|answer| KitAnswerView {
                    question: row.question,
                    answer,
                    class: row.class.as_str().to_string(),
                })
            })
            .collect();
        Ok(ManualView {
            application_id: application.id.clone(),
            posting_id: application.posting_id.clone(),
            title: offer.title.clone(),
            company: offer.company.clone(),
            status: application.status.as_str().to_string(),
            channels: channel::detect(offer)
                .into_iter()
                .map(ChannelView::from)
                .collect(),
            documents: gate
                .attachables
                .into_iter()
                .map(|item| KitDocumentView {
                    document_id: item.document_id,
                    kind: item.kind,
                    lang: item.lang,
                    version: item.version,
                })
                .collect(),
            answers,
            ready: gate.pending.is_empty(),
            pending: gate.pending,
            // The board is a fact about the Source, not about this package:
            // the bar offers the session fill before the documents are ready,
            // and the fill itself re-checks Ready (S11.2).
            session_board: crate::apply::session::SessionBoard::of_source(&offer.source_id)
                .map(|board| board.as_str().to_string()),
            // FR-92 (S9.5): warned here, before any channel sends.
            duplicate: crate::apply::guard::duplicate_for(
                self.apply.as_ref(),
                application,
                &self.clock.now_rfc3339(),
            )?,
        })
    }

    /// The package kinds that are not Ready and verified yet (AD-14).
    ///
    /// The same gate `view` reports, for a caller that only needs the answer —
    /// the Session Board apply re-checks it on every step (S11.2).
    pub fn pending(&self, application: &Application) -> Result<Vec<String>, ManualError> {
        Ok(self.gate(application)?.pending)
    }

    /// The verified PDFs of the package, bytes and safe file names (AD-14).
    pub fn export_pdfs(&self, application: &Application) -> Result<Vec<PdfExport>, ManualError> {
        let mut exports = Vec::new();
        for item in self.gate(application)?.attachables {
            let Some(bytes) = self.documents.pdf(&item.document_id)? else {
                continue;
            };
            exports.push(PdfExport {
                file_name: format!("{}-v{}.pdf", item.kind, item.version),
                kind: item.kind,
                version: item.version,
                bytes,
            });
        }
        Ok(exports)
    }

    /// Mark the application Applied after the user applied by hand (AC2).
    ///
    /// The move travels the Tracker's one writer (AD-16), with `source = user`:
    /// the history says a person applied, never an automated run.
    pub fn mark_applied(
        &self,
        application: &Application,
        offer: &Offer,
    ) -> Result<ManualView, ManualError> {
        // The S7.7 UI block is not the gate: the core re-checks what the
        // package would attach before the card may say Applied (AD-14).
        let gate = self.gate(application)?;
        if !gate.pending.is_empty() {
            return Err(ManualError::NotReady(gate.pending));
        }
        TrackerService::new(self.apply.clone(), self.clock.clone()).transition(
            TransitionInput {
                application_id: application.id.clone(),
                from_status: application.status.as_str().to_string(),
                to_status: AppStatus::Applied.as_str().to_string(),
                source: "user".to_string(),
                reason: None,
            },
        )?;
        let mut applied = application.clone();
        applied.status = AppStatus::Applied;
        self.view(&applied, offer)
    }

    /// The attachable-check for one application (AD-14): the **newest** draft
    /// of each kind, kept only when it is Ready (the validator backs every
    /// claim on the current Profile) and verified (the render passed) and has
    /// the PDF the user copies. Kinds that fail are named, so the bar can send
    /// the user back to the Studio.
    fn gate(&self, application: &Application) -> Result<Gate, ManualError> {
        let facts = self.profile.facts_current()?;
        let mut newest: Vec<(String, PackageItem)> = Vec::new();
        for item in self.apply.package(&application.id)? {
            match newest.iter_mut().find(|(kind, _)| *kind == item.kind) {
                Some((_, current)) if current.version >= item.version => {}
                Some((_, current)) => *current = item,
                None => newest.push((item.kind.clone(), item)),
            }
        }
        let mut gate = Gate::default();
        for (kind, item) in newest {
            match self.attachable(&item, &facts)? {
                Some(attachable) => gate.attachables.push(attachable),
                None => gate.pending.push(kind),
            }
        }
        Ok(gate)
    }

    /// One package item, when it may be attached (AD-14).
    fn attachable(
        &self,
        item: &PackageItem,
        facts: &[ProfileFact],
    ) -> Result<Option<KitItem>, ManualError> {
        if item.verified_at.is_none() {
            return Ok(None);
        }
        let Some(record) = self.documents.get(&item.document_id)? else {
            return Ok(None);
        };
        if !record.has_pdf {
            return Ok(None);
        }
        // A stored model this build cannot read is not attachable either: the
        // gate fails closed rather than handing out a PDF it cannot vouch for.
        let Ok(model) = model_of(&record) else {
            return Ok(None);
        };
        let report = validator::validate(&model, &|id| facts.iter().any(|fact| fact.id == id));
        if !report.is_ready() {
            return Ok(None);
        }
        Ok(Some(KitItem {
            document_id: item.document_id.clone(),
            kind: item.kind.clone(),
            lang: item.lang.clone(),
            version: u32::try_from(item.version).unwrap_or_default(),
        }))
    }
}

/// What the package may hand out, and what it may not (AD-14).
#[derive(Default)]
struct Gate {
    attachables: Vec<KitItem>,
    /// Kinds whose newest draft failed the attachable check.
    pending: Vec<String>,
}

/// Read a stored model, `None` for a row this build cannot parse (the same
/// shape the studio reads).
fn model_of(record: &DocumentRecord) -> Result<DocumentModel, serde_json::Error> {
    match record.kind {
        boba_domain::documents::DocumentKind::Cv => {
            serde_json::from_str(&record.model_json).map(DocumentModel::Cv)
        }
        boba_domain::documents::DocumentKind::CoverLetter => {
            serde_json::from_str(&record.model_json).map(DocumentModel::CoverLetter)
        }
    }
}

/// One package item that passed the attachable check (AD-14).
struct KitItem {
    document_id: String,
    kind: String,
    lang: String,
    version: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_domain::application::{PackageItem, new_application_id};
    use boba_domain::apply::channel::AtsProvider;
    use boba_domain::documents::{DocumentKind, Lang, NewDocument, TemplateId};
    use boba_domain::posting::{ApplyHints, WorkMode};

    use crate::applications::testing::FakeApplyRepo;
    use crate::ports::repo::documents::fake::FakeDocumentsRepo;
    use crate::profile::testing::FakeProfileRepo;
    use crate::system::FixedClock;

    const NOW: &str = "2026-10-17T00:00:00Z";

    /// A draft whose model parses and whose claims the empty Profile backs (a
    /// CV with no claim lines is Ready, and so is an empty cover letter).
    fn ready_model(kind: DocumentKind) -> String {
        match kind {
            DocumentKind::Cv => serde_json::json!({
                "schema_version": 1,
                "lang": "en",
                "contact": { "full_name": "Ada Lovelace" },
                "sections": [],
            }),
            DocumentKind::CoverLetter => serde_json::json!({
                "schema_version": 1,
                "lang": "en",
                "contact": { "full_name": "Ada Lovelace" },
                "paragraphs": [],
            }),
        }
        .to_string()
    }

    /// A draft the validator rejects: one claim line with no facts behind it.
    fn unbacked_model(kind: DocumentKind) -> String {
        match kind {
            DocumentKind::Cv => serde_json::json!({
                "schema_version": 1,
                "lang": "en",
                "contact": { "full_name": "Ada Lovelace" },
                "sections": [{
                    "heading": "Experience",
                    "items": [{ "text": "Shipped a billing API.", "fact_ids": [] }],
                }],
            }),
            DocumentKind::CoverLetter => serde_json::json!({
                "schema_version": 1,
                "lang": "en",
                "contact": { "full_name": "Ada Lovelace" },
                "paragraphs": [{ "text": "I shipped a billing API.", "fact_ids": [] }],
            }),
        }
        .to_string()
    }

    fn offer() -> Offer {
        Offer {
            id: "pst_01".into(),
            source_id: "src_01".into(),
            external_id: "ext-1".into(),
            title: "Backend Engineer".into(),
            company: "Nimbus".into(),
            locations: vec!["Remote (EU)".into()],
            work_mode: WorkMode::Remote,
            money: None,
            posted_at: None,
            deadline: None,
            first_seen: NOW.into(),
            last_seen: NOW.into(),
            lang: "en".into(),
            description_html: "<p>Hiring.</p>".into(),
            description_text: "Hiring.".into(),
            requirements: vec![],
            apply: ApplyHints {
                url: "https://jobs.lever.co/nimbus/abc".into(),
                email: Some("jobs@nimbus.example".into()),
            },
        }
    }

    fn application() -> Application {
        Application {
            id: new_application_id(),
            posting_id: "pst_01".into(),
            status: AppStatus::Drafted,
            offer_snapshot_json: "{}".into(),
            notes: None,
            next_action: None,
            created_at: NOW.into(),
            updated_at: NOW.into(),
        }
    }

    fn document(
        id: &str,
        application_id: &str,
        kind: DocumentKind,
        model_json: String,
    ) -> NewDocument {
        NewDocument {
            id: id.into(),
            application_id: application_id.into(),
            kind,
            lang: Lang::En,
            template: TemplateId::new("cv-classic"),
            model_json,
            language_fallback_from: None,
            untrusted_origin: true,
            created_at: NOW.into(),
        }
    }

    /// Seed one document: Ready, rendered, and verified when `verified`.
    fn seed_document(
        documents: &FakeDocumentsRepo,
        id: &str,
        application_id: &str,
        kind: DocumentKind,
        verified: bool,
        with_pdf: bool,
    ) -> i64 {
        seed_document_with_model(
            documents,
            id,
            application_id,
            kind,
            ready_model(kind),
            verified,
            with_pdf,
        )
    }

    /// Seed one document with the model a test needs.
    fn seed_document_with_model(
        documents: &FakeDocumentsRepo,
        id: &str,
        application_id: &str,
        kind: DocumentKind,
        model_json: String,
        verified: bool,
        with_pdf: bool,
    ) -> i64 {
        let version = documents
            .insert(&document(id, application_id, kind, model_json))
            .unwrap();
        if with_pdf {
            documents.set_pdf(id, b"%PDF-1.7 kit", NOW).unwrap();
        }
        if verified {
            documents.mark_verified(id, "{}", NOW).unwrap();
        }
        version
    }

    fn package_item(document_id: &str, kind: &str, version: i64, verified: bool) -> PackageItem {
        PackageItem {
            document_id: document_id.into(),
            kind: kind.into(),
            lang: "en".into(),
            version,
            verified_at: verified.then(|| NOW.to_string()),
        }
    }

    fn service(apply: &Arc<FakeApplyRepo>, documents: &Arc<FakeDocumentsRepo>) -> ManualService {
        ManualService::new(
            apply.clone(),
            documents.clone(),
            Arc::new(FakeProfileRepo::default()),
            Arc::new(FixedClock::new(NOW)),
        )
    }

    #[test]
    fn the_view_lists_the_channels_in_order() {
        let apply = Arc::new(FakeApplyRepo::default());
        let documents = Arc::new(FakeDocumentsRepo::default());
        let service = service(&apply, &documents);

        let view = service.view(&application(), &offer()).unwrap();

        assert_eq!(
            view.channels,
            vec![
                ChannelView::EmailApply {
                    address: "jobs@nimbus.example".into()
                },
                ChannelView::AtsForm {
                    provider: AtsProvider::Lever.as_str().into(),
                    url: "https://jobs.lever.co/nimbus/abc".into()
                },
                ChannelView::Manual {
                    url: "https://jobs.lever.co/nimbus/abc".into()
                },
            ]
        );
        assert_eq!(view.status, "drafted");
        assert_eq!(view.posting_id, "pst_01");
    }

    #[test]
    fn the_view_warns_when_a_similar_application_exists() {
        let apply = Arc::new(FakeApplyRepo::default());
        let documents = Arc::new(FakeDocumentsRepo::default());
        let service = service(&apply, &documents);
        let snapshot = serde_json::json!({
            "company": "Nimbus",
            "title": "Backend Engineer",
        })
        .to_string();

        // Sent three days ago to the same company for the same role (FR-92).
        apply.seed_application(Application {
            id: "app_prior".into(),
            posting_id: "pst_00".into(),
            status: AppStatus::Applied,
            offer_snapshot_json: snapshot.clone(),
            notes: None,
            next_action: None,
            created_at: "2026-10-14T00:00:00Z".into(),
            updated_at: "2026-10-14T00:00:00Z".into(),
        });
        let mut target = application();
        target.offer_snapshot_json = snapshot;

        let view = service.view(&target, &offer()).unwrap();

        assert_eq!(
            view.duplicate,
            Some(crate::apply::guard::DuplicateGuardView {
                role: "Backend Engineer".into(),
                days_ago: 3,
            })
        );
    }

    #[test]
    fn the_kit_holds_only_verified_rendered_documents() {
        let apply = Arc::new(FakeApplyRepo::default());
        let documents = Arc::new(FakeDocumentsRepo::default());
        let application = application();

        // An older draft of the same kind cannot be attached (AD-14).
        seed_document(
            &documents,
            "doc_cv_old",
            &application.id,
            DocumentKind::Cv,
            false,
            true,
        );
        let cv = seed_document(
            &documents,
            "doc_cv_1",
            &application.id,
            DocumentKind::Cv,
            true,
            true,
        );
        let cover = seed_document(
            &documents,
            "doc_cover_1",
            &application.id,
            DocumentKind::CoverLetter,
            true,
            true,
        );
        // The newest cover was verified but never rendered: no PDF to copy.
        let cover_pdf_less = seed_document(
            &documents,
            "doc_cover_pdf_less",
            &application.id,
            DocumentKind::CoverLetter,
            true,
            false,
        );

        apply.seed_package(&application.id, package_item("doc_cv_old", "cv", 0, false));
        apply.seed_package(&application.id, package_item("doc_cv_1", "cv", cv, true));
        apply.seed_package(
            &application.id,
            package_item("doc_cover_1", "cover_letter", cover, true),
        );
        apply.seed_package(
            &application.id,
            package_item("doc_cover_pdf_less", "cover_letter", cover_pdf_less, true),
        );

        let view = service(&apply, &documents)
            .view(&application, &offer())
            .unwrap();

        let ids: Vec<&str> = view
            .documents
            .iter()
            .map(|doc| doc.document_id.as_str())
            .collect();
        assert_eq!(ids, vec!["doc_cv_1"], "the newest CV is the attachable one");
        assert_eq!(view.documents[0].kind, "cv");
        assert_eq!(view.documents[0].version, cv as u32);
        assert_eq!(view.documents[0].lang, "en");
        assert_eq!(
            view.pending,
            vec!["cover_letter"],
            "the PDF-less cover is named, not handed out"
        );
        assert!(!view.ready);
    }

    #[test]
    fn the_kit_holds_only_approved_answers() {
        let apply = Arc::new(FakeApplyRepo::default());
        let documents = Arc::new(FakeDocumentsRepo::default());
        let application = application();

        apply
            .answer_bank_insert(&boba_domain::application::NewBankAnswer {
                id: "ans_1".into(),
                application_id: application.id.clone(),
                question: "What are your salary expectations?".into(),
                question_key: "what are your salary expectations".into(),
                answer: Some("€65,000".into()),
                class: boba_domain::application::AnswerClass::Salary,
                fact_ids: vec![],
                needs_review: true,
                untrusted_origin: true,
                created_at: NOW.into(),
            })
            .unwrap();
        apply
            .answer_bank_insert(&boba_domain::application::NewBankAnswer {
                id: "ans_2".into(),
                application_id: application.id.clone(),
                question: "When can you start?".into(),
                question_key: "when can you start".into(),
                answer: Some("Two weeks".into()),
                class: boba_domain::application::AnswerClass::Other,
                fact_ids: vec![],
                needs_review: false,
                untrusted_origin: true,
                created_at: NOW.into(),
            })
            .unwrap();
        apply.answer_bank_approve("ans_1", None, NOW).unwrap();

        let view = service(&apply, &documents)
            .view(&application, &offer())
            .unwrap();

        assert_eq!(view.answers.len(), 1, "the draft stays in review");
        assert_eq!(
            view.answers[0].question,
            "What are your salary expectations?"
        );
        assert_eq!(view.answers[0].answer, "€65,000");
        assert_eq!(view.answers[0].class, "salary");
    }

    #[test]
    fn an_empty_package_shows_an_empty_kit() {
        let apply = Arc::new(FakeApplyRepo::default());
        let documents = Arc::new(FakeDocumentsRepo::default());
        let view = service(&apply, &documents)
            .view(&application(), &offer())
            .unwrap();
        assert!(view.documents.is_empty());
        assert!(view.answers.is_empty());
        assert!(view.pending.is_empty());
        assert!(view.ready);
    }

    #[test]
    fn a_verified_draft_with_an_unbacked_claim_is_not_attachable() {
        let apply = Arc::new(FakeApplyRepo::default());
        let documents = Arc::new(FakeDocumentsRepo::default());
        let application = application();

        let cv = seed_document_with_model(
            &documents,
            "doc_cv_1",
            &application.id,
            DocumentKind::Cv,
            unbacked_model(DocumentKind::Cv),
            true,
            true,
        );
        apply.seed_package(&application.id, package_item("doc_cv_1", "cv", cv, true));

        let view = service(&apply, &documents)
            .view(&application, &offer())
            .unwrap();

        assert!(
            view.documents.is_empty(),
            "Ready is re-checked server-side; a verified render of an unbacked draft is not attachable"
        );
        assert_eq!(view.pending, vec!["cv"]);
        assert!(!view.ready);
    }

    #[test]
    fn marking_applied_is_refused_while_a_package_draft_is_not_ready() {
        let apply = Arc::new(FakeApplyRepo::default());
        let documents = Arc::new(FakeDocumentsRepo::default());
        let application = application();
        apply.seed_application(application.clone());

        // Verified render, but the draft names a claim with no Profile fact.
        let cv = seed_document_with_model(
            &documents,
            "doc_cv_1",
            &application.id,
            DocumentKind::Cv,
            unbacked_model(DocumentKind::Cv),
            true,
            true,
        );
        apply.seed_package(&application.id, package_item("doc_cv_1", "cv", cv, true));

        let error = service(&apply, &documents)
            .mark_applied(&application, &offer())
            .unwrap_err();

        assert_eq!(error.message_key(), "errors.apply.not_ready");
        assert!(apply.events().is_empty(), "a refused apply writes nothing");
        assert_eq!(
            apply.stored_status(&application.id),
            Some(AppStatus::Drafted)
        );
    }

    #[test]
    fn export_pdfs_returns_verified_bytes_with_safe_names() {
        let apply = Arc::new(FakeApplyRepo::default());
        let documents = Arc::new(FakeDocumentsRepo::default());
        let application = application();

        let cv = seed_document(
            &documents,
            "doc_cv_1",
            &application.id,
            DocumentKind::Cv,
            true,
            true,
        );
        seed_document(
            &documents,
            "doc_cover_old",
            &application.id,
            DocumentKind::CoverLetter,
            false,
            true,
        );
        apply.seed_package(&application.id, package_item("doc_cv_1", "cv", cv, true));
        apply.seed_package(
            &application.id,
            package_item("doc_cover_old", "cover_letter", 0, false),
        );

        let exports = service(&apply, &documents)
            .export_pdfs(&application)
            .unwrap();

        assert_eq!(exports.len(), 1);
        assert_eq!(exports[0].file_name, format!("cv-v{cv}.pdf"));
        assert_eq!(exports[0].bytes, b"%PDF-1.7 kit");
    }

    #[test]
    fn marking_applied_writes_one_user_event_and_moves_the_card() {
        let apply = Arc::new(FakeApplyRepo::default());
        let documents = Arc::new(FakeDocumentsRepo::default());
        let application = application();
        apply.seed_application(application.clone());

        let view = service(&apply, &documents)
            .mark_applied(&application, &offer())
            .unwrap();

        assert_eq!(
            apply.stored_status(&application.id),
            Some(AppStatus::Applied)
        );
        let events = apply.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].from_status, Some(AppStatus::Drafted));
        assert_eq!(events[0].to_status, AppStatus::Applied);
        assert_eq!(
            events[0].source.as_str(),
            "user",
            "a manual apply is the user's, never an automated run"
        );
        assert_eq!(view.status, "applied");
    }

    #[test]
    fn marking_an_already_applied_card_is_refused() {
        let apply = Arc::new(FakeApplyRepo::default());
        let documents = Arc::new(FakeDocumentsRepo::default());
        let mut application = application();
        application.status = AppStatus::Applied;
        apply.seed_application(application.clone());

        let error = service(&apply, &documents)
            .mark_applied(&application, &offer())
            .unwrap_err();

        assert_eq!(error.message_key(), "errors.tracker.same_status");
        assert!(apply.events().is_empty(), "a refused move writes nothing");
    }
}
