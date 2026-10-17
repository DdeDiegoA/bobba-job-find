//! The studio use case (S7.7, FR-78/FR-79, EXPERIENCE §9.5).
//!
//! The studio is where the user makes a tailored draft their own: every claim
//! with the Profile facts behind it (S7.3), the reviewer's findings (S7.4), the
//! coverage of the Offer's requirements (S7.6) and the checks a render must pass
//! (S7.5/S7.6). This module owns the four things the screen asks the core to do:
//!
//! * **read** — [`StudioService::view`]: the newest draft of each kind with its
//!   previous version, the diff between them, the validator's verdict on every
//!   claim, the requirement coverage and the checks the last render stored;
//! * **edit inline** — [`StudioService::edit`]: one claim's text becomes a new
//!   document version. The user's words are theirs, but the **facts** the claim
//!   stands on are not: the new version inherits its parent's `untrusted_origin`
//!   (ADR-010 — an edited draft still carries agent-written and posting-derived
//!   text), so an edit never promotes a draft;
//! * **the render path** — [`StudioService::render`]: the CV is fitted to its
//!   page budget (S7.5), the PDF rendered, the ATS text layer read back (S7.6)
//!   and the verdict stored on the document. A cut that changed the draft is a
//!   new version, so what the user reads is what was verified;
//! * **export** — [`StudioService::export_docx`]: the same document as a DOCX
//!   (FR-78), stored next to the PDF.
//!
//! Regenerating a section is the same write path: the generation run stores a
//! fresh draft, and [`StudioService::replace_section`] copies just the section
//! the user asked for into the draft they were reading, as a new version that
//! inherits the taint of **both** parents.
//!
//! Only a passing render is written to the document (AD-14): `verified_at` marks
//! a document nobody may send, so a run that fails a check leaves no verification
//! record behind. The screen still gets the failed checks in that same response;
//! a reload shows the fabrication verdict (which needs no render) and no render
//! record until the draft passes.

use std::sync::Arc;

use boba_domain::application::Application;
use boba_domain::documents::{
    Contact, CoverLetterDocument, CvDocument, DocumentKind, DocumentModel, DocumentRecord, Lang,
    NewDocument, new_document_id,
};
use boba_domain::keywords::{self, backed_keywords};
use boba_domain::posting::Offer;
use boba_domain::profile::ProfileFact;
use boba_domain::validator::{self, ClaimSlot, ValidationReport};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::fitting::{self, FitError, PageBudget};
use crate::ports::clock::ClockPort;
use crate::ports::render::{RenderError, RenderPort};
use crate::ports::repo::documents::{DocumentsRepo, DocumentsRepoError};
use crate::ports::verify::{VerifyError, VerifyPort};
use crate::review::{Finding, Severity};

/// Which document of the application the studio is acting on, as the IPC layer
/// carries it (`boba_domain::DocumentKind` is not a wire type).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum StudioKind {
    Cv,
    CoverLetter,
}

impl StudioKind {
    pub const fn kind(self) -> DocumentKind {
        match self {
            StudioKind::Cv => DocumentKind::Cv,
            StudioKind::CoverLetter => DocumentKind::CoverLetter,
        }
    }
}

impl From<DocumentKind> for StudioKind {
    fn from(kind: DocumentKind) -> Self {
        match kind {
            DocumentKind::Cv => StudioKind::Cv,
            DocumentKind::CoverLetter => StudioKind::CoverLetter,
        }
    }
}

/// One Profile fact behind a sentence, as the trace panel names it (FR-73).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TraceFactView {
    pub id: String,
    /// The fact's text in the draft's language; the id when it has no variant.
    pub label: String,
}

/// One claim of a draft with what backs it (FR-73/FR-74).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ClaimView {
    pub text: String,
    pub fact_ids: Vec<String>,
    pub facts: Vec<TraceFactView>,
    /// Whether the Profile backs every fact the claim names.
    pub backed: bool,
    /// Fact ids the Profile does not have, each once (FR-74).
    pub unknown_facts: Vec<String>,
}

/// A section of a CV, as the editor shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SectionView {
    pub heading: String,
    pub items: Vec<ClaimView>,
}

/// The editable content of a draft: the same claims the validator walked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DraftText {
    pub summary: Option<ClaimView>,
    pub sections: Vec<SectionView>,
    /// Cover letters only; empty for a CV.
    pub paragraphs: Vec<ClaimView>,
}

/// The layout facts of the last render (S7.5/S7.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct LayoutView {
    pub pages: u32,
    pub orphans: u32,
    pub page_budget: u32,
    /// FR-76: a CV inside its budget, a cover letter exactly one page, and
    /// neither ending on a heading with nothing under it.
    pub passes: bool,
}

