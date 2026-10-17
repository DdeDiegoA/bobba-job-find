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
use boba_app::apply::rules::{
    self, AutoRulesService, DecisionView, RuleInput, RuleView, RulesError, RulesSnapshot,
};
use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::browser::BrowserPort;
use boba_app::ports::clock::ClockPort;
use boba_app::ports::mail::{MailError, MailPort};
use boba_app::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use boba_app::ports::repo::auto_rules::AutoRulesRepo;
use boba_app::ports::repo::documents::DocumentsRepo;
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::outbox::OutboxRepo;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::scoring::ScoringRepo;
use boba_app::ports::repo::sources::SourcesRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::system::SystemClock;
use boba_domain::application::Application;
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
fn context(app: &tauri::AppHandle, application_id: &str) -> AppResult<(Application, Offer)> {
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
fn engine(app: &tauri::AppHandle) -> AppResult<ApplyEngine> {
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
    let (application, offer) = context(&app, &application_id)?;

    let card = wiring::port::<dyn ScoringRepo>(&app)?
        .latest_card(&application.posting_id)
        .map_err(|_| CommandError::Store.app_error())?;
    let package = wiring::port::<dyn ApplyRepo>(&app)?
        .package(&application_id)
        .map_err(|_| CommandError::Store.app_error())?;
    let documents = wiring::port::<dyn DocumentsRepo>(&app)?
        .list_for_application(&application_id)
        .map_err(|_| CommandError::Store.app_error())?;
    let opt_in = wiring::port::<dyn SourcesRepo>(&app)?
        .get_optin(&offer.source_id)
        .map_err(|_| CommandError::Store.app_error())?
        .is_some();
    let offset = utc_offset_minutes.unwrap_or_else(rules::local_offset_minutes);

    let candidate = PolicyCandidate {
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
    };
    // The sample only drives the 5–20 minute gap (AC4); a preview never sends,
    // so the sample is deterministic here and the reason names the longest wait.
    rules_service(&app)?
        .decide(&candidate, 0)
        .map_err(rules_error)
}

/// The Auto Rules use case, built from the mounted ports (ADR-012).
fn rules_service(app: &tauri::AppHandle) -> AppResult<AutoRulesService> {
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
}
