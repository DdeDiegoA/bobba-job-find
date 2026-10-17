//! Apply commands (S9.2, FR-81/FR-82): the Studio's apply bar.
//!
//! Thin wrappers over [`ManualService`]: resolve the ports, call the use case,
//! map the error (AD-3). The store arrives through [`wiring::port`] (ADR-012):
//! a locked or unmounted store is `errors.store.locked`, never a panic.
//!
//! The PDF kit is written to `<app data>/exports/apply/<application>/` and its
//! paths returned: the webview picks nothing, so no page can ask the core to
//! write wherever it likes. Only documents that are Ready and verified reach
//! that folder (AD-14) — the S7.7 UI block is not the gate.

use std::path::PathBuf;
use std::sync::Arc;

use boba_app::applications::TrackerError;
use boba_app::apply::ats_form::{
    AnswerBankView, AtsFormError, AtsFormService, AtsOutcome, AtsSessionView,
};
use boba_app::apply::engine::{ApplyEngine, EmailApplyView, EngineError, SendReceipt, SendRequest};
use boba_app::apply::manual::{ManualError, ManualService, ManualView};
use boba_app::apply::outbox::{
    DryRunView, Finish, OutboxError, OutboxItemView, OutboxService, OutboxSettings, OutboxView,
    Release, Submit,
};
use boba_app::apply::rules::{
    self, AutoRulesService, DecisionView, RuleInput, RuleView, RulesError, RulesSnapshot,
};
use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::browser::BrowserPort;
use boba_app::ports::clock::ClockPort;
use boba_app::ports::kill_switch::KillSwitchPort;
use boba_app::ports::mail::{MailError, MailPort};
use boba_app::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use boba_app::ports::repo::auto_rules::AutoRulesRepo;
use boba_app::ports::repo::documents::DocumentsRepo;
use boba_app::ports::repo::email::EmailRepo;
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::outbox::OutboxRepo;
use boba_app::ports::repo::outbox_hold::OutboxHoldRepo;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::scoring::ScoringRepo;
use boba_app::ports::repo::sources::SourcesRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::system::SystemClock;
use boba_domain::application::Application;
use boba_domain::apply::outbox::OutboxState;
use boba_domain::apply::policy::{self, PolicyCandidate, RuleChannel};
use boba_domain::posting::Offer;
use boba_domain::scoring::gates::{Confidence, Verdict as GateVerdict};
use boba_domain::scoring::rank::Verdict;
use tauri::Manager;

use crate::commands::studio::ExportedFile;
use crate::wiring;

/// The command-level failures this slice raises (CH7: `errors.apply.*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandError {
    Store,
    UnknownApplication,
    UnknownOffer,
    ExportPath,
    ExportWrite,
}

impl CommandError {
    const fn message_key(self) -> &'static str {
        match self {
            CommandError::Store => "errors.apply.store",
            CommandError::UnknownApplication => "errors.apply.unknown_application",
            CommandError::UnknownOffer => "errors.apply.unknown_offer",
            CommandError::ExportPath => "errors.apply.export_path",
            CommandError::ExportWrite => "errors.apply.export_write",
        }
    }

    fn app_error(self) -> AppError {
        let code = match self {
            CommandError::UnknownApplication | CommandError::UnknownOffer => ErrorCode::NotFound,
            CommandError::Store => ErrorCode::Internal,
            CommandError::ExportPath | CommandError::ExportWrite => ErrorCode::Internal,
        };
        AppError::new(code, self.message_key())
    }
}

fn service(app: &tauri::AppHandle) -> AppResult<ManualService> {
    Ok(ManualService::new(
        wiring::port::<dyn ApplyRepo>(app)?,
        wiring::port::<dyn DocumentsRepo>(app)?,
        wiring::port::<dyn ProfileRepo>(app)?,
        std::sync::Arc::new(SystemClock),
    ))
}

/// The application and the Offer its channels are detected from.
fn context<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    application_id: &str,
) -> AppResult<(Application, Offer)> {
    let application = wiring::port::<dyn ApplyRepo>(app)?
        .application_get(application_id)
        .map_err(|_| CommandError::Store.app_error())?
        .ok_or_else(|| CommandError::UnknownApplication.app_error())?;
    let offer = wiring::port::<dyn IngestionRepo>(app)?
        .get_offer(&application.posting_id)
        .map_err(|_| CommandError::Store.app_error())?
        .ok_or_else(|| CommandError::UnknownOffer.app_error())?;
    Ok((application, offer))
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: ManualError) -> AppError {
    let code = match &error {
        ManualError::Repo(ApplyRepoError::UnknownApplication(_)) => ErrorCode::NotFound,
        ManualError::Repo(ApplyRepoError::Conflict { .. }) => ErrorCode::Conflict,
        ManualError::Tracker(TrackerError::Repo(ApplyRepoError::Conflict { .. })) => {
            ErrorCode::Conflict
        }
        ManualError::Tracker(TrackerError::Rule(_)) | ManualError::NotReady(_) => {
            ErrorCode::InvalidInput
        }
        _ => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

/// An id this slice will write to disk under, so it is also a safe folder name:
/// `app_<ULID>` and nothing with a separator or a dot could ever name a path
/// outside the exports root.
fn required_id(id: String) -> AppResult<String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_input("errors.apply.missing_id"));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::invalid_input("errors.apply.invalid_id"));
    }
    Ok(trimmed.to_string())
}