/// What an ATS reads back out of the rendered PDF (S7.6, FR-77).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TextLayerView {
    pub passed: bool,
    pub missing_contact: Vec<String>,
    pub missing_keywords: Vec<String>,
    pub out_of_order: Vec<String>,
    pub undecodable: Vec<String>,
}

/// A line the fit removed, and why it went first (S7.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CutView {
    pub section: Option<String>,
    pub text: String,
    pub score: f32,
}

/// The checks one version of a draft must pass before it may be sent (AD-14).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct ChecksView {
    /// FR-74: every claim stands on a fact the Profile has.
    pub fabrication_ready: bool,
    pub layout: LayoutView,
    pub text_layer: TextLayerView,
    /// The document is `verified`: layout, ATS and fabrication all pass.
    pub verified: bool,
    /// The lines the fit removed to get the CV inside its budget.
    pub cut: Vec<CutView>,
}

/// One change between the previous draft and the current one (AC1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum DiffChange {
    Added,
    Removed,
    Changed,
}

/// A claim that differs between two versions, with the slot it sits in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DiffLine {
    /// `Summary`, the section heading and its number, or `Paragraph 3`: where it
    /// sits, so the toggle can point at the sentence.
    pub slot: String,
    pub before: Option<String>,
    pub after: Option<String>,
    pub change: DiffChange,
}

/// The previous version of a draft, as the studio links to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct PreviousVersion {
    pub id: String,
    pub version: u32,
}

/// The reviewer's findings and the version they judged (S7.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ReviewView {
    /// The version the critique was written for.
    pub document_id: String,
    pub document_version: u32,
    pub findings: Vec<Finding>,
}

/// One draft of the application, with everything the studio shows about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct StudioDocument {
    pub id: String,
    pub kind: StudioKind,
    pub version: u32,
    pub lang: String,
    pub template: String,
    pub created_at: String,
    /// Agent-written text that read the posting, until a version no longer is
    /// (ADR-010). The studio renders such a draft as data only.
    pub untrusted: bool,
    /// The Offer's language when it was neither ES nor EN (FR-70). The studio
    /// shows it as the reason for the language the user did not expect.
    pub language_fallback_from: Option<String>,
    pub text: DraftText,
    /// FR-74: no claim is missing a fact. *Ready* and *Send* stay blocked until
    /// this is true **and** the document is verified.
    pub ready: bool,
    /// The checks of the last passing render, `None` until the draft passed.
    pub checks: Option<ChecksView>,
    pub previous: Option<PreviousVersion>,
    /// The changes since [`Self::previous`], empty when both say the same thing.
    pub diff: Vec<DiffLine>,
    /// The newest critique of this kind, with the version it judged.
    pub review: Option<ReviewView>,
}

/// One requirement of the Offer with what the Profile backs of it (FR-75).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RequirementView {
    pub requirement: String,
    pub covered: Vec<String>,
    pub gaps: Vec<String>,
}

/// A template the user can render with (FR-71).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TemplateView {
    pub id: String,
    pub kind: StudioKind,
    pub default: bool,
}

/// Everything the studio screen reads for one application (EXPERIENCE §9.5).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct StudioView {
    pub application_id: String,
    pub posting_id: String,
    pub page_budget: u32,
    pub templates: Vec<TemplateView>,
    /// The Offer's requirements against the Profile, in the draft's language.
    pub requirements: Vec<RequirementView>,
    pub cv: Option<StudioDocument>,
    pub cover: Option<StudioDocument>,
}

/// Where an inline edit lands. The target is a claim, never a heading: a heading
/// carries no facts, and editing one changes nothing the validator reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "slot", rename_all = "snake_case")]
pub enum EditTarget {
    /// The CV summary line.
    Summary,
    /// Item `item` of section `section`.
    CvItem { section: u32, item: u32 },
    /// Paragraph `index` of the cover letter.
    CoverParagraph { index: u32 },
}

/// One section regeneration the user asked for (AC1): the section to copy from
/// the newest draft into the version the user is reading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RegenerateInput {
    pub application_id: String,
    pub kind: StudioKind,
    /// A CV section, or `None` for the CV summary; for a cover letter, the
    /// paragraph's index.
    pub section: Option<u32>,
    /// The document the user was reading; the copy lands on top of it.
    pub into: String,
}

/// One inline edit the user made (AC1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EditInput {
    pub application_id: String,
    pub kind: StudioKind,
    pub target: EditTarget,
    pub text: String,
}

/// The DOCX export of one document (FR-78).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocxExport {
    pub bytes: Vec<u8>,
    pub version: u32,
    /// A file name safe on every platform: no path separator, no space.
    pub file_name: String,
}

