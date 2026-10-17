//! ATS Form assist (S9.4, FR-82/FR-88, AD-10).
//!
//! The core fills the employer's hosted form in the user's own Chrome — headed,
//! on the Boba profile, only after an explicit opt-in — and the **user** presses
//! submit. What the core does is:
//!
//! * open the ATS page and read the form (through [`BrowserPort`]);
//! * fill it from the Package's verified documents and the answers the user
//!   approved (AD-14, S7.8), and from the CV's own contact block;
//! * stop and hand control back on a CAPTCHA, a login wall, or a required
//!   question it has no confident answer for (AC2);
//! * record an audit `intent` before the submission is in the user's hands,
//!   and — once the page confirms, on the form's own host — an `outcome`
//!   carrying the evidence: the confirmation text and the screenshot's blake3
//!   (AC3, FR-88, AD-12). The browser closes when the attempt ends.
//!
//! Page text never reaches an agent run: there is no path from a form page to
//! an LLM, so the untrusted-content gate is not even the boundary here (AD-7).
//!
//! **Needs check** is not a status: `application.status` keeps its ten-value
//! vocabulary. A submission without evidence is recorded as a failed `outcome`
//! with `needs_check: true`, and the card does not move (FR-88).

use std::collections::BTreeMap;

use boba_domain::application::{AppStatus, Application, BankAnswer};
use boba_domain::apply::channel::{self, AtsProvider};
use boba_domain::audit::AuditOutcome;
use boba_domain::documents::Contact;
use boba_domain::posting::Offer;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::answers::{self, AnswersError};
use crate::applications::{TrackerError, TrackerService, TransitionInput};
use crate::apply::manual::{ManualError, ManualService, PdfExport};
use crate::ports::browser::{BrowserError, BrowserPort, PageField, PageFieldKind, Upload};
use crate::ports::clock::ClockPort;
use crate::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use crate::ports::repo::documents::{DocumentsRepo, DocumentsRepoError};
use crate::ports::repo::profile::{ProfileRepo, ProfileRepoError};
use crate::ports::repo::system::{NewIntent, SystemRepo, SystemRepoError};

/// The setting that records the browser opt-in, and the version marker it
/// carries: re-confirming the warning writes a new marker.
pub const OPT_IN_KEY: &str = "apply.ats_browser";
pub const OPT_IN_MARKER: &str = "v1";

/// The audit action an ATS submission attempt writes (AD-12).
pub const AUDIT_ACTION: &str = "apply.ats_submitted";

/// The audit action the browser opt-in writes (AD-12: a settings change that
/// affects automation is audited).
pub const OPT_IN_ACTION: &str = "settings.apply.ats_browser";

/// Why an ATS assist step refused to run.
#[derive(Debug, thiserror::Error)]
pub enum AtsFormError {
    /// The Offer has no hosted ATS form to fill.
    #[error("this Offer has no ATS Form channel")]
    NotAtsForm,
    /// The user has not accepted the browser warning yet (AC1).
    #[error("browser assist is not turned on")]
    OptInRequired,
    /// A package draft is not Ready and verified (AD-14).
    #[error("not Ready and verified: {0:?}")]
    NotReady(Vec<String>),
    /// The attempt id names no unresolved submission.
    #[error("unknown ATS attempt: {0}")]
    UnknownAttempt(String),
    #[error(transparent)]
    Manual(#[from] ManualError),
    #[error(transparent)]
    Tracker(#[from] TrackerError),
    #[error(transparent)]
    Answers(#[from] AnswersError),
    #[error(transparent)]
    System(#[from] SystemRepoError),
    #[error(transparent)]
    Browser(#[from] BrowserError),
    #[error(transparent)]
    Repo(#[from] ApplyRepoError),
    #[error(transparent)]
    Documents(#[from] DocumentsRepoError),
    #[error(transparent)]
    Profile(#[from] ProfileRepoError),
}

impl AtsFormError {
    /// The translated message key the UI shows (CH7: `errors.<area>.*`).
    pub fn message_key(&self) -> &'static str {
        match self {
            AtsFormError::NotAtsForm => "errors.apply.ats_not_a_form",
            AtsFormError::OptInRequired => "errors.apply.ats_opt_in",
            AtsFormError::NotReady(_) => "errors.apply.not_ready",
            AtsFormError::UnknownAttempt(_) => "errors.apply.ats_unknown_attempt",
            AtsFormError::Manual(error) => error.message_key(),
            AtsFormError::Tracker(error) => error.message_key(),
            AtsFormError::Answers(error) => match error {
                AnswersError::UnknownAnswer(_) => "errors.apply.ats_unknown_answer",
                AnswersError::Edit(_) => "errors.apply.ats_answer_edit",
                AnswersError::UnknownApplication(_) => "errors.apply.unknown_application",
                AnswersError::Payload(_) | AnswersError::Repo(_) => "errors.apply.store",
            },
            AtsFormError::Browser(_) => "errors.apply.ats_browser",
            AtsFormError::System(_)
            | AtsFormError::Repo(_)
            | AtsFormError::Documents(_)
            | AtsFormError::Profile(_) => "errors.apply.store",
        }
    }
}

/// One field the core filled, as the user checks it before submitting (AC1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FilledField {
    pub handle: String,
    pub label: String,
    pub kind: String,
}

/// One field the fill left alone, so the user can see what it skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct UnansweredField {
    pub label: String,
    pub kind: String,
    pub required: bool,
}

/// Why automation stopped and the user took over (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum HandoffKind {
    Captcha,
    LoginWall,
    UnknownRequired,
    MissingDocument,
}

/// One handoff, with the field that caused it when a field did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Handoff {
    pub kind: HandoffKind,
    pub label: Option<String>,
}

/// The filled form handed to the user, and the window it lives in (AC1/AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AtsSessionView {
    pub application_id: String,
    /// The attempt's correlation id, which `confirm` takes back (AC3).
    pub attempt_id: String,
    pub url: String,
    pub provider: String,
    pub filled: Vec<FilledField>,
    /// Document kinds attached from the Package (`cv`, `cover_letter`).
    pub attached: Vec<String>,
    pub unanswered: Vec<UnansweredField>,
    pub handoff: Option<Handoff>,
}

/// The evidence a confirmed submission keeps (AC3, FR-88).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SubmissionEvidence {
    pub confirmation_text: String,
    pub screenshot_blake3: String,
}

/// What a submission turned out to be (AC3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AtsOutcome {
    /// The page confirmed it: evidence stored, the card is Applied.
    Applied { evidence: SubmissionEvidence },
    /// No confirmation was found: the attempt is recorded and the card stays,
    /// marked Needs check from that outcome (FR-88).
    NeedsCheck { screenshot_blake3: String },
}