/// The channels and the copy-ready kit for one application (AC1/AC2).
#[tauri::command]
#[specta::specta]
pub fn apply_view(app: tauri::AppHandle, application_id: String) -> AppResult<ManualView> {
    let application_id = required_id(application_id)?;
    let (application, offer) = context(&app, &application_id)?;
    service(&app)?.view(&application, &offer).map_err(to_app)
}

/// Mark the application Applied after the user applied by hand (AC2). The core
/// re-checks Ready + verified first, so a manual apply never attaches a draft
/// the checks did not pass (AD-14).
#[tauri::command]
#[specta::specta]
pub fn apply_mark_applied(app: tauri::AppHandle, application_id: String) -> AppResult<ManualView> {
    let application_id = required_id(application_id)?;
    let (application, offer) = context(&app, &application_id)?;
    service(&app)?
        .mark_applied(&application, &offer)
        .map_err(to_app)
}

/// Write the kit's PDFs for the user to attach by hand and return where they
/// are (AD-14: only Ready and verified documents reach the folder).
#[tauri::command]
#[specta::specta]
pub fn apply_export_pdfs(
    app: tauri::AppHandle,
    application_id: String,
) -> AppResult<Vec<ExportedFile>> {
    let application_id = required_id(application_id)?;
    let (application, _offer) = context(&app, &application_id)?;
    let exports = service(&app)?.export_pdfs(&application).map_err(to_app)?;
    let dir = kit_dir(&app, &application_id)?;
    exports
        .into_iter()
        .map(|export| {
            let path = dir.join(&export.file_name);
            std::fs::write(&path, &export.bytes)
                .map_err(|_| CommandError::ExportWrite.app_error())?;
            Ok(ExportedFile {
                path: path.to_string_lossy().to_string(),
                file_name: export.file_name,
                version: export.version,
            })
        })
        .collect()
}

/// The Email Apply engine (S9.3), built from the mounted ports.
fn engine<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> AppResult<ApplyEngine> {
    Ok(ApplyEngine::new(
        wiring::port::<dyn ApplyRepo>(app)?,
        wiring::port::<dyn DocumentsRepo>(app)?,
        wiring::port::<dyn ProfileRepo>(app)?,
        wiring::port::<dyn OutboxRepo>(app)?,
        wiring::port::<dyn SystemRepo>(app)?,
        wiring::port::<dyn MailPort>(app)?,
        std::sync::Arc::new(SystemClock),
    ))
}

/// The ATS Form assist service (S9.4): the browser port writes nothing to the
/// store itself, and the audit owner records every attempt (AD-12).
fn ats_service(app: &tauri::AppHandle) -> AppResult<AtsFormService> {
    Ok(AtsFormService::new(
        wiring::port::<dyn ApplyRepo>(app)?,
        wiring::port::<dyn DocumentsRepo>(app)?,
        wiring::port::<dyn ProfileRepo>(app)?,
        wiring::port::<dyn BrowserPort>(app)?,
        wiring::port::<dyn SystemRepo>(app)?,
        std::sync::Arc::new(SystemClock),
    ))
}