/// Why the studio refused an action.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StudioError {
    /// The application has no draft of that kind yet.
    #[error("`{application_id}` has no {kind} draft")]
    NoDraft {
        application_id: String,
        kind: &'static str,
    },
    /// An edit named a claim the draft does not have, or would empty it.
    #[error("this edit names a claim the draft does not have, or would empty it")]
    InvalidEdit,
    /// No agent CLI is installed, so no run can start.
    #[error("no agent CLI is installed")]
    NoEngine,
    /// More than one agent CLI is installed: the engine step (S3.2) is what
    /// picks one, so the studio refuses to guess.
    #[error("more than one agent CLI is installed; the engine step picks one")]
    AmbiguousEngine,
    /// The stored model is not a document this build can read.
    #[error("the stored draft could not be read: {0}")]
    Model(String),
    #[error("document store error: {0}")]
    Repo(#[from] DocumentsRepoError),
    #[error(transparent)]
    Render(#[from] RenderError),
    #[error(transparent)]
    Verify(#[from] VerifyError),
    #[error(transparent)]
    Fit(#[from] FitError),
}

impl StudioError {
    /// The translated message key the UI shows (CH7: `errors.<area>.*`).
    pub fn message_key(&self) -> &'static str {
        match self {
            StudioError::NoDraft { .. } => "errors.studio.no_draft",
            StudioError::InvalidEdit => "errors.studio.invalid_edit",
            StudioError::NoEngine => "errors.studio.no_engine",
            StudioError::AmbiguousEngine => "errors.studio.ambiguous_engine",
            StudioError::Model(_) => "errors.studio.stored_draft",
            StudioError::Repo(_) => "errors.studio.store",
            StudioError::Render(_) => "errors.studio.render",
            StudioError::Verify(_) => "errors.studio.verify",
            StudioError::Fit(_) => "errors.studio.fit",
        }
    }
}

/// A stored version as the wire carries it: specta forbids `i64` (a JS number
/// cannot hold it), and a `document.version` never comes near `u32::MAX`.
fn version_of(version: i64) -> u32 {
    u32::try_from(version).unwrap_or(u32::MAX)
}

/// The studio over the document store, the renderers and the clock.
pub struct StudioService {
    documents: Arc<dyn DocumentsRepo>,
    render: Arc<dyn RenderPort>,
    verify: Arc<dyn VerifyPort>,
    clock: Arc<dyn ClockPort>,
}

impl StudioService {
    pub fn new(
        documents: Arc<dyn DocumentsRepo>,
        render: Arc<dyn RenderPort>,
        verify: Arc<dyn VerifyPort>,
        clock: Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            documents,
            render,
            verify,
            clock,
        }
    }

    /// The studio screen: both drafts as they stand, with their checks (AC1).
    pub fn view(
        &self,
        application: &Application,
        offer: &Offer,
        facts: &[ProfileFact],
        budget: PageBudget,
    ) -> Result<StudioView, StudioError> {
        let cv = self.document(&application.id, DocumentKind::Cv, facts)?;
        let cover = self.document(&application.id, DocumentKind::CoverLetter, facts)?;
        let lang = cv
            .as_ref()
            .map(|doc| doc.lang.clone())
            .or_else(|| cover.as_ref().map(|doc| doc.lang.clone()))
            .unwrap_or_else(|| Lang::for_offer(&offer.lang, None).lang.code().to_string());
        Ok(StudioView {
            application_id: application.id.clone(),
            posting_id: application.posting_id.clone(),
            page_budget: budget.pages(),
            templates: templates(self.render.as_ref()),
            requirements: coverage(&offer.requirements, facts, &lang),
            cv,
            cover,
        })
    }

    /// Edit one claim inline (AC1): a new version, the checks re-run, and the
    /// diff against the version the user was reading.
    pub fn edit(
        &self,
        application: &Application,
        offer: &Offer,
        facts: &[ProfileFact],
        budget: PageBudget,
        input: &EditInput,
    ) -> Result<StudioView, StudioError> {
        let kind = input.kind.kind();
        let current = self.newest(&application.id, kind)?;
        if input.text.trim().is_empty() {
            return Err(StudioError::InvalidEdit);
        }
        let model = model_of(&current)?;
        let edited = replace_claim_text(&model, &input.target, &input.text)?;
        // The edit belongs to the user, but the draft it edits does not: the
        // new version inherits its parent's taint (ADR-010, S7.7 ruling).
        self.write_version(&current, &edited, current.untrusted_origin)?;
        self.render(application, offer, facts, budget)
    }

    /// The render path (AC1, AD-14): fit the CV to its budget (S7.5), render the
    /// PDF, read its ATS text layer back (S7.6), and store the verdict on the
    /// version that was rendered.
    pub fn render(
        &self,
        application: &Application,
        offer: &Offer,
        facts: &[ProfileFact],
        budget: PageBudget,
    ) -> Result<StudioView, StudioError> {
        let mut fresh = Vec::new();
        if let Ok(current) = self.newest(&application.id, DocumentKind::Cv) {
            fresh.push(self.render_cv(&current, offer, facts, budget)?);
        }
        if let Ok(current) = self.newest(&application.id, DocumentKind::CoverLetter) {
            fresh.push(self.render_cover(&current, facts, budget)?);
        }
        let mut view = self.view(application, offer, facts, budget)?;
        for (id, checks) in fresh {
            for document in [&mut view.cv, &mut view.cover].into_iter().flatten() {
                if document.id == id {
                    document.checks = Some(checks.clone());
                }
            }
        }
        Ok(view)
    }

    /// Export the newest version of one kind as a DOCX (FR-78, AC2).
    pub fn export_docx(
        &self,
        application: &Application,
        kind: StudioKind,
    ) -> Result<DocxExport, StudioError> {
        let current = self.newest(&application.id, kind.kind())?;
        let model = model_of(&current)?;
        let bytes = self
            .render
            .render_docx(&model, &current.template, current.lang)?;
        self.documents
            .set_docx(&current.id, &bytes, &self.clock.now_rfc3339())?;
        Ok(DocxExport {
            file_name: format!(
                "boba-{}-{}-v{}.docx",
                file_stem(&application.id),
                kind.kind().as_str(),
                current.version
            ),
            bytes,
            version: version_of(current.version),
        })
    }

    /// Regenerate one section: copy the named section of the newest draft of
    /// `kind` into the draft `into` (the version the user was reading), as a new
    /// version, then re-run the checks (AC1).
    ///
    /// `section` names a CV section, or `None` for the CV summary; a cover letter
    /// regenerates one paragraph at a time. The new version carries the taint of
    /// **both** parents: the base draft and the regenerated section.
    pub fn replace_section(
        &self,
        application: &Application,
        offer: &Offer,
        facts: &[ProfileFact],
        budget: PageBudget,
        input: &RegenerateInput,
    ) -> Result<StudioView, StudioError> {
        let kind = input.kind.kind();
        let records = self.versions(&application.id, kind)?;
        let newest = records.last().ok_or_else(|| StudioError::NoDraft {
            application_id: application.id.clone(),
            kind: kind.as_str(),
        })?;
        let base = records
            .iter()
            .find(|record| record.id == input.into)
            .ok_or(StudioError::InvalidEdit)?;
        let model = splice(&model_of(base)?, &model_of(newest)?, input.section)?;
        let untrusted = base.untrusted_origin || newest.untrusted_origin;
        self.write_version(base, &model, untrusted)?;
        self.render(application, offer, facts, budget)
    }

    /// The CV half of the render path: fit it to the budget, render and verify
    /// it, and store the verdict.
    fn render_cv(
        &self,
        current: &DocumentRecord,
        offer: &Offer,
        facts: &[ProfileFact],
        budget: PageBudget,
    ) -> Result<(String, ChecksView), StudioError> {
        let DocumentModel::Cv(cv) = model_of(current)? else {
            return Err(StudioError::Model("a CV row holds a CV".to_string()));
        };
        let cover = match self.newest(&current.application_id, DocumentKind::CoverLetter) {
            Ok(record) => match model_of(&record)? {
                DocumentModel::CoverLetter(cover) => cover,
                DocumentModel::Cv(_) => empty_cover(cv.lang),
            },
            Err(StudioError::NoDraft { .. }) => empty_cover(cv.lang),
            Err(other) => return Err(other),
        };
        let lang = profile_lang(cv.lang.code());
        let outcome = fitting::fit_cv(
            &cv,
            &cover,
            &current.template,
            budget,
            &|claim| relevance(claim, &offer.requirements, lang),
            self.verify.as_ref(),
        )?;
        let cut: Vec<CutView> = outcome
            .cut
            .iter()
            .map(|line| CutView {
                section: line.section.clone(),
                text: line.text.clone(),
                score: line.score,
            })
            .collect();
        // A cut that changed the draft is a new version: the user must read the
        // draft the checks judged, not the one they wrote before the fit.
        let (record, model) = if cut.is_empty() {
            (current.clone(), DocumentModel::Cv(cv))
        } else {
            let model = DocumentModel::Cv(outcome.cv);
            let id = self.write_version(current, &model, current.untrusted_origin)?;
            let record = self.documents.get(&id)?.ok_or_else(|| {
                StudioError::Model("the fitted version was not stored".to_string())
            })?;
            (record, model)
        };
        let checks = self.measure(&record, &model, &offer.requirements, facts, budget, cut)?;
        Ok((record.id, checks))
    }

    /// The cover-letter half of the render path: no cutting — its rule is one
    /// page, and a letter that overflows warns instead of losing a line.
    fn render_cover(
        &self,
        current: &DocumentRecord,
        facts: &[ProfileFact],
        budget: PageBudget,
    ) -> Result<(String, ChecksView), StudioError> {
        let model = model_of(current)?;
        if !matches!(model, DocumentModel::CoverLetter(_)) {
            return Err(StudioError::Model(
                "a cover-letter row holds a cover letter".to_string(),
            ));
        }
        let checks = self.measure(current, &model, &[], facts, budget, Vec::new())?;
        Ok((current.id.clone(), checks))
    }

    /// Render one version, read its text layer and fabrication verdict, store
    /// the PDF, and record the verdict **only** when every check passed (AD-14:
    /// `verified_at` is what the tracker reads).
    fn measure(
        &self,
        record: &DocumentRecord,
        model: &DocumentModel,
        requirements: &[String],
        facts: &[ProfileFact],
        budget: PageBudget,
        cut: Vec<CutView>,
    ) -> Result<ChecksView, StudioError> {
        let lang = model.lang();
        let required = backed_keywords(requirements, facts, profile_lang(lang.code()));
        let rendered = self.render.render_pdf(model, &record.template, lang)?;
        let text_layer = self.verify.text_layer(model, &rendered, &required)?;
        let fabrication = validator::validate(model, &|id| fact(facts, id).is_some());
        let report = boba_domain::documents::VerificationReport {
            layout: rendered.layout,
            text_layer: text_layer.clone(),
            fabrication_ready: fabrication.is_ready(),
        };
        let kind = model.kind();
        let checks = ChecksView {
            fabrication_ready: fabrication.is_ready(),
            layout: LayoutView {
                pages: rendered.layout.pages,
                orphans: rendered.layout.orphans,
                page_budget: budget.pages(),
                passes: report.layout_passes(kind, budget.pages()),
            },
            text_layer: TextLayerView {
                passed: text_layer.passed(),
                missing_contact: text_layer.missing_contact.clone(),
                missing_keywords: text_layer.missing_keywords.clone(),
                out_of_order: text_layer.out_of_order.clone(),
                undecodable: text_layer.undecodable.clone(),
            },
            verified: report.is_verified(kind, budget.pages()),
            cut,
        };
        let now = self.clock.now_rfc3339();
        self.documents.set_pdf(&record.id, &rendered.bytes, &now)?;
        if checks.verified {
            let json = serde_json::to_string(&checks)
                .map_err(|error| StudioError::Model(error.to_string()))?;
            self.documents.mark_verified(&record.id, &json, &now)?;
        }
        Ok(checks)
    }

    /// The newest version of one kind, or [`StudioError::NoDraft`].
    fn newest(
        &self,
        application_id: &str,
        kind: DocumentKind,
    ) -> Result<DocumentRecord, StudioError> {
        self.versions(application_id, kind)?
            .pop()
            .ok_or_else(|| StudioError::NoDraft {
                application_id: application_id.to_string(),
                kind: kind.as_str(),
            })
    }

    /// Every version of one kind, oldest first.
    fn versions(
        &self,
        application_id: &str,
        kind: DocumentKind,
    ) -> Result<Vec<DocumentRecord>, StudioError> {
        Ok(self
            .documents
            .list_for_application(application_id)?
            .into_iter()
            .filter(|record| record.kind == kind)
            .collect())
    }

    /// One kind's newest draft, as the studio shows it.
    fn document(
        &self,
        application_id: &str,
        kind: DocumentKind,
        facts: &[ProfileFact],
    ) -> Result<Option<StudioDocument>, StudioError> {
        let records = self.versions(application_id, kind)?;
        let Some(current) = records.last() else {
            return Ok(None);
        };
        let previous = records.iter().rev().nth(1);
        let model = model_of(current)?;
        let report = validator::validate(&model, &|id| fact(facts, id).is_some());
        let previous_model = previous.map(model_of).transpose()?;

        Ok(Some(StudioDocument {
            id: current.id.clone(),
            kind: kind.into(),
            version: version_of(current.version),
            lang: current.lang.code().to_string(),
            template: current.template.as_str().to_string(),
            created_at: current.created_at.clone(),
            untrusted: current.untrusted_origin,
            language_fallback_from: current.language_fallback_from.clone(),
            text: draft_text(&model, &report, facts),
            ready: report.is_ready(),
            checks: checks_of(current),
            previous: previous.map(|record| PreviousVersion {
                id: record.id.clone(),
                version: version_of(record.version),
            }),
            diff: previous_model
                .as_ref()
                .map(|older| diff(older, &model))
                .unwrap_or_default(),
            review: review_of(&records)?,
        }))
    }

    /// Write `model` as the next version of `parent`'s application and kind.
    /// Taint is inherited unless a caller knows better: an edit, a cut or a
    /// copied section never promotes a draft out of `untrusted_origin` on its
    /// own (ADR-010, S7.7 ruling).
    fn write_version(
        &self,
        parent: &DocumentRecord,
        model: &DocumentModel,
        untrusted: bool,
    ) -> Result<String, StudioError> {
        let id = new_document_id();
        self.documents.insert(&NewDocument {
            id: id.clone(),
            application_id: parent.application_id.clone(),
            kind: parent.kind,
            lang: parent.lang,
            template: parent.template.clone(),
            model_json: json_of(model)?,
            language_fallback_from: parent.language_fallback_from.clone(),
            untrusted_origin: untrusted,
            created_at: self.clock.now_rfc3339(),
        })?;
        Ok(id)
    }
}

/// Which CLI a generation run starts on: the **only** one installed.
///
/// Choosing an engine is the engine step's job (S3.2). Until that preference
/// exists, a machine with one CLI needs no choice, and a machine with none or
/// several is asked to finish onboarding rather than have the core guess.
pub fn pick_cli(installed: &[String]) -> Result<String, StudioError> {
    match installed {
        [] => Err(StudioError::NoEngine),
        [only] => Ok(only.clone()),
        _ => Err(StudioError::AmbiguousEngine),
    }
}

/// The templates the user can pick, in the renderer's own order: the renderer
/// owns the registry (FR-71), so the studio never keeps a second list.
fn templates(render: &dyn RenderPort) -> Vec<TemplateView> {
    render
        .templates()
        .into_iter()
        .map(|template| TemplateView {
            id: template.id.as_str().to_string(),
            kind: template.kind.into(),
            default: template.default,
        })
        .collect()
}

/// How well a line answers the Offer (S7.5): the share of the Offer's
/// requirement keywords the line actually says. Deterministic and offline
/// (AD-8); the optional embeddings model is not this pass.
fn relevance(
    claim: &boba_domain::documents::Claim,
    requirements: &[String],
    lang: boba_domain::profile::Lang,
) -> f32 {
    let _ = lang;
    let keywords: Vec<String> = requirements
        .iter()
        .flat_map(|requirement| keywords::requirement_keywords(requirement))
        .collect();
    if keywords.is_empty() {
        return 0.0;
    }
    let words = keywords::words(&claim.text);
    let hits = keywords
        .iter()
        .filter(|keyword| words.iter().any(|word| word == *keyword))
        .count();
    hits as f32 / keywords.len() as f32
}

/// Copy one section of `source` into `base` (the version the user was reading):
/// a CV section, the CV summary, or one cover-letter paragraph. A paragraph the
/// base does not have yet is appended; a section it does not have is refused —
/// a regenerated section never restructures the draft.
fn splice(
    base: &DocumentModel,
    source: &DocumentModel,
    section: Option<u32>,
) -> Result<DocumentModel, StudioError> {
    match (base, source) {
        (DocumentModel::Cv(base_cv), DocumentModel::Cv(source_cv)) => {
            let mut cv = base_cv.clone();
            match section {
                None => {
                    let summary = source_cv.summary.clone().ok_or(StudioError::InvalidEdit)?;
                    cv.summary = Some(summary);
                }
                Some(index) => {
                    let index = index as usize;
                    let items = source_cv
                        .sections
                        .get(index)
                        .map(|section| section.items.clone())
                        .ok_or(StudioError::InvalidEdit)?;
                    let target = cv.sections.get_mut(index).ok_or(StudioError::InvalidEdit)?;
                    target.items = items;
                }
            }
            Ok(DocumentModel::Cv(cv))
        }
        (DocumentModel::CoverLetter(base_cover), DocumentModel::CoverLetter(source_cover)) => {
            let index = section.ok_or(StudioError::InvalidEdit)? as usize;
            let paragraph = source_cover
                .paragraphs
                .get(index)
                .cloned()
                .ok_or(StudioError::InvalidEdit)?;
            let mut cover = base_cover.clone();
            if index < cover.paragraphs.len() {
                cover.paragraphs[index] = paragraph;
            } else if index == cover.paragraphs.len() {
                cover.paragraphs.push(paragraph);
            } else {
                return Err(StudioError::InvalidEdit);
            }
            Ok(DocumentModel::CoverLetter(cover))
        }
        _ => Err(StudioError::InvalidEdit),
    }
}

/// An id as a file name: letters, digits, `-` and `_`, so the export can be
/// written on every platform without a separator or a reserved character.
fn file_stem(id: &str) -> String {
    id.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

/// The Offer's requirements against the Profile (FR-75). A fact backs a keyword
/// through its variant in the **draft's** language (AD-17), so the stored code is
/// turned into the Profile's own language type first.
fn coverage(requirements: &[String], facts: &[ProfileFact], lang: &str) -> Vec<RequirementView> {
    keywords::coverage(requirements, facts, profile_lang(lang))
        .into_iter()
        .map(|coverage| RequirementView {
            requirement: coverage.requirement,
            covered: coverage.covered,
            gaps: coverage.gaps,
        })
        .collect()
}

/// The content language a draft is written in, as the Profile's keywords read it.
fn profile_lang(code: &str) -> boba_domain::profile::Lang {
    if code == Lang::Es.code() {
        boba_domain::profile::Lang::Es
    } else {
        boba_domain::profile::Lang::En
    }
}

fn fact<'a>(facts: &'a [ProfileFact], id: &str) -> Option<&'a ProfileFact> {
    facts.iter().find(|fact| fact.id == id)
}

