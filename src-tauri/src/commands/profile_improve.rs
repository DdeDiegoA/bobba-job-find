//! Fact improvement commands (FX10a): thin wrappers over
//! [`ImproveService`] (AD-3).
//!
//! The run proposes; the core disposes. `profile_improve_start` dispatches an
//! `improve_fact` run through the agent runtime (its untrusted task kind takes
//! the per-run lockdown gate), and nothing reaches the profile until
//! `profile_improve_confirm` accepts that one suggestion. A stale confirm —
//! the fact changed after the suggestion was made — is refused with nothing
//! written. A successful confirm is audited (ADR-011).
//!
//! The store, the agent runtime and the audit log arrive through
//! [`wiring::port`] (ADR-012); a locked or unmounted port is the typed error,
//! never a panic.

use std::sync::Arc;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::agent::AgentPort;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::profile::ProfileSnapshot;
use boba_app::profile::improve::{
    ImproveOutcome, ImproveRequest, ImproveService, ProfileImproveError,
};
use boba_app::profile_import::FactEdit;
use boba_app::system::SystemClock;

use crate::commands::agent::{ENGINE_KEY, EngineChoice};
use crate::wiring;

/// The failures only a command can hit, each with its own key (CH7:
/// `errors.profile_improve.*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandError {
    Store,
    NoEngine,
    Run,
}

impl CommandError {
    /// The key the UI translates (the gate scans `message_key` fns, AC4).
    const fn message_key(self) -> &'static str {
        match self {
            CommandError::Store => "errors.profile_improve.store",
            CommandError::NoEngine => "errors.profile_improve.no_engine",
            CommandError::Run => "errors.profile_improve.run",
        }
    }

    fn app_error(self) -> AppError {
        let code = match self {
            CommandError::NoEngine => ErrorCode::InvalidInput,
            CommandError::Store | CommandError::Run => ErrorCode::Internal,
        };
        AppError::new(code, self.message_key())
    }
}

/// The service over the mounted ports (ADR-012: ports come only from the
/// helper).
fn service(app: &tauri::AppHandle) -> AppResult<ImproveService> {
    Ok(ImproveService::new(
        wiring::port::<dyn ProfileRepo>(app)?,
        wiring::port::<dyn AgentPort>(app)?,
        Arc::new(SystemClock),
        wiring::port::<dyn SystemRepo>(app)?,
    ))
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: ProfileImproveError) -> AppError {
    use boba_app::profile::ProfileError;
    let code = match &error {
        ProfileImproveError::UnknownItem(_) | ProfileImproveError::UnknownProposal(_) => {
            ErrorCode::NotFound
        }
        ProfileImproveError::Stale(_) => ErrorCode::Conflict,
        ProfileImproveError::Profile(ProfileError::Rule(_) | ProfileError::UnknownKind(_)) => {
            ErrorCode::InvalidInput
        }
        ProfileImproveError::Profile(
            ProfileError::UnknownItem(_) | ProfileError::UnknownRevision(_),
        ) => ErrorCode::NotFound,
        ProfileImproveError::Store
        | ProfileImproveError::Contract
        | ProfileImproveError::Run(_)
        | ProfileImproveError::Profile(_) => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

/// The engine the user picked in the wizard's engine step (S3.2). Without one,
/// the improvement cannot ask the agent and the step says so.
fn chosen_engine(app: &tauri::AppHandle) -> AppResult<EngineChoice> {
    let repo = wiring::port::<dyn SystemRepo>(app)?;
    let raw = repo
        .setting_get(ENGINE_KEY)
        .map_err(|_| CommandError::Store.app_error())?;
    let choice: Option<EngineChoice> = raw
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .filter(|choice: &EngineChoice| !choice.cli.trim().is_empty());
    choice.ok_or_else(|| CommandError::NoEngine.app_error())
}

/// Run one `improve_fact` pass for a fact (AC1, AC2). Runs off the UI thread:
/// the run waits on an agent CLI and can take minutes (S3.5 review F1).
#[tauri::command]
#[specta::specta]
pub async fn profile_improve_start(
    app: tauri::AppHandle,
    item_id: String,
    language: String,
) -> AppResult<ImproveOutcome> {
    tauri::async_runtime::spawn_blocking(move || start(&app, item_id, language))
        .await
        .map_err(|_| CommandError::Run.app_error())?
}

fn start(app: &tauri::AppHandle, item_id: String, language: String) -> AppResult<ImproveOutcome> {
    let engine = chosen_engine(app)?;
    service(app)?
        .start(ImproveRequest {
            item_id,
            cli: engine.cli,
            model: engine.model,
            target_language: language,
        })
        .map_err(to_app)
}

/// Confirm one suggestion — as proposed, or with the user's edit (AC2). A
/// stale suggestion is refused with nothing written (AC3).
#[tauri::command]
#[specta::specta]
pub fn profile_improve_confirm(
    app: tauri::AppHandle,
    id: String,
    edit: Option<FactEdit>,
) -> AppResult<ProfileSnapshot> {
    service(&app)?.confirm(&id, edit).map_err(to_app)
}

/// Reject one suggestion: it never becomes a fact (FX10b's Reject).
#[tauri::command]
#[specta::specta]
pub fn profile_improve_discard(app: tauri::AppHandle, id: String) -> AppResult<()> {
    service(&app)?.discard(&id).map_err(to_app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_map_to_codes_and_i18n_keys() {
        let mapped = to_app(ProfileImproveError::UnknownItem("01X".to_string()));
        assert_eq!(mapped.code, ErrorCode::NotFound);
        assert_eq!(mapped.message_key, "errors.profile_improve.unknown_item");

        let mapped = to_app(ProfileImproveError::Stale("01P".to_string()));
        assert_eq!(mapped.code, ErrorCode::Conflict);
        assert_eq!(mapped.message_key, "errors.profile_improve.stale");

        let mapped = to_app(ProfileImproveError::Contract);
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message_key, "errors.profile_improve.contract");

        let mapped = to_app(ProfileImproveError::Store);
        assert_eq!(mapped.message_key, "errors.profile_improve.store");
    }

    #[test]
    fn command_errors_carry_their_own_keys() {
        assert_eq!(
            CommandError::NoEngine.app_error().message_key,
            "errors.profile_improve.no_engine"
        );
        assert_eq!(
            CommandError::Store.app_error().message_key,
            "errors.profile_improve.store"
        );
    }
}