/// Map an Email Apply failure onto the typed IPC error (AD-3).
fn email_error(error: EngineError) -> AppError {
    let code = match &error {
        EngineError::NoEmailChannel
        | EngineError::EmptySubject
        | EngineError::Manual(ManualError::NotReady(_))
        | EngineError::Manual(ManualError::Tracker(TrackerError::Rule(_)))
        | EngineError::Mail(MailError::Rejected(_)) => ErrorCode::InvalidInput,
        EngineError::AlreadySent(_) | EngineError::NeedsCheck | EngineError::ConfirmResend => {
            ErrorCode::Conflict
        }
        EngineError::Manual(ManualError::Repo(ApplyRepoError::Conflict { .. }))
        | EngineError::Manual(ManualError::Tracker(TrackerError::Repo(
            ApplyRepoError::Conflict { .. },
        ))) => ErrorCode::Conflict,
        EngineError::Mail(MailError::UnknownAccount) => ErrorCode::NotFound,
        EngineError::Mail(MailError::Reauthorize) => ErrorCode::Unauthorized,
        EngineError::Mail(MailError::Transport(_)) => ErrorCode::Unavailable,
        EngineError::Manual(_) | EngineError::Outbox(_) | EngineError::Audit(_) => {
            ErrorCode::Internal
        }
        EngineError::Tracker(_) => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

/// Map an ATS assist failure onto the typed IPC error (AD-3).
fn to_app_ats(error: AtsFormError) -> AppError {
    let code = match &error {
        AtsFormError::NotAtsForm
        | AtsFormError::OptInRequired
        | AtsFormError::NotReady(_)
        | AtsFormError::Answers(boba_app::answers::AnswersError::Edit(_)) => {
            ErrorCode::InvalidInput
        }
        AtsFormError::UnknownAttempt(_) => ErrorCode::NotFound,
        AtsFormError::Answers(boba_app::answers::AnswersError::UnknownApplication(_)) => {
            ErrorCode::NotFound
        }
        AtsFormError::Answers(boba_app::answers::AnswersError::UnknownAnswer(_)) => {
            ErrorCode::NotFound
        }
        AtsFormError::Tracker(TrackerError::Repo(ApplyRepoError::Conflict { .. })) => {
            ErrorCode::Conflict
        }
        _ => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

/// The draft, the verified attachments and the approved answers the review
/// screen shows before Send (S9.3, AC1/AC3).
#[tauri::command]
#[specta::specta]
pub fn apply_email_review(
    app: tauri::AppHandle,
    application_id: String,
) -> AppResult<EmailApplyView> {
    let application_id = required_id(application_id)?;
    let (application, offer) = context(&app, &application_id)?;
    engine(&app)?
        .review(&application, &offer)
        .map_err(email_error)
}

/// Send the reviewed email application (S9.3, AC1).
///
/// The core re-checks the package (AD-14), reads the attempts before sending
/// (AC2) and writes the audit `intent`/`outcome` around the send (AD-12); the
/// screen is never the gate.
#[tauri::command]
#[specta::specta]
pub async fn apply_email_send(
    app: tauri::AppHandle,
    application_id: String,
    request: SendRequest,
) -> AppResult<SendReceipt> {
    let application_id = required_id(application_id)?;
    let (application, offer) = context(&app, &application_id)?;
    engine(&app)?
        .send(&application, &offer, &request)
        .await
        .map_err(email_error)
}

/// The application's answer bank: the drafts to look at and the approved rows
/// (S7.8 surface, S9.4).
#[tauri::command]
#[specta::specta]
pub fn apply_answers(app: tauri::AppHandle, application_id: String) -> AppResult<AnswerBankView> {
    let application_id = required_id(application_id)?;
    ats_service(&app)?
        .answers(&application_id)
        .map_err(to_app_ats)
}

/// Approve one drafted answer, edited or as it stands (S7.8 AC3).
#[tauri::command]
#[specta::specta]
pub fn apply_answer_approve(
    app: tauri::AppHandle,
    application_id: String,
    answer_id: String,
    answer: Option<String>,
) -> AppResult<AnswerBankView> {
    let application_id = required_id(application_id)?;
    let answer_id = required_id(answer_id)?;
    ats_service(&app)?
        .approve(&application_id, &answer_id, answer.as_deref())
        .map_err(to_app_ats)
}

/// Whether the user has accepted the browser warning (AC1).
#[tauri::command]
#[specta::specta]
pub fn apply_ats_opted_in(app: tauri::AppHandle) -> AppResult<bool> {
    ats_service(&app)?.opted_in().map_err(to_app_ats)
}

/// Turn browser assist on or off; the core records the consent (AC1, AD-12).
#[tauri::command]
#[specta::specta]
pub fn apply_ats_opt_in(app: tauri::AppHandle, enabled: bool) -> AppResult<bool> {
    ats_service(&app)?.set_opt_in(enabled).map_err(to_app_ats)
}

/// Open the ATS form in the user's own Chrome and fill it (AC1/AC2).
///
/// Off the UI thread: launching Chrome and reading the page block, and the
/// window the user needs to take over must stay responsive (S9.4 review F2).
#[tauri::command]
#[specta::specta]
pub async fn apply_ats_start(
    app: tauri::AppHandle,
    application_id: String,
) -> AppResult<AtsSessionView> {
    let application_id = required_id(application_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let (application, offer) = context(&app, &application_id)?;
        ats_service(&app)?
            .start(&application, &offer)
            .map_err(to_app_ats)
    })
    .await
    .map_err(|_| CommandError::Store.app_error())?
}

/// Read the page after the user submitted: evidence, Applied or Needs check
/// (AC3, FR-88). Off the UI thread, like `apply_ats_start` (S9.4 review F2).
#[tauri::command]
#[specta::specta]
pub async fn apply_ats_confirm(
    app: tauri::AppHandle,
    application_id: String,
    attempt_id: String,
) -> AppResult<AtsOutcome> {
    let application_id = required_id(application_id)?;
    let attempt_id = required_id(attempt_id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let (application, _offer) = context(&app, &application_id)?;
        ats_service(&app)?
            .confirm(&attempt_id, &application)
            .map_err(to_app_ats)
    })
    .await
    .map_err(|_| CommandError::Store.app_error())?
}

// ---- Auto Rules (S9.6, FR-83/FR-87/FR-108, AD-11) ---------------------------

/// The rules and the global Auto settings (AC1).
#[tauri::command]
#[specta::specta]
pub fn apply_rules_snapshot(app: tauri::AppHandle) -> AppResult<RulesSnapshot> {
    rules_service(&app)?.snapshot().map_err(rules_error)
}

/// Create or edit one Auto Rule; core validates every bound (AC1, AD-11).
#[tauri::command]
#[specta::specta]
pub fn apply_rule_save(app: tauri::AppHandle, input: RuleInput) -> AppResult<RuleView> {
    rules_service(&app)?.save_rule(input).map_err(rules_error)
}

/// Remove one Auto Rule. Returns whether a row went.
#[tauri::command]
#[specta::specta]
pub fn apply_rule_delete(app: tauri::AppHandle, rule_id: String) -> AppResult<bool> {
    rules_service(&app)?
        .delete_rule(&rule_id)
        .map_err(rules_error)
}

/// Turn Auto on (only after the consequences dialog, AC3) or off.
#[tauri::command]
#[specta::specta]
pub fn apply_rules_set_enabled(
    app: tauri::AppHandle,
    enabled: bool,
    confirmed: bool,
) -> AppResult<RulesSnapshot> {
    rules_service(&app)?
        .set_enabled(enabled, confirmed)
        .map_err(rules_error)
}

/// Set the cap across every rule (FR-83); core validates the range.
#[tauri::command]
#[specta::specta]
pub fn apply_rules_set_global_cap(app: tauri::AppHandle, cap: u8) -> AppResult<RulesSnapshot> {
    rules_service(&app)?
        .set_global_cap(cap)
        .map_err(rules_error)
}

/// FR-108: turn Auto off and restore the default caps (AC4).
#[tauri::command]
#[specta::specta]
pub fn apply_rules_reset(app: tauri::AppHandle) -> AppResult<RulesSnapshot> {
    rules_service(&app)?
        .reset_to_defaults()
        .map_err(rules_error)
}

/// Why Auto would or would not send one application (AC2). This is the reason
/// list the screen shows verbatim; the send paths call the same use case
/// in-process, so the preview and enforcement cannot drift.
///
/// `local_minute` is the user's minute of day and `utc_offset_minutes` their
/// offset from UTC, sent by the UI because the active hours and the daily caps
/// are local (FR-83, S9.6 review F1); `None` reads the OS offset here.
#[tauri::command]
#[specta::specta]
pub fn apply_rules_explain(
    app: tauri::AppHandle,
    application_id: String,
    channel: String,
    local_minute: Option<u16>,
    utc_offset_minutes: Option<i32>,
) -> AppResult<DecisionView> {
    let application_id = required_id(application_id)?;
    let channel = RuleChannel::parse(&channel)
        .ok_or_else(|| AppError::invalid_input("errors.apply.rule_invalid"))?;
    let candidate = candidate_for(
        &app,
        &application_id,
        channel,
        local_minute,
        utc_offset_minutes,
    )?;
    // The sample only drives the 5–20 minute gap (AC4); a preview never sends,
    // so the sample is deterministic here and the reason names the longest wait.
    rules_service(&app)?
        .decide(&candidate, 0)
        .map_err(rules_error)
}

/// The facts about one application the policy reads (S9.6 AC2). Both the preview
/// and the Outbox release use it, so what the user sees and what enforcement
/// does cannot drift.
fn candidate_for<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    application_id: &str,
    channel: RuleChannel,
    local_minute: Option<u16>,
    utc_offset_minutes: Option<i32>,
) -> AppResult<PolicyCandidate> {
    let (application, offer) = context(app, application_id)?;

    let card = wiring::port::<dyn ScoringRepo>(app)?
        .latest_card(&application.posting_id)
        .map_err(|_| CommandError::Store.app_error())?;
    let package = wiring::port::<dyn ApplyRepo>(app)?
        .package(application_id)
        .map_err(|_| CommandError::Store.app_error())?;
    let documents = wiring::port::<dyn DocumentsRepo>(app)?
        .list_for_application(application_id)
        .map_err(|_| CommandError::Store.app_error())?;
    let opt_in = wiring::port::<dyn SourcesRepo>(app)?
        .get_optin(&offer.source_id)
        .map_err(|_| CommandError::Store.app_error())?
        .is_some();
    let offset = utc_offset_minutes.unwrap_or_else(rules::local_offset_minutes);

    Ok(PolicyCandidate {
        source_id: offer.source_id.clone(),
        channel,
        company_key: policy::company_key(&offer.company),
        rank: card.as_ref().map(|card| card.card.rank),
        fit: card.as_ref().and_then(|card| card.card.fit),
        verdict: card
            .as_ref()
            .map(|card| card.card.verdict)
            .unwrap_or(Verdict::Poor),
        confidence: card
            .as_ref()
            .map(|card| card.card.confidence)
            .unwrap_or(Confidence::Low),
        gates: card
            .as_ref()
            .map(|card| card.gates.verdict())
            .unwrap_or(GateVerdict::Pass),
        // The owning slices supply these when they land: the fraud badge (E6),
        // the duplicate guard (S9.5) and the agent runtime's lockdown verdict.
        // Until then no badge and no duplicate is known, and the package was
        // not produced by an agent on this path.
        fraud_badge: false,
        documents_verified: !package.is_empty()
            && package.iter().all(|item| item.verified_at.is_some()),
        language_variant_confirmed: documents
            .iter()
            .all(|document| document.language_fallback_from.is_none()),
        duplicate: false,
        agent_used: false,
        cli_tool_lockdown: true,
        source_auto_opt_in: opt_in,
        utc_offset_minutes: offset,
        local_minute: local_minute.unwrap_or_else(|| {
            rules::local_minute_at(&ClockPort::now_rfc3339(&SystemClock), offset)
        }),
    })
}

/// The Auto Rules use case, built from the mounted ports (ADR-012).
fn rules_service<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> AppResult<AutoRulesService> {
    Ok(AutoRulesService::new(
        wiring::port::<dyn AutoRulesRepo>(app)?,
        wiring::port::<dyn SystemRepo>(app)?,
        Arc::new(SystemClock),
    ))
}

/// Map an Auto Rules failure onto the typed IPC error (AD-3, CH7).
fn rules_error(error: RulesError) -> AppError {
    let code = match &error {
        RulesError::InvalidRule(_)
        | RulesError::InvalidSettings(_)
        | RulesError::CannotEnable(_)
        | RulesError::UnknownChannel(_)
        | RulesError::UnknownConfidence(_)
        | RulesError::UnknownVerdict(_) => ErrorCode::InvalidInput,
        RulesError::UnknownRule(_) => ErrorCode::NotFound,
        RulesError::Repo(_) | RulesError::System(_) | RulesError::Clock => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

/// The Outbox panel: the items and every rule's dry run (S9.7, FR-84/FR-85).
#[tauri::command]
#[specta::specta]
pub fn apply_outbox_view(app: tauri::AppHandle) -> AppResult<OutboxView> {
    outbox_service(&app)?.snapshot().map_err(outbox_error)
}

/// Set the hold every automatic send waits (FR-85); core validates the range.
#[tauri::command]
#[specta::specta]
pub fn apply_outbox_set_hold(app: tauri::AppHandle, minutes: u16) -> AppResult<OutboxSettings> {
    outbox_service(&app)?
        .set_hold(minutes)
        .map_err(outbox_error)
}

/// Cancel one item during its hold; nothing is sent for it (FR-85).
#[tauri::command]
#[specta::specta]
pub fn apply_outbox_cancel(app: tauri::AppHandle, id: String) -> AppResult<OutboxItemView> {
    let id = required_id(id)?;
    outbox_service(&app)?.cancel(&id).map_err(outbox_error)
}

/// Edit the subject or the body during the hold; the send uses the edit (FR-85).
#[tauri::command]
#[specta::specta]
pub fn apply_outbox_edit(
    app: tauri::AppHandle,
    id: String,
    subject: Option<String>,
    body: Option<String>,
) -> AppResult<OutboxItemView> {
    let id = required_id(id)?;
    outbox_service(&app)?
        .edit(&id, subject, body)
        .map_err(outbox_error)
}

/// End one rule's dry run and let it send from now on (FR-84 AC1).
#[tauri::command]
#[specta::specta]
pub fn apply_outbox_activate_rule(app: tauri::AppHandle, rule_id: String) -> AppResult<DryRunView> {
    let rule_id = required_id(rule_id)?;
    outbox_service(&app)?
        .activate_rule(&rule_id)
        .map_err(outbox_error)
}

/// Set how many matches one rule's dry run reports (FR-84, minimum 1).
#[tauri::command]
#[specta::specta]
pub fn apply_outbox_set_dry_run(
    app: tauri::AppHandle,
    rule_id: String,
    matches: u8,
) -> AppResult<DryRunView> {
    let rule_id = required_id(rule_id)?;
    outbox_service(&app)?
        .set_dry_run(&rule_id, matches)
        .map_err(outbox_error)
}

/// Release every item whose hold has ended (FR-85). The policy runs again for
/// each one; a still-Auto email is sent from the connected mailbox, and anything
/// else moves to the review queue. Returns the items it touched.
#[tauri::command]
#[specta::specta]
pub async fn apply_outbox_release_due(app: tauri::AppHandle) -> AppResult<Vec<OutboxItemView>> {
    release_due(&app).await
}

/// The release sweep, run by the scheduler tick every minute (S9.7, FR-85).
pub fn spawn_outbox_sweep<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = release_due(&app).await {
            tracing::warn!(key = error.message_key, "outbox: release sweep failed");
        }
    });
}