fn fact_label(fact: &ProfileFact, lang: Lang) -> String {
    let text = match lang {
        Lang::En => fact.text_en.as_ref().or(fact.text_es.as_ref()),
        Lang::Es => fact.text_es.as_ref().or(fact.text_en.as_ref()),
    };
    text.cloned().unwrap_or_else(|| fact.id.clone())
}

/// The editable text of a draft, claim by claim, with what backs each one.
fn draft_text(
    model: &DocumentModel,
    report: &ValidationReport,
    facts: &[ProfileFact],
) -> DraftText {
    let lang = model.lang();
    let claim = |trace: &boba_domain::validator::ClaimTrace| ClaimView {
        text: trace.text.clone(),
        fact_ids: trace.fact_ids.clone(),
        facts: trace
            .fact_ids
            .iter()
            .filter_map(|id| fact(facts, id))
            .map(|fact| TraceFactView {
                id: fact.id.clone(),
                label: fact_label(fact, lang),
            })
            .collect(),
        backed: trace.is_backed(),
        unknown_facts: match &trace.rejection {
            Some(boba_domain::validator::Rejection::UnknownFacts(ids)) => ids.clone(),
            _ => Vec::new(),
        },
    };

    let mut summary = None;
    let mut sections = headings(model);
    let mut paragraphs = Vec::new();
    for trace in &report.claims {
        match &trace.slot {
            ClaimSlot::Summary => summary = Some(claim(trace)),
            ClaimSlot::CvSection { section, .. } => {
                if let Some(view) = sections.get_mut(*section) {
                    view.items.push(claim(trace));
                }
            }
            ClaimSlot::CoverParagraph { .. } => paragraphs.push(claim(trace)),
        }
    }
    DraftText {
        summary,
        sections,
        paragraphs,
    }
}