/// One screening answer as the approve panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AnswerView {
    pub id: String,
    pub question: String,
    pub answer: Option<String>,
    pub class: String,
    pub needs_review: bool,
    /// `false` while the row is still a draft (S7.8: only approval makes it
    /// reusable).
    pub approved: bool,
}

/// An application's answer bank, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct AnswerBankView {
    pub application_id: String,
    pub answers: Vec<AnswerView>,
}

/// The ATS Form assist use cases (AC1–AC3) over the browser port.
pub struct AtsFormService {
    apply: std::sync::Arc<dyn ApplyRepo>,
    documents: std::sync::Arc<dyn DocumentsRepo>,
    profile: std::sync::Arc<dyn ProfileRepo>,
    browser: std::sync::Arc<dyn BrowserPort>,
    system: std::sync::Arc<dyn SystemRepo>,
    clock: std::sync::Arc<dyn ClockPort>,
}

impl AtsFormService {
    pub fn new(
        apply: std::sync::Arc<dyn ApplyRepo>,
        documents: std::sync::Arc<dyn DocumentsRepo>,
        profile: std::sync::Arc<dyn ProfileRepo>,
        browser: std::sync::Arc<dyn BrowserPort>,
        system: std::sync::Arc<dyn SystemRepo>,
        clock: std::sync::Arc<dyn ClockPort>,
    ) -> Self {
        Self {
            apply,
            documents,
            profile,
            browser,
            system,
            clock,
        }
    }

    /// Whether the user accepted the browser warning (AC1).
    pub fn opted_in(&self) -> Result<bool, AtsFormError> {
        Ok(self
            .system
            .setting_get(OPT_IN_KEY)?
            .as_deref()
            .is_some_and(|value| value == OPT_IN_MARKER))
    }

    /// Open the form in the Boba browser and fill it (AC1/AC2).
    pub fn start(
        &self,
        application: &Application,
        offer: &Offer,
    ) -> Result<AtsSessionView, AtsFormError> {
        // AC1: nothing drives the user's browser before they accept the warning.
        if !self.opted_in()? {
            return Err(AtsFormError::OptInRequired);
        }
        let (provider, url) = ats_channel(offer).ok_or(AtsFormError::NotAtsForm)?;

        // AD-14: the same gate the manual kit uses, before the browser opens.
        let view = self.manual().view(application, offer)?;
        if !view.pending.is_empty() {
            return Err(AtsFormError::NotReady(view.pending));
        }
        let mut documents: BTreeMap<String, PdfExport> = BTreeMap::new();
        for export in self.manual().export_pdfs(application)? {
            documents.insert(export.kind.clone(), export);
        }
        let answers = self.reusable_answers(&application.id)?;
        let identity = self.identity(application)?;

        let snapshot = self.browser.open(provider, &url)?;
        let mut filled = Vec::new();
        let mut attached: Vec<String> = Vec::new();
        let mut unanswered = Vec::new();
        let mut handoff = snapshot.challenge.map(|challenge| Handoff {
            kind: match challenge {
                crate::ports::browser::Challenge::Captcha => HandoffKind::Captcha,
                crate::ports::browser::Challenge::LoginWall => HandoffKind::LoginWall,
            },
            label: None,
        });

        if handoff.is_none() {
            for field in &snapshot.fields {
                match plan_field(field, &identity, &answers, &documents) {
                    FieldPlan::Fill(value) => {
                        self.browser.fill_text(&field.handle, value.text())?;
                        filled.push(filled_field(field));
                    }
                    FieldPlan::Attach(kind) => {
                        let Some(export) = documents.get(kind) else {
                            handoff = Some(Handoff {
                                kind: HandoffKind::MissingDocument,
                                label: Some(field.label.clone()),
                            });
                            break;
                        };
                        self.browser.attach(
                            &field.handle,
                            &Upload {
                                file_name: export.file_name.clone(),
                                bytes: export.bytes.clone(),
                            },
                        )?;
                        if !attached.iter().any(|seen| seen == kind) {
                            attached.push(kind.to_string());
                        }
                        filled.push(filled_field(field));
                    }
                    FieldPlan::Leave => unanswered.push(UnansweredField {
                        label: field.label.clone(),
                        kind: field.kind.as_str().to_string(),
                        required: field.required,
                    }),
                    FieldPlan::Handoff { kind, label } => {
                        // AC2: stop here; the user takes over from this point.
                        handoff = Some(Handoff { kind, label });
                        break;
                    }
                }
            }
        }

        // AD-12: the attempt is on record before the user's submit can happen.
        // Page text is not part of it — only ids, the URL and the documents.
        // The id travels back from the UI: keep it to id characters (a
        // timestamp's `:` and `+` are not).
        let stamp = self.clock.now_rfc3339().replace([':', '+', '.'], "-");
        let attempt_id = format!("ats_{}_{stamp}", application.id);
        self.system.audit_append_intent(&NewIntent {
            action: AUDIT_ACTION.to_string(),
            correlation_id: attempt_id.clone(),
            // The user presses submit in their own browser.
            actor: "user".to_string(),
            target_kind: "application".to_string(),
            target_id: Some(application.id.clone()),
            detail_json: serde_json::json!({
                "provider": provider.as_str(),
                "url": url,
                "documents": attached,
            })
            .to_string(),
            payload_blake3: None,
            created_at: self.clock.now_rfc3339(),
        })?;

        Ok(AtsSessionView {
            application_id: application.id.clone(),
            attempt_id,
            url,
            provider: provider.as_str().to_string(),
            filled,
            attached,
            unanswered,
            handoff,
        })
    }

