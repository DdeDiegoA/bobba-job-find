//! Studio tests (S7.7): the read path, the inline edit, the render path, the
//! DOCX export and the per-section regeneration.
//!
//! Every port is a fake except the document store, whose in-memory
//! implementation is shared with the rest of `boba-app`
//! ([`crate::ports::repo::documents::fake`]).

use std::sync::Arc;

use boba_domain::application::{AppStatus, Application};
use boba_domain::documents::{
    Claim, Contact, CoverLetterDocument, CvDocument, DocumentKind, DocumentModel, Lang,
    LayoutReport, NewDocument, RenderedPdf, SCHEMA_VERSION, Section, TemplateId, TemplateInfo,
    TextLayerReport,
};
use boba_domain::posting::{ApplyHints, Offer, WorkMode};
use boba_domain::profile::{FactKind, ProfileFact, Provenance};

use super::*;
use crate::ports::repo::documents::fake::FakeDocumentsRepo;

struct FixedClock;

impl ClockPort for FixedClock {
    fn now_rfc3339(&self) -> String {
        "2026-10-17T02:00:00Z".to_string()
    }
}

/// A renderer that answers with fixed bytes and a one-page layout.
struct FakeRender {
    docx: Vec<u8>,
}

impl RenderPort for FakeRender {
    fn templates(&self) -> Vec<TemplateInfo> {
        // The four templates the Typst renderer registers (S7.1).
        [
            ("cv-classic", DocumentKind::Cv, true),
            ("cv-modern", DocumentKind::Cv, false),
            ("cover-classic", DocumentKind::CoverLetter, true),
            ("cover-modern", DocumentKind::CoverLetter, false),
        ]
        .into_iter()
        .map(|(id, kind, default)| TemplateInfo {
            id: TemplateId::new(id),
            kind,
            default,
        })
        .collect()
    }

    fn render_pdf(
        &self,
        _document: &DocumentModel,
        _template: &TemplateId,
        _locale: Lang,
    ) -> Result<RenderedPdf, RenderError> {
        Ok(RenderedPdf {
            bytes: b"%PDF-fake".to_vec(),
            layout: LayoutReport {
                pages: 1,
                orphans: 0,
            },
        })
    }

    fn render_docx(
        &self,
        _document: &DocumentModel,
        _template: &TemplateId,
        _locale: Lang,
    ) -> Result<Vec<u8>, RenderError> {
        Ok(self.docx.clone())
    }
}

/// A measurer whose page count follows the number of claims, so a cut that
/// removes a line really does shorten the document (S7.5 walks that loop).
struct FakeVerify {
    text_layer: TextLayerReport,
    /// Claims per page.
    per_page: usize,
}

impl FakeVerify {
    fn passing() -> Self {
        Self {
            text_layer: TextLayerReport::default(),
            per_page: 2,
        }
    }
}

impl VerifyPort for FakeVerify {
    fn layout(
        &self,
        document: &DocumentModel,
        _template: &TemplateId,
    ) -> Result<LayoutReport, VerifyError> {
        let claims = match document {
            DocumentModel::Cv(cv) => {
                cv.sections
                    .iter()
                    .map(|section| section.items.len())
                    .sum::<usize>()
                    + usize::from(cv.summary.is_some())
            }
            DocumentModel::CoverLetter(cover) => cover.paragraphs.len(),
        };
        Ok(LayoutReport {
            pages: claims.div_ceil(self.per_page).max(1) as u32,
            orphans: 0,
        })
    }

    fn text_layer(
        &self,
        _document: &DocumentModel,
        _rendered: &RenderedPdf,
        _required_keywords: &[String],
    ) -> Result<TextLayerReport, VerifyError> {
        Ok(self.text_layer.clone())
    }
}

fn service(documents: Arc<FakeDocumentsRepo>, verify: FakeVerify) -> StudioService {
    StudioService::new(
        documents,
        Arc::new(FakeRender {
            docx: b"PK-fake-docx".to_vec(),
        }),
        Arc::new(verify),
        Arc::new(FixedClock),
    )
}

fn contact() -> Contact {
    Contact {
        full_name: "Ada Lovelace".into(),
        headline: None,
        email: Some("ada@example.com".into()),
        phone: None,
        location: None,
        links: Vec::new(),
    }
}