/// One release sweep at a time, process-wide (review B-3): the scheduler tick
/// and the Today button can overlap, and only one may read a Source's spacing
/// window. A second sweep returns at once instead of blocking the tick.
#[derive(Debug)]
struct SweepLock(std::sync::atomic::AtomicBool);

impl SweepLock {
    fn try_enter(&self) -> Option<SweepGuard<'_>> {
        self.0
            .compare_exchange(
                false,
                true,
                std::sync::atomic::Ordering::SeqCst,
                std::sync::atomic::Ordering::SeqCst,
            )
            .ok()
            .map(|_| SweepGuard(self))
    }
}

/// Releases the sweep lock when the sweep ends, however it ends.
struct SweepGuard<'a>(&'a SweepLock);

impl Drop for SweepGuard<'_> {
    fn drop(&mut self) {
        self.0.0.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

static SWEEP: SweepLock = SweepLock(std::sync::atomic::AtomicBool::new(false));

/// One release sweep. A hold that ended is re-checked against the policy, and a
/// send that dies here leaves the attempt evidence the review screen reads.
async fn release_due<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> AppResult<Vec<OutboxItemView>> {
    let Some(_running) = SWEEP.try_enter() else {
        return Ok(Vec::new());
    };
    let service = outbox_service(app)?;
    let due = service.due().map_err(outbox_error)?;
    Ok(sweep_all(
        due,
        |item| sweep_item(app, &service, item),
        |item, error| {
            // One broken item never stalls the rest (review B-1). It leaves the
            // automatic path for good: the user reviews it by hand.
            tracing::warn!(outbox = %item.id, %error, "outbox: due item left for review");
            // A held item lost its context before the claim; a `releasing` one
            // failed after it (`send_email` already tried to park it).
            let note = if item.state == OutboxState::Held {
                "context_missing"
            } else {
                "send_failed"
            };
            if let Err(error) = service.park(&item.id, note) {
                tracing::warn!(outbox = %item.id, %error, "outbox: item could not be parked");
            }
        },
    )
    .await)
}

/// Run the per-item sweep, isolating failures: an item whose context is gone is
/// parked and every later due item still gets its turn (review B-1).
async fn sweep_all<T, F, Fut, E>(
    items: Vec<boba_domain::apply::outbox::OutboxItem>,
    mut sweep: F,
    mut on_error: impl FnMut(&boba_domain::apply::outbox::OutboxItem, &E),
) -> Vec<T>
where
    F: FnMut(boba_domain::apply::outbox::OutboxItem) -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, E>>,
{
    let mut touched = Vec::new();
    for item in items {
        match sweep(item.clone()).await {
            Ok(Some(value)) => touched.push(value),
            Ok(None) => {}
            Err(error) => on_error(&item, &error),
        }
    }
    touched
}

/// Release one due item, if it still may go; `None` when the Source's spacing
/// (or the clock) asked it to wait.
async fn sweep_item<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    service: &OutboxService,
    item: boba_domain::apply::outbox::OutboxItem,
) -> AppResult<Option<OutboxItemView>> {
    // Only Email Apply has an automatic channel in this build: an ATS form waits
    // for the user (S9.4) and the Session Board is S11.3's. The item goes to the
    // review queue rather than pretending to send.
    if item.channel != "email" {
        return Ok(Some(
            service
                .park(&item.id, "channel_unavailable")
                .map_err(outbox_error)?,
        ));
    }
    let channel = RuleChannel::parse(&item.channel)
        .ok_or_else(|| AppError::invalid_input("errors.apply.rule_invalid"))?;
    let candidate = candidate_for(app, &item.application_id, channel, None, None)?;
    match service
        .release(&item.id, &candidate, spacing_sample())
        .map_err(outbox_error)?
    {
        Release::Review(_) => Ok(Some(service.item_view(&item.id).map_err(outbox_error)?)),
        // The Source's anti-ban gap is not over: the item stays held and the
        // next tick tries again (FR-91).
        Release::Deferred { .. } => Ok(None),
        Release::Sending {
            item: stored,
            rule_id,
        } => Ok(Some(
            send_email(app, service, &stored, &rule_id, &candidate).await?,
        )),
    }
}