    /// Take the page's answer after the user submitted (AC3).
    pub fn confirm(
        &self,
        attempt_id: &str,
        application: &Application,
    ) -> Result<AtsOutcome, AtsFormError> {
        // Revoking the opt-in stops even a read of the open page (AC1, F3).
        if !self.opted_in()? {
            return Err(AtsFormError::OptInRequired);
        }
        let intent = self
            .system
            .audit_unresolved_intents()?
            .into_iter()
            .find(|row| row.action == AUDIT_ACTION && row.correlation_id == attempt_id)
            .ok_or_else(|| AtsFormError::UnknownAttempt(attempt_id.to_string()))?;
        let snapshot = self.browser.snapshot()?;
        let screenshot = self.browser.screenshot()?;
        // The session is over: leave no browser running (§6.4). A failure to
        // close must not cost the evidence, so it is not fatal.
        let _ = self.browser.close();
        let now = self.clock.now_rfc3339();

        // A confirmation only counts on the form's own host: a redirect to
        // another site's "thank you" page is not evidence of this submission.
        let on_form_host = intent_url(&intent)
            .as_deref()
            .is_none_or(|url| same_host(url, &snapshot.url));
        let confirmation = snapshot.confirmation.filter(|_| on_form_host);

        let Some(confirmation) = confirmation else {
            // FR-88: no evidence, so the card does not move. The failed
            // outcome with `needs_check` is what marks it in the Tracker.
            self.system.audit_append_outcome(
                &intent,
                AuditOutcome::Failed,
                &serde_json::json!({
                    "needs_check": true,
                    "url": snapshot.url,
                    "screenshot_blake3": screenshot.png_blake3,
                })
                .to_string(),
                &now,
            )?;
            return Ok(AtsOutcome::NeedsCheck {
                screenshot_blake3: screenshot.png_blake3,
            });
        };

        // Evidence first: the Application moves to Applied only because it
        // exists (FR-88). The screenshot is kept as its hash (AD-12).
        self.system.audit_append_outcome(
            &intent,
            AuditOutcome::Ok,
            &serde_json::json!({
                "confirmation_text": confirmation.text,
                "url": snapshot.url,
                "screenshot_blake3": screenshot.png_blake3,
            })
            .to_string(),
            &now,
        )?;
        if application.status != AppStatus::Applied {
            TrackerService::new(self.apply.clone(), self.clock.clone()).transition(
                TransitionInput {
                    application_id: application.id.clone(),
                    from_status: application.status.as_str().to_string(),
                    to_status: AppStatus::Applied.as_str().to_string(),
                    source: "apply_outcome".to_string(),
                    reason: Some("ats_form_confirmed".to_string()),
                },
            )?;
        }
        Ok(AtsOutcome::Applied {
            evidence: SubmissionEvidence {
                confirmation_text: confirmation.text,
                screenshot_blake3: screenshot.png_blake3,
            },
        })
    }

    /// Record the user's browser opt-in (AC1), audited as a settings change
    /// that affects automation (AD-12).
    pub fn set_opt_in(&self, enabled: bool) -> Result<bool, AtsFormError> {
        let now = self.clock.now_rfc3339();
        let intent = self.system.audit_append_intent(&NewIntent {
            action: OPT_IN_ACTION.to_string(),
            correlation_id: format!("optin_{}_{}", if enabled { "on" } else { "off" }, now),
            actor: "user".to_string(),
            target_kind: "setting".to_string(),
            target_id: Some(OPT_IN_KEY.to_string()),
            detail_json: serde_json::json!({
                "enabled": enabled,
                "warning_version": OPT_IN_MARKER,
            })
            .to_string(),
            payload_blake3: None,
            created_at: now.clone(),
        })?;
        if enabled {
            self.system.setting_put(OPT_IN_KEY, OPT_IN_MARKER, &now)?;
        } else {
            self.system.setting_delete(OPT_IN_KEY)?;
        }
        self.system
            .audit_append_outcome(&intent, AuditOutcome::Ok, "{}", &now)?;
        Ok(enabled)
    }

    /// The application's answer bank, drafts and approved rows (S7.8 surface).
    pub fn answers(&self, application_id: &str) -> Result<AnswerBankView, AtsFormError> {
        let answers = self
            .apply
            .answer_bank_for_application(application_id)?
            .into_iter()
            .map(|row| AnswerView {
                id: row.id,
                question: row.question,
                answer: row.answer,
                class: row.class.as_str().to_string(),
                needs_review: row.needs_review,
                approved: row.approved_at.is_some(),
            })
            .collect();
        Ok(AnswerBankView {
            application_id: application_id.to_string(),
            answers,
        })
    }

    /// Approve one row, edited or as it stands (S7.8 AC3).
    pub fn approve(
        &self,
        application_id: &str,
        answer_id: &str,
        edited: Option<&str>,
    ) -> Result<AnswerBankView, AtsFormError> {
        answers::approve(
            self.apply.as_ref(),
            answer_id,
            edited,
            &self.clock.now_rfc3339(),
        )?;
        self.answers(application_id)
    }

    /// The approved answers the fill may reuse (S7.8: only what the user
    /// approved, and only once it has text).
    fn reusable_answers(&self, application_id: &str) -> Result<Vec<BankAnswer>, AtsFormError> {
        Ok(self
            .apply
            .answer_bank_for_application(application_id)?
            .into_iter()
            .filter(BankAnswer::is_reusable)
            .collect())
    }

    /// The CV's contact block, when the package holds a verified one.
    fn identity(&self, application: &Application) -> Result<Identity, AtsFormError> {
        let newest = self
            .apply
            .package(&application.id)?
            .into_iter()
            .filter(|item| item.kind == "cv" && item.verified_at.is_some())
            .max_by_key(|item| item.version);
        let Some(item) = newest else {
            return Ok(Identity::default());
        };
        let Some(record) = self.documents.get(&item.document_id)? else {
            return Ok(Identity::default());
        };
        // The stored model is the CV document itself; the kind picks the
        // parser (the same shape the Studio and the manual kit read).
        let Ok(cv) = serde_json::from_str::<boba_domain::documents::CvDocument>(&record.model_json)
        else {
            return Ok(Identity::default());
        };
        Ok(Identity::of(&cv.contact))
    }

    /// The Manual use cases' gate and kit, reused: the attachable check
    /// (AD-14) and the approved answers are the same for every channel.
    fn manual(&self) -> ManualService {
        ManualService::new(
            self.apply.clone(),
            self.documents.clone(),
            self.profile.clone(),
            self.clock.clone(),
        )
    }
}

/// The ATS Form channel an Offer has, if any.
fn ats_channel(offer: &Offer) -> Option<(AtsProvider, String)> {
    channel::detect(offer)
        .into_iter()
        .find_map(|channel| match channel {
            channel::ApplyChannel::AtsForm { provider, url } => Some((provider, url)),
            _ => None,
        })
}