fn cv() -> CvDocument {
    CvDocument {
        schema_version: SCHEMA_VERSION,
        lang: Lang::En,
        contact: contact(),
        summary: Some(Claim {
            text: "Backend developer.".into(),
            fact_ids: vec!["fact_1".into()],
        }),
        sections: vec![Section {
            heading: "Experience".into(),
            items: vec![
                Claim {
                    text: "Shipped a billing API.".into(),
                    fact_ids: vec!["fact_2".into()],
                },
                Claim {
                    text: "Led the platform team.".into(),
                    fact_ids: vec!["fact_3".into()],
                },
            ],
        }],
        declared_gaps: vec!["Kubernetes".into()],
    }
}

fn cover() -> CoverLetterDocument {
    CoverLetterDocument {
        schema_version: SCHEMA_VERSION,
        lang: Lang::En,
        contact: contact(),
        company: Some("Acme".into()),
        recipient: None,
        date: None,
        salutation: Some("Dear team,".into()),
        paragraphs: vec![Claim {
            text: "I would like to apply.".into(),
            fact_ids: vec!["fact_1".into()],
        }],
        closing: None,
        declared_gaps: Vec::new(),
    }
}

fn fact(id: &str, kind: FactKind, text: &str) -> ProfileFact {
    ProfileFact {
        id: id.to_string(),
        item_id: format!("item_{id}"),
        kind,
        text_en: Some(text.to_string()),
        text_es: Some(text.to_string()),
        evidence: None,
        detail_json: "{}".into(),
        supersedes: None,
        profile_version: 1,
        provenance: Provenance::User,
        agent_run_id: None,
        created_at: "2026-10-17T00:00:00Z".into(),
    }
}

fn facts() -> Vec<ProfileFact> {
    vec![
        fact("fact_1", FactKind::Identity, "Ada Lovelace"),
        fact(
            "fact_2",
            FactKind::Experience,
            "Shipped a billing API in Rust",
        ),
        fact("fact_3", FactKind::Experience, "Led the platform team"),
    ]
}

fn application() -> Application {
    Application {
        id: "app_1".into(),
        posting_id: "pst_1".into(),
        status: AppStatus::Saved,
        offer_snapshot_json: "{}".into(),
        notes: None,
        next_action: None,
        created_at: "2026-10-17T00:00:00Z".into(),
        updated_at: "2026-10-17T00:00:00Z".into(),
    }
}

fn offer() -> Offer {
    Offer {
        id: "pst_1".into(),
        source_id: "src_1".into(),
        external_id: "ext_1".into(),
        title: "Backend engineer".into(),
        company: "Acme".into(),
        locations: vec![],
        work_mode: WorkMode::Remote,
        money: None,
        posted_at: None,
        deadline: None,
        first_seen: "2026-10-17T00:00:00Z".into(),
        last_seen: "2026-10-17T00:00:00Z".into(),
        lang: "en".into(),
        description_html: "<p>Rust</p>".into(),
        description_text: "Rust".into(),
        requirements: vec!["Rust".into(), "Kubernetes".into()],
        apply: ApplyHints {
            url: "https://acme.example/jobs/1".into(),
            email: None,
        },
    }
}

/// Store one draft, as a tailoring run would have.
fn store_cv(documents: &FakeDocumentsRepo, model: &CvDocument) -> String {
    let id = format!("doc_{}", documents.rows().len() + 1);
    documents
        .insert(&NewDocument {
            id: id.clone(),
            application_id: "app_1".into(),
            kind: DocumentKind::Cv,
            lang: model.lang,
            template: TemplateId::new("cv-classic"),
            model_json: serde_json::to_string(model).expect("json"),
            language_fallback_from: None,
            untrusted_origin: true,
            created_at: "2026-10-17T00:00:00Z".into(),
        })
        .expect("insert");
    id
}

fn store_cover(documents: &FakeDocumentsRepo, model: &CoverLetterDocument) -> String {
    let id = format!("doc_{}", documents.rows().len() + 1);
    documents
        .insert(&NewDocument {
            id: id.clone(),
            application_id: "app_1".into(),
            kind: DocumentKind::CoverLetter,
            lang: model.lang,
            template: TemplateId::new("cover-classic"),
            model_json: serde_json::to_string(model).expect("json"),
            language_fallback_from: None,
            untrusted_origin: true,
            created_at: "2026-10-17T00:00:00Z".into(),
        })
        .expect("insert");
    id
}

