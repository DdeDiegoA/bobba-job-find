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

use boba_app::applications::TrackerError;
use boba_app::apply::engine::outbox::OutboxRepo;
use boba_app::apply::engine::{ApplyEngine, EmailApplyView, EngineError, SendReceipt, SendRequest};
use boba_app::apply::manual::{ManualError, ManualService, ManualView};
use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::mail::{MailError, MailPort};
use boba_app::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use boba_app::ports::repo::documents::DocumentsRepo;
use boba_app::ports::repo::ingestion::IngestionRepo;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::system::SystemClock;
use boba_domain::application::Application;
use boba_domain::posting::Offer;
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