/// A CV's section headings with no items yet: the claims are appended by slot.
fn headings(model: &DocumentModel) -> Vec<SectionView> {
    match model {
        DocumentModel::Cv(cv) => cv
            .sections
            .iter()
            .map(|section| SectionView {
                heading: section.heading.clone(),
                items: Vec::new(),
            })
            .collect(),
        DocumentModel::CoverLetter(_) => Vec::new(),
    }
}

/// The stored checks of one version, `None` until a render passed.
fn checks_of(record: &DocumentRecord) -> Option<ChecksView> {
    record
        .verification_json
        .as_ref()
        .and_then(|json| serde_json::from_str(json).ok())
}

/// The newest critique among these versions, with the version it judged.
fn review_of(records: &[DocumentRecord]) -> Result<Option<ReviewView>, StudioError> {
    for record in records.iter().rev() {
        let Some(json) = &record.review_json else {
            continue;
        };
        let findings: Vec<Finding> =
            serde_json::from_str(json).map_err(|error| StudioError::Model(error.to_string()))?;
        return Ok(Some(ReviewView {
            document_id: record.id.clone(),
            document_version: version_of(record.version),
            findings: ranked(findings),
        }));
    }
    Ok(None)
}

/// Findings, most serious first, keeping the reviewer's order within a severity
/// (the same ranking [`crate::review::Review::top_findings`] applies).
fn ranked(findings: Vec<Finding>) -> Vec<Finding> {
    let mut ranked = Vec::with_capacity(findings.len());
    for severity in Severity::ALL {
        ranked.extend(
            findings
                .iter()
                .filter(|finding| finding.severity == severity)
                .cloned(),
        );
    }
    ranked
}