fn budget(pages: u32) -> PageBudget {
    PageBudget::new(pages).expect("a 1-3 page budget")
}

fn view(service: &StudioService, documents: &FakeDocumentsRepo) -> StudioView {
    let _ = documents;
    service
        .view(&application(), &offer(), &facts(), budget(2))
        .expect("view")
}

// AC1: the screen reads both drafts with their versions, and a draft that has a
// previous version carries the changes since it.
#[test]
fn view_shows_both_drafts_their_previous_version_and_the_diff() {
    let documents = FakeDocumentsRepo::default();
    store_cv(&documents, &cv());
    let mut edited = cv();
    edited.sections[0].items[0].text = "Shipped the billing API.".into();
    store_cv(&documents, &edited);
    store_cover(&documents, &cover());
    let service = service(Arc::new(documents), FakeVerify::passing());

    let view = view(&service, &FakeDocumentsRepo::default());

    let cv = view.cv.expect("a CV");
    assert_eq!(cv.version, 2);
    assert_eq!(cv.previous.as_ref().map(|p| p.version), Some(1));
    assert_eq!(cv.diff.len(), 1, "only the edited claim differs");
    assert_eq!(cv.diff[0].slot, "Experience 1");
    assert_eq!(cv.diff[0].before.as_deref(), Some("Shipped a billing API."));
    assert_eq!(
        cv.diff[0].after.as_deref(),
        Some("Shipped the billing API.")
    );
    assert_eq!(cv.diff[0].change, DiffChange::Changed);

    let cover = view.cover.expect("a cover letter");
    assert_eq!(cover.version, 1);
    assert!(cover.previous.is_none());
    assert!(cover.diff.is_empty());
}

// AC1/FR-74: a claim no fact backs, or one naming a fact the Profile does not
// have, is not ready — and the trace names what is missing.
#[test]
fn view_marks_unbacked_and_unknown_claims_and_blocks_ready() {
    let documents = FakeDocumentsRepo::default();
    let mut model = cv();
    model.summary = Some(Claim {
        text: "Wrote a book.".into(),
        fact_ids: vec![],
    });
    model.sections[0].items[1].fact_ids = vec!["fact_nope".into()];
    store_cv(&documents, &model);
    let service = service(Arc::new(documents), FakeVerify::passing());

    let cv = view(&service, &FakeDocumentsRepo::default())
        .cv
        .expect("a CV");

    assert!(!cv.ready, "an unbacked claim blocks Ready");
    let summary = cv.text.summary.as_ref().expect("a summary");
    assert!(!summary.backed);
    assert!(summary.facts.is_empty());
    let second = &cv.text.sections[0].items[1];
    assert!(!second.backed);
    assert_eq!(second.unknown_facts, vec!["fact_nope".to_string()]);
    assert!(
        cv.text.sections[0].items[0].backed,
        "a backed claim stays green"
    );
    assert_eq!(
        cv.text.sections[0].items[0].facts[0].label, "Shipped a billing API in Rust",
        "the trace names the Profile fact, in the draft's language"
    );
}

// AC1/FR-75: the Offer's requirements are shown against the Profile; a gap is
// never covered.
#[test]
fn view_reports_requirement_coverage_and_declared_gaps() {
    let documents = FakeDocumentsRepo::default();
    store_cv(&documents, &cv());
    let service = service(Arc::new(documents), FakeVerify::passing());

    let view = view(&service, &FakeDocumentsRepo::default());

    assert_eq!(view.requirements.len(), 2);
    assert_eq!(view.requirements[0].requirement, "Rust");
    assert_eq!(view.requirements[0].covered, vec!["rust".to_string()]);
    assert!(view.requirements[0].gaps.is_empty());
    assert_eq!(view.requirements[1].requirement, "Kubernetes");
    assert_eq!(view.requirements[1].gaps, vec!["kubernetes".to_string()]);
    assert_eq!(
        view.page_budget, 2,
        "the studio shows the budget its checks use"
    );
}