/// Send one released Email Apply and record how it ended. An error here must
/// never leave the claimed item `releasing` until the next launch (review B-2),
/// so every error path parks it first.
async fn send_email<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    service: &OutboxService,
    item: &boba_domain::apply::outbox::OutboxItem,
    rule_id: &str,
    candidate: &PolicyCandidate,
) -> AppResult<OutboxItemView> {
    with_cleanup(
        || deliver_email(app, service, item, rule_id, candidate),
        || {
            if let Err(error) = service.finish(&item.id, Finish::NeedsReview) {
                tracing::warn!(outbox = %item.id, %error, "outbox: could not park a failed send");
            }
        },
    )
    .await
}

/// Run one delivery, and on **any** error run the cleanup before returning it
/// (review B-2): a claimed item never stays `releasing` until the next launch.
async fn with_cleanup<T, F, Fut, C>(deliver: F, cleanup: C) -> AppResult<T>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = AppResult<T>>,
    C: FnOnce(),
{
    let result = deliver().await;
    if result.is_err() {
        cleanup();
    }
    result
}

/// The send itself: mailbox, pre-submit check, cap entry, channel.
async fn deliver_email<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    service: &OutboxService,
    item: &boba_domain::apply::outbox::OutboxItem,
    rule_id: &str,
    candidate: &PolicyCandidate,
) -> AppResult<OutboxItemView> {
    let account = wiring::port::<dyn EmailRepo>(app)?
        .list_accounts()
        .map_err(|_| CommandError::Store.app_error())?
        .into_iter()
        .next();
    let Some(account) = account else {
        // No mailbox: nothing is sent and nothing is counted; the user connects
        // one and sends by hand meanwhile (review B-3).
        return service.park(&item.id, "no_mailbox").map_err(outbox_error);
    };
    // The last enforcement point (S9.6 AC2): the Kill Switch and `decide()` run
    // again immediately before the message is composed and sent.
    if service
        .confirm_before_submit(&item.id, candidate, spacing_sample())
        .map_err(outbox_error)?
        == Submit::Stop
    {
        return service.item_view(&item.id).map_err(outbox_error);
    }
    // The cap entry is written now: the mailbox exists and the policy said Go,
    // so this send is on its way (S9.6 AC2).
    service
        .record_send(&item.id, rule_id)
        .map_err(outbox_error)?;
    let (application, offer) = context(app, &item.application_id)?;
    let request = SendRequest {
        account_id: account.id,
        subject: item.subject_override.clone(),
        body_text: item.body_override.clone(),
        confirm_resend: false,
    };
    match engine(app)?.send(&application, &offer, &request).await {
        Ok(_) => service.finish(&item.id, Finish::Sent).map_err(outbox_error),
        Err(_) => {
            // The attempt row holds the evidence; an automatic retry would be a
            // second send, so the item goes to the user.
            service
                .finish(&item.id, Finish::NeedsReview)
                .map_err(outbox_error)
        }
    }
}