/// The URL an attempt was started on, as the intent recorded it.
fn intent_url(intent: &boba_domain::audit::AuditRow) -> Option<String> {
    let detail: serde_json::Value = serde_json::from_str(&intent.detail_json).ok()?;
    detail.get("url")?.as_str().map(str::to_string)
}

/// Whether two URLs name the same host (port and case ignored, `www.` too).
fn same_host(a: &str, b: &str) -> bool {
    host_of(a).is_some_and(|left| host_of(b).is_some_and(|right| left == right))
}

/// The host of an `http(s)` URL, lower-cased with a leading `www.` removed.
fn host_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    if !matches!(scheme.to_ascii_lowercase().as_str(), "http" | "https") {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.split('@').next_back()?.split(':').next()?;
    let host = host.trim().to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host).to_string();
    (!host.is_empty()).then_some(host)
}

/// The identity fields a form asks for, read from the CV's contact block.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct Identity {
    first: Option<String>,
    last: Option<String>,
    full: Option<String>,
    email: Option<String>,
    phone: Option<String>,
    location: Option<String>,
    links: Vec<(String, String)>,
}

impl Identity {
    fn of(contact: &Contact) -> Self {
        let mut parts = contact.full_name.split_whitespace();
        let first = parts.next().map(str::to_string);
        let last = parts.collect::<Vec<_>>().join(" ");
        Self {
            first,
            last: (!last.is_empty()).then_some(last),
            full: Some(contact.full_name.trim().to_string()),
            email: contact.email.clone(),
            phone: contact.phone.clone(),
            location: contact.location.clone(),
            links: contact
                .links
                .iter()
                .map(|link| (link.label.to_ascii_lowercase(), link.url.clone()))
                .collect(),
        }
    }

    fn link(&self, needle: &str) -> Option<String> {
        self.links
            .iter()
            .find(|(label, _)| label.contains(needle))
            .map(|(_, url)| url.clone())
    }
}

/// One value to type into the page. A sensitive value (a pay or eligibility
/// answer, S7.8) prints as redacted, so no log can carry it (AC1).
#[derive(Clone, PartialEq, Eq)]
struct FillValue {
    text: String,
    sensitive: bool,
}

impl FillValue {
    fn plain(text: String) -> Self {
        Self {
            text,
            sensitive: false,
        }
    }

    fn sensitive(text: String) -> Self {
        Self {
            text,
            sensitive: true,
        }
    }

    fn text(&self) -> &str {
        &self.text
    }
}

impl std::fmt::Debug for FillValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.sensitive {
            f.write_str("FillValue(<redacted>)")
        } else {
            f.write_str("FillValue(…)")
        }
    }
}

/// What to do with one field on the page.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FieldPlan<'a> {
    Fill(FillValue),
    Attach(&'a str),
    Leave,
    Handoff {
        kind: HandoffKind,
        label: Option<String>,
    },
}

/// The plan for one field (AC1/AC2).
fn plan_field<'a>(
    field: &PageField,
    identity: &Identity,
    answers: &'a [BankAnswer],
    documents: &BTreeMap<String, PdfExport>,
) -> FieldPlan<'a> {
    if field.kind == PageFieldKind::File {
        return match document_kind_for(&field.label) {
            Some(kind) if documents.contains_key(kind) => FieldPlan::Attach(kind),
            Some(_) => FieldPlan::Handoff {
                kind: HandoffKind::MissingDocument,
                label: Some(field.label.clone()),
            },
            None if field.required => unknown_required(field),
            None => FieldPlan::Leave,
        };
    }

    if field.kind.is_click_target() {
        // A checkbox or radio is a choice among the page's own options; the
        // answer bank holds prose, not that choice, so guessing is not honest.
        return if field.required {
            unknown_required(field)
        } else {
            FieldPlan::Leave
        };
    }

    if !field.kind.takes_text() {
        return if field.required {
            unknown_required(field)
        } else {
            FieldPlan::Leave
        };
    }

    if let Some(text) = identity_value(&field.label, identity) {
        return FieldPlan::Fill(FillValue::plain(text));
    }
    let Some(answer) = answer_for(&field.label, answers) else {
        return if field.required {
            unknown_required(field)
        } else {
            FieldPlan::Leave
        };
    };
    let Some(text) = answer.answer.clone().filter(|text| !text.trim().is_empty()) else {
        return if field.required {
            unknown_required(field)
        } else {
            FieldPlan::Leave
        };
    };
    if field.kind == PageFieldKind::Select && !option_matches(&field.options, &text) {
        // The page offers a fixed list and none of it is the answer: not a
        // confident answer, so a required select hands control back.
        return if field.required {
            unknown_required(field)
        } else {
            FieldPlan::Leave
        };
    }
    if answer.class.comes_from_preferences() {
        FieldPlan::Fill(FillValue::sensitive(text))
    } else {
        FieldPlan::Fill(FillValue::plain(text))
    }
}

fn unknown_required(field: &PageField) -> FieldPlan<'static> {
    FieldPlan::Handoff {
        kind: HandoffKind::UnknownRequired,
        label: Some(field.label.clone()),
    }
}

fn filled_field(field: &PageField) -> FilledField {
    FilledField {
        handle: field.handle.clone(),
        label: field.label.clone(),
        kind: field.kind.as_str().to_string(),
    }
}

/// A label with its case, runs of whitespace, and the punctuation pages put
/// around a caption (`*`, `:`, a trailing `?`) taken off: the same question
/// from two forms then reads the same.
fn clean_label(label: &str) -> String {
    label
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
        .trim_matches(|ch: char| {
            ch.is_whitespace() || matches!(ch, '*' | ':' | '?' | '!' | '.' | ',' | '-')
        })
        .to_string()
}