// FR-70: an Offer in another language drafted in EN carries the reason, so the
// studio can explain the language the user did not expect.
#[test]
fn view_shows_the_language_fallback_notice() {
    let documents = FakeDocumentsRepo::default();
    documents
        .insert(&NewDocument {
            id: "doc_fallback".into(),
            application_id: "app_1".into(),
            kind: DocumentKind::Cv,
            lang: Lang::En,
            template: TemplateId::new("cv-classic"),
            model_json: serde_json::to_string(&cv()).expect("json"),
            language_fallback_from: Some("de".into()),
            untrusted_origin: true,
            created_at: "2026-10-17T00:00:00Z".into(),
        })
        .expect("insert");
    let service = service(Arc::new(documents), FakeVerify::passing());

    let cv = view(&service, &FakeDocumentsRepo::default())
        .cv
        .expect("a CV");
    assert_eq!(cv.language_fallback_from.as_deref(), Some("de"));
}

// S7.4: the studio shows the newest critique, and which version it judged — a
// critique stays readable after a revision made a newer version.
#[test]
fn view_shows_the_newest_review_with_the_version_it_judged() {
    let documents = FakeDocumentsRepo::default();
    store_cv(&documents, &cv());
    let findings = serde_json::json!([
        {"severity": "low", "target": "cv", "message": "Minor.", "suggestion": "Tighten it."},
        {"severity": "high", "target": "cv", "message": "No metric on the billing line."}
    ]);
    documents
        .set_review("app_1", &findings.to_string(), "t")
        .expect("review");
    // A revision stores a newer version that carries no critique of its own.
    let mut revised = cv();
    revised.summary = Some(Claim {
        text: "Backend developer with eight years.".into(),
        fact_ids: vec!["fact_1".into()],
    });
    store_cv(&documents, &revised);
    let service = service(Arc::new(documents), FakeVerify::passing());

    let cv = view(&service, &FakeDocumentsRepo::default())
        .cv
        .expect("a CV");
    let review = cv.review.expect("the critique the reviewer wrote");
    assert_eq!(review.document_version, 1, "it judged version 1");
    assert_eq!(review.findings.len(), 2);
    assert_eq!(
        review.findings[0].severity,
        Severity::High,
        "most serious first"
    );
}

// AC1: an application with no drafts yet is a valid studio — the templates and
// the coverage are still there.
#[test]
fn view_with_no_drafts_still_lists_templates_and_coverage() {
    let service = service(
        Arc::new(FakeDocumentsRepo::default()),
        FakeVerify::passing(),
    );

    let view = view(&service, &FakeDocumentsRepo::default());

    assert!(view.cv.is_none());
    assert!(view.cover.is_none());
    assert_eq!(view.templates.len(), 4);
    assert_eq!(view.requirements.len(), 2);
}

// AC1 + the S7.7 taint ruling: an inline edit is a new version, the user's text
// is stored, the claim keeps its facts — and the draft is **not** promoted out
// of `untrusted_origin` by an edit.
#[test]
fn edit_stores_a_new_version_keeps_facts_and_inherits_the_taint() {
    let documents = Arc::new(FakeDocumentsRepo::default());
    store_cv(&documents, &cv());
    let service = service(documents.clone(), FakeVerify::passing());

    service
        .edit(
            &application(),
            &offer(),
            &facts(),
            budget(2),
            &EditInput {
                application_id: "app_1".into(),
                kind: StudioKind::Cv,
                target: EditTarget::CvItem {
                    section: 0,
                    item: 0,
                },
                text: "Shipped the billing API for 2M users.".into(),
            },
        )
        .expect("edit");

    let rows = documents.rows();
    assert_eq!(rows.len(), 2, "the edit is a new version");
    let edited = &rows[1];
    assert!(
        edited.untrusted_origin,
        "an edit inherits its parent's taint"
    );
    let model: CvDocument = serde_json::from_str(&edited.model_json).expect("json");
    assert_eq!(
        model.sections[0].items[0].text,
        "Shipped the billing API for 2M users."
    );
    assert_eq!(
        model.sections[0].items[0].fact_ids,
        vec!["fact_2".to_string()],
        "the user rewrote the sentence, not what it stands on"
    );
}