/// The claims of two versions, paired by slot, with only the changes kept.
fn diff(older: &DocumentModel, current: &DocumentModel) -> Vec<DiffLine> {
    let before = claims_of(older);
    let after = claims_of(current);
    let mut lines = Vec::new();
    for index in 0..before.len().max(after.len()) {
        let old = before.get(index);
        let new = after.get(index);
        let slot = new
            .or(old)
            .map(|(slot, _)| slot.clone())
            .unwrap_or_default();
        match (old, new) {
            (Some((_, old_text)), Some((_, new_text))) => {
                if old_text != new_text {
                    lines.push(DiffLine {
                        slot,
                        before: Some(old_text.clone()),
                        after: Some(new_text.clone()),
                        change: DiffChange::Changed,
                    });
                }
            }
            (Some((_, old_text)), None) => lines.push(DiffLine {
                slot,
                before: Some(old_text.clone()),
                after: None,
                change: DiffChange::Removed,
            }),
            (None, Some((_, new_text))) => lines.push(DiffLine {
                slot,
                before: None,
                after: Some(new_text.clone()),
                change: DiffChange::Added,
            }),
            (None, None) => {}
        }
    }
    lines
}

/// Every claim of a draft with the slot label the diff shows.
fn claims_of(model: &DocumentModel) -> Vec<(String, String)> {
    let mut claims = Vec::new();
    match model {
        DocumentModel::Cv(cv) => {
            if let Some(summary) = &cv.summary {
                claims.push(("Summary".to_string(), summary.text.clone()));
            }
            for section in &cv.sections {
                for (index, item) in section.items.iter().enumerate() {
                    claims.push((
                        format!("{} {}", section.heading, index + 1),
                        item.text.clone(),
                    ));
                }
            }
        }
        DocumentModel::CoverLetter(cover) => {
            for (index, paragraph) in cover.paragraphs.iter().enumerate() {
                claims.push((format!("Paragraph {}", index + 1), paragraph.text.clone()));
            }
        }
    }
    claims
}