/// A random sample for the S9.6 spacing gap, drawn from the system clock.
fn spacing_sample() -> u64 {
    time::OffsetDateTime::now_utc().unix_timestamp_nanos() as u64
}

/// The Outbox use case, built from the mounted ports (ADR-012).
fn outbox_service<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> AppResult<OutboxService> {
    Ok(OutboxService::new(
        wiring::port::<dyn OutboxHoldRepo>(app)?,
        Arc::new(rules_service(app)?),
        wiring::port::<dyn SystemRepo>(app)?,
        Arc::new(SystemClock),
        Some(Arc::new(crate::scheduler::notify::TauriNotifier::new(
            app.clone(),
        ))),
        // The Kill Switch (S9.8) mounts its adapter next wave; until then the
        // port is unmounted, which reads as "not engaged".
        wiring::port::<dyn KillSwitchPort>(app).ok(),
    ))
}

/// Map an Outbox failure onto the typed IPC error (AD-3, CH7).
fn outbox_error(error: OutboxError) -> AppError {
    let code = match &error {
        OutboxError::Hold(_)
        | OutboxError::DryRun(_)
        | OutboxError::Edit
        | OutboxError::UnknownChannel(_) => ErrorCode::InvalidInput,
        OutboxError::UnknownItem(_) => ErrorCode::NotFound,
        OutboxError::UnknownRule(_) => ErrorCode::NotFound,
        OutboxError::NotWaiting(_) | OutboxError::AlreadyQueued(_) => ErrorCode::Conflict,
        OutboxError::KillSwitch => ErrorCode::Conflict,
        OutboxError::Rules(rules) => return rules_error(rules.clone()),
        OutboxError::Repo(_) | OutboxError::System(_) | OutboxError::Clock => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

/// The folder one application's kit is written to, created on demand.
fn kit_dir(app: &tauri::AppHandle, application_id: &str) -> AppResult<PathBuf> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|_| CommandError::ExportPath.app_error())?;
    // `required_id` already refused anything that is not a bare id, so the id
    // cannot climb out of the exports root.
    let dir = base.join("exports").join("apply").join(application_id);
    std::fs::create_dir_all(&dir).map_err(|_| CommandError::ExportPath.app_error())?;
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_domain::apply::outbox::{DryRunViolation, HoldViolation, OutboxItem};

    #[test]
    fn ats_errors_map_to_codes_and_i18n_keys() {
        let mapped = to_app_ats(AtsFormError::OptInRequired);
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.apply.ats_opt_in");

        let mapped = to_app_ats(AtsFormError::NotAtsForm);
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.apply.ats_not_a_form");

        let mapped = to_app_ats(AtsFormError::UnknownAttempt("ats_x".into()));
        assert_eq!(mapped.code, ErrorCode::NotFound);
        assert_eq!(mapped.message_key, "errors.apply.ats_unknown_attempt");

        let mapped = to_app_ats(AtsFormError::NotReady(vec!["cv".into()]));
        assert_eq!(mapped.message_key, "errors.apply.not_ready");
    }

    #[test]
    fn errors_map_to_codes_and_i18n_keys() {
        let mapped = to_app(ManualError::NotReady(vec!["cv".into()]));
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.apply.not_ready");

        let mapped = to_app(ManualError::Tracker(TrackerError::Rule(
            boba_app::applications::RuleError::SameStatus,
        )));
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.tracker.same_status");

        let mapped = to_app(ManualError::Repo(ApplyRepoError::UnknownApplication(
            "app_1".into(),
        )));
        assert_eq!(mapped.code, ErrorCode::NotFound);
        assert_eq!(mapped.message_key, "errors.apply.unknown_application");

        let mapped = to_app(ManualError::Documents(
            boba_app::ports::repo::documents::DocumentsRepoError::new("disk"),
        ));
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message_key, "errors.apply.store");
    }

    #[test]
    fn email_failures_map_to_the_ui_contract() {
        for (error, code, key) in [
            (
                EngineError::NoEmailChannel,
                ErrorCode::InvalidInput,
                "errors.apply.no_email_channel",
            ),
            (
                EngineError::AlreadySent("m".into()),
                ErrorCode::Conflict,
                "errors.apply.already_sent",
            ),
            (
                EngineError::NeedsCheck,
                ErrorCode::Conflict,
                "errors.apply.needs_check",
            ),
            (
                EngineError::ConfirmResend,
                ErrorCode::Conflict,
                "errors.apply.confirm_resend",
            ),
            (
                EngineError::EmptySubject,
                ErrorCode::InvalidInput,
                "errors.apply.empty_subject",
            ),
            (
                EngineError::Mail(MailError::Transport("offline".into())),
                ErrorCode::Unavailable,
                "errors.apply.send_unreachable",
            ),
            (
                EngineError::Mail(MailError::Rejected("refused".into())),
                ErrorCode::InvalidInput,
                "errors.apply.send_rejected",
            ),
            (
                EngineError::Mail(MailError::Reauthorize),
                ErrorCode::Unauthorized,
                "errors.apply.reauthorize",
            ),
            (
                EngineError::Mail(MailError::UnknownAccount),
                ErrorCode::NotFound,
                "errors.apply.unknown_account",
            ),
            (
                EngineError::Manual(ManualError::NotReady(vec!["cv".into()])),
                ErrorCode::InvalidInput,
                "errors.apply.not_ready",
            ),
        ] {
            let mapped = email_error(error);
            assert_eq!(mapped.code, code);
            assert_eq!(mapped.message_key, key);
        }
    }

    #[test]
    fn outbox_failures_map_to_codes_and_i18n_keys() {
        for (error, code, key) in [
            (
                OutboxError::Hold(HoldViolation::TooShort),
                ErrorCode::InvalidInput,
                "errors.apply.outbox_hold",
            ),
            (
                OutboxError::DryRun(DryRunViolation::TooLong),
                ErrorCode::InvalidInput,
                "errors.apply.outbox_dry_run",
            ),
            (
                OutboxError::UnknownItem("out_1".into()),
                ErrorCode::NotFound,
                "errors.apply.outbox_not_found",
            ),
            (
                OutboxError::NotWaiting("out_1".into()),
                ErrorCode::Conflict,
                "errors.apply.outbox_not_live",
            ),
            (
                OutboxError::AlreadyQueued("app_1".into()),
                ErrorCode::Conflict,
                "errors.apply.outbox_queued",
            ),
            (
                OutboxError::Edit,
                ErrorCode::InvalidInput,
                "errors.apply.outbox_edit",
            ),
            (
                OutboxError::KillSwitch,
                ErrorCode::Conflict,
                "errors.apply.outbox_kill_switch",
            ),
            (
                OutboxError::Clock,
                ErrorCode::Internal,
                "errors.apply.outbox_clock",
            ),
        ] {
            let mapped = outbox_error(error);
            assert_eq!(mapped.code, code);
            assert_eq!(mapped.message_key, key);
        }
    }

    /// Poll a future to completion on one thread (the same helper the engine
    /// tests use): the sweep's fakes answer immediately.
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        use std::task::{Context, Poll, Waker};

        let mut context = Context::from_waker(Waker::noop());
        let mut future = std::pin::pin!(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(value) => return value,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }

    fn due_item(id: &str) -> OutboxItem {
        OutboxItem {
            id: id.to_string(),
            application_id: "app_01".to_string(),
            rule_id: "rule_1".to_string(),
            source_id: "remotive".to_string(),
            channel: "email".to_string(),
            company_key: "acme".to_string(),
            title: "Backend Engineer".to_string(),
            company: "Acme".to_string(),
            state: OutboxState::Held,
            hold_minutes: 30,
            release_at: "2026-10-17T09:00:00Z".to_string(),
            subject_override: None,
            body_override: None,
            note: None,
            created_at: "2026-10-17T08:30:00Z".to_string(),
            updated_at: "2026-10-17T08:30:00Z".to_string(),
        }
    }

    #[test]
    fn one_broken_due_item_never_blocks_the_others() {
        let items = vec![due_item("out_1"), due_item("out_2"), due_item("out_3")];
        let mut parked = Vec::new();
        let touched = block_on(sweep_all(
            items,
            |item| {
                let id = item.id.clone();
                async move {
                    if id == "out_2" {
                        Err("context missing")
                    } else {
                        Ok(Some(id))
                    }
                }
            },
            |item, error| parked.push(format!("{}:{error}", item.id)),
        ));

        assert_eq!(touched, vec!["out_1".to_string(), "out_3".to_string()]);
        assert_eq!(parked, vec!["out_2:context missing".to_string()]);
    }

    #[test]
    fn only_one_sweep_runs_at_a_time() {
        let lock = SweepLock(std::sync::atomic::AtomicBool::new(false));
        let first = lock.try_enter().expect("the first sweep enters");
        assert!(lock.try_enter().is_none(), "a second sweep returns at once");
        drop(first);
        assert!(lock.try_enter().is_some(), "the lock is free again");
    }

    #[test]
    fn an_error_after_the_claim_always_runs_the_cleanup() {
        let cleaned = std::cell::Cell::new(0);
        let failed = block_on(with_cleanup(
            || async { Err::<(), AppError>(AppError::invalid_input("errors.apply.store")) },
            || cleaned.set(cleaned.get() + 1),
        ));
        assert!(failed.is_err());
        assert_eq!(cleaned.get(), 1, "the claimed item is parked (review B-2)");

        let sent = block_on(with_cleanup(
            || async { Ok::<_, AppError>(7) },
            || cleaned.set(cleaned.get() + 1),
        ));
        assert_eq!(sent.unwrap(), 7);
        assert_eq!(cleaned.get(), 1, "a successful send cleans nothing");
    }
}