// AC1: the edit response carries the re-run checks and the diff against the
// version the user was reading.
#[test]
fn edit_returns_the_rerun_checks_and_the_diff() {
    let documents = Arc::new(FakeDocumentsRepo::default());
    store_cv(&documents, &cv());
    let service = service(documents.clone(), FakeVerify::passing());

    let view = service
        .edit(
            &application(),
            &offer(),
            &facts(),
            budget(2),
            &EditInput {
                application_id: "app_1".into(),
                kind: StudioKind::Cv,
                target: EditTarget::Summary,
                text: "Backend developer with eight years.".into(),
            },
        )
        .expect("edit");

    let cv = view.cv.expect("a CV");
    assert_eq!(cv.version, 2);
    assert_eq!(cv.previous.as_ref().map(|p| p.version), Some(1));
    assert_eq!(cv.diff.len(), 1);
    assert_eq!(cv.diff[0].slot, "Summary");
    assert_eq!(
        cv.diff[0].after.as_deref(),
        Some("Backend developer with eight years.")
    );
    let checks = cv.checks.expect("the checks ran again");
    assert!(checks.verified, "the fake renderer passes every check");
    assert!(checks.fabrication_ready);
}

// AC1: the checks re-run on the new version, so an edit that breaks the
// fabrication rule is caught before the user can send it.
#[test]
fn an_edit_that_breaks_the_fabrication_rule_is_not_verified() {
    let documents = Arc::new(FakeDocumentsRepo::default());
    let mut model = cv();
    model.summary = Some(Claim {
        text: "Backend developer.".into(),
        fact_ids: vec!["fact_1".into()],
    });
    store_cv(&documents, &model);
    let service = service(documents.clone(), FakeVerify::passing());
    // The user's edit is stored as sent; what the *next read* can no longer
    // claim is that the draft is verified: the checks below say so.
    let mut edited = model.clone();
    edited.summary.as_mut().expect("summary").fact_ids = vec!["fact_gone".into()];
    let id = store_cv(&documents, &edited);
    documents
        .mark_verified(&id, "{}", "t")
        .expect("a stored verdict");

    let cv = view(&service, &FakeDocumentsRepo::default())
        .cv
        .expect("a CV");
    assert!(!cv.ready, "a fact the Profile does not have blocks Ready");
}

#[test]
fn edit_refuses_an_empty_claim_or_one_the_draft_does_not_have() {
    let documents = Arc::new(FakeDocumentsRepo::default());
    store_cv(&documents, &cv());
    let service = service(documents.clone(), FakeVerify::passing());

    let bad = |target: EditTarget, text: &str| EditInput {
        application_id: "app_1".into(),
        kind: StudioKind::Cv,
        target,
        text: text.to_string(),
    };

    assert_eq!(
        service
            .edit(
                &application(),
                &offer(),
                &facts(),
                budget(2),
                &bad(EditTarget::Summary, "   "),
            )
            .expect_err("an empty claim is not a claim"),
        StudioError::InvalidEdit
    );
    assert_eq!(
        service
            .edit(
                &application(),
                &offer(),
                &facts(),
                budget(2),
                &bad(
                    EditTarget::CvItem {
                        section: 9,
                        item: 9
                    },
                    "Whatever."
                ),
            )
            .expect_err("no such claim"),
        StudioError::InvalidEdit
    );
    assert_eq!(documents.rows().len(), 1, "a refused edit writes nothing");
}

#[test]
fn editing_an_application_with_no_draft_is_a_typed_error() {
    let service = service(
        Arc::new(FakeDocumentsRepo::default()),
        FakeVerify::passing(),
    );
    let error = service
        .edit(
            &application(),
            &offer(),
            &facts(),
            budget(2),
            &EditInput {
                application_id: "app_1".into(),
                kind: StudioKind::Cv,
                target: EditTarget::Summary,
                text: "Anything.".into(),
            },
        )
        .expect_err("no draft to edit");
    assert_eq!(
        error.message_key(),
        "errors.studio.no_draft",
        "the UI shows a translated message"
    );
}

// AC1/S7.5: a CV that lays out past its budget is cut, and the cut is written as
// a new version — the user reads what was verified.
#[test]
fn render_fits_the_cv_and_writes_the_cut_as_a_new_version() {
    let documents = Arc::new(FakeDocumentsRepo::default());
    store_cv(&documents, &cv());
    // Two claims per page: the three-claim CV lays out to two pages.
    let service = service(documents.clone(), FakeVerify::passing());

    let view = service
        .render(&application(), &offer(), &facts(), budget(1))
        .expect("render");

    let rows = documents.rows();
    assert_eq!(rows.len(), 2, "the cut is a new version");
    let fitted: CvDocument = serde_json::from_str(&rows[1].model_json).expect("json");
    let claims = fitted.sections.iter().map(|s| s.items.len()).sum::<usize>()
        + usize::from(fitted.summary.is_some());
    assert!(claims <= 2, "the fitted CV fits one page: {claims} claims");
    assert!(rows[1].untrusted_origin, "a cut never promotes a draft");

    let cv = view.cv.expect("a CV");
    assert_eq!(cv.version, 2);
    let checks = cv.checks.expect("the render stored its verdict");
    assert_eq!(checks.cut.len(), 1, "the removed line is shown");
    assert!(checks.verified, "the fitted CV passed every check");
    assert!(
        documents.stored_pdf(&cv.id).is_some(),
        "the PDF of the verified version is stored"
    );
}