fn model_of(record: &DocumentRecord) -> Result<DocumentModel, StudioError> {
    match record.kind {
        DocumentKind::Cv => serde_json::from_str::<CvDocument>(&record.model_json)
            .map(DocumentModel::Cv)
            .map_err(|error| StudioError::Model(error.to_string())),
        DocumentKind::CoverLetter => {
            serde_json::from_str::<CoverLetterDocument>(&record.model_json)
                .map(DocumentModel::CoverLetter)
                .map_err(|error| StudioError::Model(error.to_string()))
        }
    }
}

fn json_of(model: &DocumentModel) -> Result<String, StudioError> {
    match model {
        DocumentModel::Cv(cv) => serde_json::to_string(cv),
        DocumentModel::CoverLetter(cover) => serde_json::to_string(cover),
    }
    .map_err(|error| StudioError::Model(error.to_string()))
}

/// The model with one claim's text replaced. The claim keeps its facts: the user
/// rewrote the sentence, not what it stands on (AD-15).
fn replace_claim_text(
    model: &DocumentModel,
    target: &EditTarget,
    text: &str,
) -> Result<DocumentModel, StudioError> {
    let mut model = model.clone();
    let claim = match (&mut model, target) {
        (DocumentModel::Cv(cv), EditTarget::Summary) => cv.summary.as_mut(),
        (DocumentModel::Cv(cv), EditTarget::CvItem { section, item }) => cv
            .sections
            .get_mut(*section as usize)
            .and_then(|section| section.items.get_mut(*item as usize)),
        (DocumentModel::CoverLetter(cover), EditTarget::CoverParagraph { index }) => {
            cover.paragraphs.get_mut(*index as usize)
        }
        _ => None,
    }
    .ok_or(StudioError::InvalidEdit)?;
    claim.text = text.to_string();
    Ok(model)
}

/// The contact block an empty document carries: nothing here is rendered, the
/// studio only reads it.
fn empty_contact() -> Contact {
    Contact {
        full_name: String::new(),
        headline: None,
        email: None,
        phone: None,
        location: None,
        links: Vec::new(),
    }
}

fn empty_cover(lang: Lang) -> CoverLetterDocument {
    CoverLetterDocument {
        schema_version: boba_domain::documents::SCHEMA_VERSION,
        lang,
        contact: empty_contact(),
        company: None,
        recipient: None,
        date: None,
        salutation: None,
        paragraphs: Vec::new(),
        closing: None,
        declared_gaps: Vec::new(),
    }
}

#[cfg(test)]
mod tests;