/// The identity value for a form's caption, if the CV carries one.
fn identity_value(label: &str, identity: &Identity) -> Option<String> {
    let label = clean_label(label);
    if label.is_empty() {
        return None;
    }
    let has = |words: &[&str]| words.iter().any(|word| label.contains(word));
    if has(&["last name", "surname", "family name", "apellido"]) {
        return identity.last.clone();
    }
    if has(&["first name", "given name", "primer nombre"]) {
        return identity.first.clone();
    }
    if has(&["email", "e-mail", "correo"]) {
        return identity.email.clone();
    }
    if has(&[
        "phone",
        "mobile",
        "telephone",
        "teléfono",
        "telefono",
        "celular",
    ]) {
        return identity.phone.clone();
    }
    if has(&["linkedin"]) {
        return identity.link("linkedin");
    }
    if has(&["github"]) {
        return identity.link("github");
    }
    if has(&["portfolio", "website", "personal site", "portafolio"]) {
        return identity
            .link("portfolio")
            .or_else(|| identity.link("website"))
            .or_else(|| identity.link("web"));
    }
    if has(&[
        "location",
        "city",
        "where are you based",
        "ubicación",
        "ubicacion",
        "ciudad",
    ]) {
        return identity.location.clone();
    }
    if label == "name" || has(&["full name", "your name", "nombre completo"]) {
        return identity.full.clone();
    }
    None
}

/// The approved answer whose question matches this caption (S7.8).
fn answer_for<'a>(label: &str, answers: &'a [BankAnswer]) -> Option<&'a BankAnswer> {
    let key = boba_domain::application::question_key(&clean_label(label));
    answers
        .iter()
        .find(|answer| !answer.question_key.is_empty() && answer.question_key == key)
}

/// Whether a select offers the answer's text as one of its choices.
fn option_matches(options: &[String], value: &str) -> bool {
    let value = value.trim();
    options
        .iter()
        .any(|option| option.trim().eq_ignore_ascii_case(value))
}