// AD-14: `verified_at` is what the tracker reads, so it is written **only** when
// every check passes.
#[test]
fn render_stores_a_verification_record_only_when_every_check_passes() {
    let documents = Arc::new(FakeDocumentsRepo::default());
    store_cv(&documents, &cv());
    let mut verify = FakeVerify::passing();
    verify.text_layer = TextLayerReport {
        missing_keywords: vec!["rust".into()],
        ..TextLayerReport::default()
    };
    let service = service(documents.clone(), verify);

    let view = service
        .render(&application(), &offer(), &facts(), budget(2))
        .expect("render");

    let cv = view.cv.expect("a CV");
    let checks = cv.checks.expect("the failed checks are still shown");
    assert!(!checks.text_layer.passed);
    assert!(!checks.verified);
    assert_eq!(
        documents.verification(&cv.id),
        None,
        "an unverified render leaves no verification record"
    );
    assert!(
        documents.stored_pdf(&cv.id).is_some(),
        "the PDF is still rendered and stored"
    );
}

// AC2: DOCX export renders the newest version and stores the bytes next to the
// PDF.
#[test]
fn export_docx_returns_the_renderers_bytes_and_stores_them() {
    let documents = Arc::new(FakeDocumentsRepo::default());
    store_cv(&documents, &cv());
    let service = service(documents.clone(), FakeVerify::passing());

    let export = service
        .export_docx(&application(), StudioKind::Cv)
        .expect("export");

    assert_eq!(export.bytes, b"PK-fake-docx".to_vec());
    assert_eq!(export.version, 1);
    assert_eq!(export.file_name, "boba-app_1-cv-v1.docx");
    assert_eq!(
        documents.rows()[0].id,
        "doc_1",
        "the newest version is the one exported"
    );
}

#[test]
fn export_docx_without_that_draft_is_a_typed_error() {
    let service = service(
        Arc::new(FakeDocumentsRepo::default()),
        FakeVerify::passing(),
    );
    let error = service
        .export_docx(&application(), StudioKind::CoverLetter)
        .expect_err("no cover letter");
    assert_eq!(error.message_key(), "errors.studio.no_draft");
}

// AC1: regenerating a section copies just that section from the newest draft into
// the one the user was reading — as a new version, tainted by both parents.
#[test]
fn replace_section_copies_only_that_section_from_the_newest_draft() {
    let documents = Arc::new(FakeDocumentsRepo::default());
    let reading = store_cv(&documents, &cv());
    let mut regenerated = cv();
    regenerated.sections[0].items = vec![Claim {
        text: "Shipped the billing API for 2M users.".into(),
        fact_ids: vec!["fact_2".into()],
    }];
    regenerated.summary = Some(Claim {
        text: "A different summary the user did not ask for.".into(),
        fact_ids: vec!["fact_1".into()],
    });
    store_cv(&documents, &regenerated);
    let service = service(documents.clone(), FakeVerify::passing());

    let view = service
        .replace_section(
            &application(),
            &offer(),
            &facts(),
            budget(2),
            &RegenerateInput {
                application_id: "app_1".into(),
                kind: StudioKind::Cv,
                section: Some(0),
                into: reading.clone(),
            },
        )
        .expect("regenerate one section");

    let rows = documents.rows();
    assert_eq!(rows.len(), 3);
    let spliced: CvDocument = serde_json::from_str(&rows[2].model_json).expect("json");
    assert_eq!(
        spliced.sections[0].items.len(),
        1,
        "the regenerated section replaces the old one whole"
    );
    assert_eq!(
        spliced.sections[0].items[0].text,
        "Shipped the billing API for 2M users."
    );
    assert_eq!(
        spliced
            .summary
            .as_ref()
            .map(|summary| summary.text.as_str()),
        Some("Backend developer."),
        "the summary of the version the user was reading is untouched"
    );
    assert!(rows[2].untrusted_origin);
    let cv = view.cv.expect("a CV");
    assert_eq!(cv.version, 3, "the splice is the newest version");
    assert!(
        !cv.diff.is_empty(),
        "the diff shows what regeneration changed"
    );
}

// AC1: the summary regenerates the same way.
#[test]
fn replace_section_of_the_summary_copies_the_summary() {
    let documents = Arc::new(FakeDocumentsRepo::default());
    let reading = store_cv(&documents, &cv());
    let mut regenerated = cv();
    regenerated.summary = Some(Claim {
        text: "Backend developer with eight years of APIs.".into(),
        fact_ids: vec!["fact_1".into()],
    });
    store_cv(&documents, &regenerated);
    let service = service(documents.clone(), FakeVerify::passing());

    service
        .replace_section(
            &application(),
            &offer(),
            &facts(),
            budget(2),
            &RegenerateInput {
                application_id: "app_1".into(),
                kind: StudioKind::Cv,
                section: None,
                into: reading.clone(),
            },
        )
        .expect("regenerate the summary");

    let spliced: CvDocument = serde_json::from_str(&documents.rows()[2].model_json).expect("json");
    assert_eq!(
        spliced
            .summary
            .as_ref()
            .map(|summary| summary.text.as_str()),
        Some("Backend developer with eight years of APIs.")
    );
    assert_eq!(
        spliced.sections[0].items[0].text, "Shipped a billing API.",
        "the sections of the version the user was reading stay"
    );
}

// AC1: for a cover letter, a "section" is one paragraph.
#[test]
fn replace_section_of_a_cover_letter_copies_one_paragraph() {
    let documents = Arc::new(FakeDocumentsRepo::default());
    let reading = store_cover(&documents, &cover());
    let mut regenerated = cover();
    regenerated.paragraphs = vec![
        Claim {
            text: "I would like to apply for the backend role.".into(),
            fact_ids: vec!["fact_1".into()],
        },
        Claim {
            text: "I built the billing API you describe.".into(),
            fact_ids: vec!["fact_2".into()],
        },
    ];
    store_cover(&documents, &regenerated);
    let service = service(documents.clone(), FakeVerify::passing());

    service
        .replace_section(
            &application(),
            &offer(),
            &facts(),
            budget(2),
            &RegenerateInput {
                application_id: "app_1".into(),
                kind: StudioKind::CoverLetter,
                section: Some(1),
                into: reading.clone(),
            },
        )
        .expect("regenerate one paragraph");

    let spliced: CoverLetterDocument =
        serde_json::from_str(&documents.rows()[2].model_json).expect("json");
    assert_eq!(spliced.paragraphs.len(), 2);
    assert_eq!(
        spliced.paragraphs[1].text,
        "I built the billing API you describe."
    );
}

#[test]
fn replace_section_of_a_section_neither_draft_has_is_refused() {
    let documents = Arc::new(FakeDocumentsRepo::default());
    let reading = store_cv(&documents, &cv());
    store_cv(&documents, &cv());
    let service = service(documents.clone(), FakeVerify::passing());

    let error = service
        .replace_section(
            &application(),
            &offer(),
            &facts(),
            budget(2),
            &RegenerateInput {
                application_id: "app_1".into(),
                kind: StudioKind::Cv,
                section: Some(7),
                into: reading.clone(),
            },
        )
        .expect_err("no such section");
    assert_eq!(error.message_key(), "errors.studio.invalid_edit");
}

// S7.7: a machine with exactly one detected CLI needs no choice; none or
// several is the engine step's question (S3.2), so the studio refuses instead of
// guessing.
#[test]
fn the_generation_run_uses_the_only_installed_cli() {
    assert_eq!(
        pick_cli(&["claude_code".to_string()]),
        Ok("claude_code".to_string())
    );
    assert_eq!(
        pick_cli(&[]).unwrap_err().message_key(),
        "errors.studio.no_engine"
    );
    assert_eq!(
        pick_cli(&["claude_code".to_string(), "codex".to_string()])
            .unwrap_err()
            .message_key(),
        "errors.studio.ambiguous_engine"
    );
}