/// Which Package document a file input asks for, if the caption names one.
fn document_kind_for(label: &str) -> Option<&'static str> {
    let label = clean_label(label);
    if label.contains("cover") || label.contains("carta") {
        return Some("cover_letter");
    }
    let names_cv = label.contains("resume")
        || label.contains("curriculum")
        || label.split_whitespace().any(|word| word == "cv");
    names_cv.then_some("cv")
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::{Arc, Mutex};

    use boba_domain::application::{PackageItem, new_application_id};
    use boba_domain::documents::{DocumentKind, Lang, NewDocument, TemplateId};
    use boba_domain::posting::{ApplyHints, WorkMode};

    use crate::applications::testing::FakeApplyRepo;
    use crate::ports::browser::{FormSnapshot, Screenshot, Upload};
    use crate::ports::repo::documents::fake::FakeDocumentsRepo;
    use crate::profile::testing::FakeProfileRepo;
    use crate::system::{FixedClock, testing::FakeSystemRepo};

    const NOW: &str = "2026-10-17T00:00:00Z";
    const CV_PDF: &[u8] = b"%PDF-1.7 cv";

    /// A browser the test scripts: what the page says, and what it was asked.
    struct FakeBrowser {
        snapshot: Mutex<FormSnapshot>,
        fills: Mutex<Vec<(String, String)>>,
        attached: Mutex<Vec<(String, Upload)>>,
        screenshots: Mutex<u32>,
        closes: Mutex<u32>,
        png_blake3: Mutex<String>,
        refused: Mutex<bool>,
        /// Whether a window is open: `snapshot`/`screenshot` never launch one.
        live: Mutex<bool>,
    }

    impl Default for FakeBrowser {
        fn default() -> Self {
            Self {
                snapshot: Mutex::new(FormSnapshot {
                    url: String::new(),
                    title: String::new(),
                    text: String::new(),
                    fields: vec![],
                    challenge: None,
                    confirmation: None,
                }),
                fills: Mutex::new(vec![]),
                attached: Mutex::new(vec![]),
                screenshots: Mutex::new(0),
                closes: Mutex::new(0),
                png_blake3: Mutex::new(String::new()),
                refused: Mutex::new(false),
                live: Mutex::new(false),
            }
        }
    }

    impl FakeBrowser {
        fn with_page(fields: Vec<PageField>) -> Self {
            let browser = Self::default();
            *browser.snapshot.lock().unwrap() = FormSnapshot {
                url: "https://jobs.lever.co/nimbus/abc".into(),
                title: "Apply".into(),
                text: "Apply to Nimbus".into(),
                fields,
                challenge: None,
                confirmation: None,
            };
            *browser.png_blake3.lock().unwrap() = "hash-of-png".into();
            browser
        }

        fn set(&self, snapshot: FormSnapshot) {
            *self.snapshot.lock().unwrap() = snapshot;
        }
    }

    impl BrowserPort for FakeBrowser {
        fn open(&self, _provider: AtsProvider, _url: &str) -> Result<FormSnapshot, BrowserError> {
            if *self.refused.lock().unwrap() {
                return Err(BrowserError::NotRunning);
            }
            *self.live.lock().unwrap() = true;
            Ok(self.snapshot.lock().unwrap().clone())
        }

        fn snapshot(&self) -> Result<FormSnapshot, BrowserError> {
            if !*self.live.lock().unwrap() {
                return Err(BrowserError::NotRunning);
            }
            Ok(self.snapshot.lock().unwrap().clone())
        }

        fn fill_text(&self, handle: &str, value: &str) -> Result<(), BrowserError> {
            self.fills
                .lock()
                .unwrap()
                .push((handle.to_string(), value.to_string()));
            Ok(())
        }

        fn choose(&self, handle: &str) -> Result<(), BrowserError> {
            self.fills
                .lock()
                .unwrap()
                .push((handle.to_string(), "chosen".to_string()));
            Ok(())
        }

        fn attach(&self, handle: &str, file: &Upload) -> Result<(), BrowserError> {
            self.attached
                .lock()
                .unwrap()
                .push((handle.to_string(), file.clone()));
            *self.png_blake3.lock().unwrap() = "hash-of-png".into();
            Ok(())
        }

        fn screenshot(&self) -> Result<Screenshot, BrowserError> {
            if !*self.live.lock().unwrap() {
                return Err(BrowserError::NotRunning);
            }
            *self.screenshots.lock().unwrap() += 1;
            Ok(Screenshot {
                png_blake3: self.png_blake3.lock().unwrap().clone(),
            })
        }

        fn close(&self) -> Result<(), BrowserError> {
            *self.closes.lock().unwrap() += 1;
            *self.live.lock().unwrap() = false;
            Ok(())
        }
    }

    fn field(handle: &str, label: &str, kind: PageFieldKind, required: bool) -> PageField {
        PageField {
            handle: handle.into(),
            label: label.into(),
            kind,
            required,
            options: vec![],
        }
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
                email: None,
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

    fn ready_model() -> String {
        serde_json::json!({
            "schema_version": 1,
            "lang": "en",
            "contact": {
                "full_name": "Ada Lovelace",
                "email": "ada@example.com",
                "phone": "+34 600 000 000",
                "location": "Madrid",
            },
            "sections": [],
        })
        .to_string()
    }

    fn seed_cv(apply: &FakeApplyRepo, documents: &FakeDocumentsRepo, application_id: &str) -> i64 {
        let version = documents
            .insert(&NewDocument {
                id: "doc_cv".into(),
                application_id: application_id.into(),
                kind: DocumentKind::Cv,
                lang: Lang::En,
                template: TemplateId::new("cv-classic"),
                model_json: ready_model(),
                language_fallback_from: None,
                untrusted_origin: true,
                created_at: NOW.into(),
            })
            .unwrap();
        documents.set_pdf("doc_cv", CV_PDF, NOW).unwrap();
        documents.mark_verified("doc_cv", "{}", NOW).unwrap();
        apply.seed_package(
            application_id,
            PackageItem {
                document_id: "doc_cv".into(),
                kind: "cv".into(),
                lang: "en".into(),
                version,
                verified_at: Some(NOW.into()),
            },
        );
        version
    }

    fn seed_answer(
        apply: &FakeApplyRepo,
        application_id: &str,
        id: &str,
        question: &str,
        answer: &str,
        class: boba_domain::application::AnswerClass,
        approved: bool,
    ) {
        apply
            .answer_bank_insert(&boba_domain::application::NewBankAnswer {
                id: id.into(),
                application_id: application_id.into(),
                question: question.into(),
                question_key: boba_domain::application::question_key(question),
                answer: Some(answer.into()),
                class,
                fact_ids: vec![],
                needs_review: false,
                untrusted_origin: true,
                created_at: NOW.into(),
            })
            .unwrap();
        if approved {
            apply.answer_bank_approve(id, None, NOW).unwrap();
        }
    }

    struct Harness {
        apply: Arc<FakeApplyRepo>,
        documents: Arc<FakeDocumentsRepo>,
        system: Arc<FakeSystemRepo>,
        browser: Arc<FakeBrowser>,
        service: AtsFormService,
    }

    fn harness(fields: Vec<PageField>, opted_in: bool) -> Harness {
        let apply = Arc::new(FakeApplyRepo::default());
        let documents = Arc::new(FakeDocumentsRepo::default());
        let system = Arc::new(FakeSystemRepo::default());
        system
            .setting_put(
                OPT_IN_KEY,
                if opted_in { OPT_IN_MARKER } else { "off" },
                NOW,
            )
            .unwrap();
        let browser = Arc::new(FakeBrowser::with_page(fields));
        let service = AtsFormService::new(
            apply.clone(),
            documents.clone(),
            Arc::new(FakeProfileRepo::default()),
            browser.clone(),
            system.clone(),
            Arc::new(FixedClock::new(NOW)),
        );
        Harness {
            apply,
            documents,
            system,
            browser,
            service,
        }
    }

    // AC1: the opt-in is what turns automation on.
    #[test]
    fn starting_without_the_opt_in_is_refused() {
        let h = harness(vec![], false);
        let application = application();
        let error = h.service.start(&application, &offer()).unwrap_err();
        assert_eq!(error.message_key(), "errors.apply.ats_opt_in");
        assert!(
            h.apply.events().is_empty(),
            "nothing is written without the opt-in"
        );
    }

    // AD-12: turning browser automation on or off is a settings change that
    // affects automation, so it leaves a chained intent/outcome pair.
    #[test]
    fn the_opt_in_is_stored_and_audited() {
        let h = harness(vec![], false);
        assert!(!h.service.opted_in().unwrap());

        assert!(h.service.set_opt_in(true).unwrap());

        assert!(h.service.opted_in().unwrap());
        assert_eq!(
            h.system.audit_verify_chain().unwrap().rows,
            2,
            "an intent and its outcome"
        );
        assert!(h.system.audit_unresolved_intents().unwrap().is_empty());

        assert!(!h.service.set_opt_in(false).unwrap());
        assert!(!h.service.opted_in().unwrap());
        assert_eq!(h.system.audit_verify_chain().unwrap().rows, 4);
    }

    #[test]
    fn an_offer_without_an_ats_form_is_refused() {
        let h = harness(vec![], true);
        let mut offer = offer();
        offer.apply.url = "https://nimbus.example/jobs/1".into();
        let error = h.service.start(&application(), &offer).unwrap_err();
        assert_eq!(error.message_key(), "errors.apply.ats_not_a_form");
    }

    // AC1: identity from the CV's contact block, answers from the bank, the
    // verified PDF attached, and an intent on record before the user submits.
    #[test]
    fn starting_fills_identity_and_approved_answers_and_attaches_the_cv() {
        let h = harness(
            vec![
                field("boba-1", "First name", PageFieldKind::Text, true),
                field("boba-2", "Last name", PageFieldKind::Text, true),
                field("boba-3", "Email", PageFieldKind::Email, true),
                field("boba-4", "Phone", PageFieldKind::Tel, false),
                field("boba-5", "Resume/CV", PageFieldKind::File, true),
                field(
                    "boba-6",
                    "When can you start?",
                    PageFieldKind::TextArea,
                    true,
                ),
            ],
            true,
        );
        let application = application();
        seed_cv(&h.apply, &h.documents, &application.id);
        seed_answer(
            &h.apply,
            &application.id,
            "ans_1",
            "When can you start?",
            "Two weeks",
            boba_domain::application::AnswerClass::Other,
            true,
        );

        let session = h.service.start(&application, &offer()).unwrap();

        let fills = h.browser.fills.lock().unwrap().clone();
        assert_eq!(
            fills,
            vec![
                ("boba-1".to_string(), "Ada".to_string()),
                ("boba-2".to_string(), "Lovelace".to_string()),
                ("boba-3".to_string(), "ada@example.com".to_string()),
                ("boba-4".to_string(), "+34 600 000 000".to_string()),
                ("boba-6".to_string(), "Two weeks".to_string()),
            ]
        );
        let attached = h.browser.attached.lock().unwrap().clone();
        assert_eq!(attached.len(), 1);
        assert_eq!(attached[0].0, "boba-5");
        assert_eq!(attached[0].1.bytes, CV_PDF);
        assert_eq!(session.attached, vec!["cv"]);
        assert!(session.handoff.is_none());
        assert_eq!(
            session.filled.len(),
            6,
            "four typed values, the CV and the answer"
        );
        assert_eq!(session.filled[0].handle, "boba-1");
        assert!(session.filled.iter().any(|field| field.handle == "boba-5"));
        assert!(!session.attempt_id.is_empty());

        let unresolved = h.system.audit_unresolved_intents().unwrap();
        assert_eq!(unresolved.len(), 1, "the intent is written before submit");
        assert_eq!(unresolved[0].action, AUDIT_ACTION);
        assert_eq!(unresolved[0].correlation_id, session.attempt_id);
    }

    // S7.8: only what the user approved is reused.
    #[test]
    fn a_draft_answer_is_not_filled() {
        let h = harness(
            vec![field(
                "boba-1",
                "When can you start?",
                PageFieldKind::Text,
                false,
            )],
            true,
        );
        let application = application();
        seed_cv(&h.apply, &h.documents, &application.id);
        seed_answer(
            &h.apply,
            &application.id,
            "ans_1",
            "When can you start?",
            "Two weeks",
            boba_domain::application::AnswerClass::Other,
            false,
        );

        let session = h.service.start(&application, &offer()).unwrap();

        assert!(h.browser.fills.lock().unwrap().is_empty());
        assert_eq!(session.unanswered.len(), 1);
    }

    // AC2: a CAPTCHA stops the fill and hands the window to the user.
    #[test]
    fn a_captcha_hands_control_back_without_filling() {
        let h = harness(
            vec![field("boba-1", "Email", PageFieldKind::Email, true)],
            true,
        );
        let application = application();
        seed_cv(&h.apply, &h.documents, &application.id);
        let mut page = h.browser.snapshot.lock().unwrap().clone();
        page.challenge = Some(crate::ports::browser::Challenge::Captcha);
        h.browser.set(page);

        let session = h.service.start(&application, &offer()).unwrap();

        assert!(h.browser.fills.lock().unwrap().is_empty());
        assert_eq!(
            session.handoff,
            Some(Handoff {
                kind: HandoffKind::Captcha,
                label: None
            })
        );
        assert_eq!(
            h.system.audit_unresolved_intents().unwrap().len(),
            1,
            "the attempt stays on record: the user may still submit"
        );
    }

    #[test]
    fn a_login_wall_hands_control_back() {
        let h = harness(vec![], true);
        let application = application();
        seed_cv(&h.apply, &h.documents, &application.id);
        let mut page = h.browser.snapshot.lock().unwrap().clone();
        page.challenge = Some(crate::ports::browser::Challenge::LoginWall);
        h.browser.set(page);

        let session = h.service.start(&application, &offer()).unwrap();

        assert_eq!(
            session.handoff,
            Some(Handoff {
                kind: HandoffKind::LoginWall,
                label: None
            })
        );
    }

    // AC2: an unknown required question stops the fill where it is.
    #[test]
    fn an_unknown_required_question_hands_control_back_and_stops() {
        let h = harness(
            vec![
                field("boba-1", "Email", PageFieldKind::Email, true),
                field(
                    "boba-2",
                    "Describe your favourite database migration",
                    PageFieldKind::TextArea,
                    true,
                ),
                field("boba-3", "Phone", PageFieldKind::Tel, false),
            ],
            true,
        );
        let application = application();
        seed_cv(&h.apply, &h.documents, &application.id);

        let session = h.service.start(&application, &offer()).unwrap();

        let fills = h.browser.fills.lock().unwrap().clone();
        assert_eq!(
            fills,
            vec![("boba-1".to_string(), "ada@example.com".to_string())]
        );
        assert_eq!(
            session.handoff,
            Some(Handoff {
                kind: HandoffKind::UnknownRequired,
                label: Some("Describe your favourite database migration".into())
            })
        );
    }

    // AD-14: an unverified package never reaches the browser.
    #[test]
    fn a_package_that_is_not_ready_is_refused_before_the_browser_opens() {
        let h = harness(vec![], true);
        let application = application();
        // A document with a PDF but no verification.
        h.documents
            .insert(&NewDocument {
                id: "doc_cv".into(),
                application_id: application.id.clone(),
                kind: DocumentKind::Cv,
                lang: Lang::En,
                template: TemplateId::new("cv-classic"),
                model_json: ready_model(),
                language_fallback_from: None,
                untrusted_origin: true,
                created_at: NOW.into(),
            })
            .unwrap();
        h.documents.set_pdf("doc_cv", CV_PDF, NOW).unwrap();
        h.apply.seed_package(
            &application.id,
            PackageItem {
                document_id: "doc_cv".into(),
                kind: "cv".into(),
                lang: "en".into(),
                version: 1,
                verified_at: None,
            },
        );
        *h.browser.refused.lock().unwrap() = true;

        let error = h.service.start(&application, &offer()).unwrap_err();

        assert_eq!(error.message_key(), "errors.apply.not_ready");
        assert!(h.system.audit_unresolved_intents().unwrap().is_empty());
    }

    // AC1: a sensitive answer is typed but never shown in a view or a log.
    #[test]
    fn a_salary_answer_is_filled_but_never_in_the_view() {
        let h = harness(
            vec![field(
                "boba-1",
                "What are your salary expectations?",
                PageFieldKind::Text,
                true,
            )],
            true,
        );
        let application = application();
        seed_cv(&h.apply, &h.documents, &application.id);
        seed_answer(
            &h.apply,
            &application.id,
            "ans_1",
            "What are your salary expectations?",
            "EUR 65,000 per year",
            boba_domain::application::AnswerClass::Salary,
            true,
        );

        let session = h.service.start(&application, &offer()).unwrap();

        assert_eq!(h.browser.fills.lock().unwrap().len(), 1);
        let debug = format!("{session:?}");
        assert!(
            !debug.contains("65,000"),
            "a sensitive value is never part of a view: {debug}"
        );
        let json = serde_json::to_string(&session).unwrap();
        assert!(!json.contains("65,000"), "{json}");
    }

    // AC3: a confirmation page moves the card, with the evidence as its record.
    #[test]
    fn a_confirmation_moves_the_card_and_records_the_evidence() {
        let h = harness(vec![], true);
        let application = application();
        seed_cv(&h.apply, &h.documents, &application.id);
        h.apply.seed_application(application.clone());
        let session = h.service.start(&application, &offer()).unwrap();
        let mut page = h.browser.snapshot.lock().unwrap().clone();
        page.confirmation = Some(crate::ports::browser::Confirmation {
            text: "Thank you for applying to Nimbus.".into(),
        });
        h.browser.set(page);

        let outcome = h
            .service
            .confirm(&session.attempt_id, &application)
            .unwrap();

        assert_eq!(
            outcome,
            AtsOutcome::Applied {
                evidence: SubmissionEvidence {
                    confirmation_text: "Thank you for applying to Nimbus.".into(),
                    screenshot_blake3: "hash-of-png".into(),
                }
            }
        );
        let events = h.apply.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].to_status, AppStatus::Applied);
        assert_eq!(events[0].source.as_str(), "apply_outcome");
        assert!(h.system.audit_unresolved_intents().unwrap().is_empty());
        assert_eq!(h.system.audit_verify_chain().unwrap().rows, 2);
        assert_eq!(
            *h.browser.closes.lock().unwrap(),
            1,
            "the session's browser is closed when the attempt ends"
        );
    }

    // A confirmation only counts on the form's own host: another site's
    // "thank you" page is not evidence of this submission (AC3, FR-88).
    #[test]
    fn a_confirmation_on_another_host_is_not_evidence() {
        let h = harness(vec![], true);
        let application = application();
        seed_cv(&h.apply, &h.documents, &application.id);
        h.apply.seed_application(application.clone());
        let session = h.service.start(&application, &offer()).unwrap();
        let mut page = h.browser.snapshot.lock().unwrap().clone();
        page.url = "https://evil.example/thanks".into();
        page.confirmation = Some(crate::ports::browser::Confirmation {
            text: "Thank you for applying to Nimbus.".into(),
        });
        h.browser.set(page);

        let outcome = h
            .service
            .confirm(&session.attempt_id, &application)
            .unwrap();

        assert!(matches!(outcome, AtsOutcome::NeedsCheck { .. }));
        assert!(h.apply.events().is_empty(), "the card must not move");
    }

    // AC3: no evidence means Needs check, and the card does not move.
    #[test]
    fn without_a_confirmation_the_attempt_is_needs_check_and_the_card_stays() {
        let h = harness(vec![], true);
        let application = application();
        seed_cv(&h.apply, &h.documents, &application.id);
        let session = h.service.start(&application, &offer()).unwrap();

        let outcome = h
            .service
            .confirm(&session.attempt_id, &application)
            .unwrap();

        assert_eq!(
            outcome,
            AtsOutcome::NeedsCheck {
                screenshot_blake3: "hash-of-png".into()
            }
        );
        assert!(
            h.apply.events().is_empty(),
            "a submission without evidence never moves the card"
        );
        assert!(h.system.audit_unresolved_intents().unwrap().is_empty());
        assert_eq!(h.system.audit_verify_chain().unwrap().rows, 2);
        assert_eq!(*h.browser.screenshots.lock().unwrap(), 1);
    }

    // F3: after a restart (or a closed window) confirm must not open a fresh
    // browser; the attempt stays unresolved so it can be checked later.
    #[test]
    fn confirming_with_no_live_window_keeps_the_attempt_unresolved() {
        let h = harness(vec![], true);
        let application = application();
        seed_cv(&h.apply, &h.documents, &application.id);
        let session = h.service.start(&application, &offer()).unwrap();
        // The user closed the Chrome window.
        *h.browser.live.lock().unwrap() = false;

        let error = h
            .service
            .confirm(&session.attempt_id, &application)
            .unwrap_err();

        assert!(matches!(
            error,
            AtsFormError::Browser(BrowserError::NotRunning)
        ));
        assert_eq!(
            h.system.audit_unresolved_intents().unwrap().len(),
            1,
            "the attempt stays open for a later check"
        );
        assert_eq!(
            h.system.audit_verify_chain().unwrap().rows,
            1,
            "no outcome is recorded for a page that was never read"
        );
        assert!(h.apply.events().is_empty());
    }

    // F3: revoking the opt-in stops a read too.
    #[test]
    fn confirming_without_the_opt_in_is_refused() {
        let h = harness(vec![], true);
        let application = application();
        seed_cv(&h.apply, &h.documents, &application.id);
        let session = h.service.start(&application, &offer()).unwrap();
        h.system.setting_delete(OPT_IN_KEY).unwrap();

        let error = h
            .service
            .confirm(&session.attempt_id, &application)
            .unwrap_err();

        assert_eq!(error.message_key(), "errors.apply.ats_opt_in");
        assert_eq!(h.system.audit_unresolved_intents().unwrap().len(), 1);
    }

    #[test]
    fn confirming_an_unknown_attempt_is_refused() {
        let h = harness(vec![], true);
        let error = h.service.confirm("ats_nope", &application()).unwrap_err();
        assert_eq!(error.message_key(), "errors.apply.ats_unknown_attempt");
    }

    // S7.8: the approve panel lists drafts and approved rows, and approving an
    // edited answer makes it reusable.
    #[test]
    fn the_answer_bank_lists_drafts_and_approving_makes_them_reusable() {
        let h = harness(vec![], true);
        let application = application();
        h.apply.seed_application(application.clone());
        seed_answer(
            &h.apply,
            &application.id,
            "ans_1",
            "When can you start?",
            "Two weeks",
            boba_domain::application::AnswerClass::Other,
            false,
        );

        let view = h.service.answers(&application.id).unwrap();
        assert_eq!(view.answers.len(), 1);
        assert_eq!(view.answers[0].answer.as_deref(), Some("Two weeks"));
        assert!(!view.answers[0].approved);

        let view = h
            .service
            .approve(&application.id, "ans_1", Some("Next month"))
            .unwrap();
        assert!(view.answers[0].approved);
        assert_eq!(view.answers[0].answer.as_deref(), Some("Next month"));
        assert_eq!(
            h.apply
                .answer_bank_reusable(&["when can you start".to_string()])
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn approving_an_unknown_answer_is_refused() {
        let h = harness(vec![], true);
        let error = h.service.approve("app_1", "ans_nope", None).unwrap_err();
        assert_eq!(error.message_key(), "errors.apply.ats_unknown_answer");
    }
}
