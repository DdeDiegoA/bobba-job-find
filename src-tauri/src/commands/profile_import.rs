//! Master CV import commands (S3.5, FR-5): thin wrappers over
//! [`ImportService`]. Resolve the ports, call the use case, map the error (AD-3).
//!
//! The store and the agent runtime arrive through [`wiring::port`] (ADR-012): a
//! locked or unmounted port is the typed error, never a panic. The chosen CLI
//! is the same `setting` row the engine step wrote (S3.2), so the wizard's
//! answer is what the import runs on.
//!
//! Nothing here saves a fact: `start` only stores pending proposals, and
//! `confirm` saves the one the user approved (AD-5, AC1).

use std::path::PathBuf;
use std::sync::Arc;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::agent::AgentPort;
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::profile::ProfileSnapshot;
use boba_app::profile_import::{
    FactEdit, ImportError, ImportFile, ImportOutcome, ImportRequest, ImportService, ProposalView,
    TextError, read_files,
};
use boba_app::system::SystemClock;

use crate::commands::agent::{ENGINE_KEY, EngineChoice};
use crate::wiring;

/// The failures only a command can hit, each with its own key (CH7:
/// `errors.profile_import.*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandError {
    Store,
    NoEngine,
    Nothing,
    Run,
}

impl CommandError {
    /// The key the UI translates (the gate scans `message_key` fns, AC4).
    const fn message_key(self) -> &'static str {
        match self {
            CommandError::Store => "errors.profile_import.store",
            CommandError::NoEngine => "errors.profile_import.no_engine",
            CommandError::Nothing => "errors.profile_import.nothing",
            CommandError::Run => "errors.profile_import.run",
        }
    }

    fn app_error(self) -> AppError {
        let code = match self {
            CommandError::NoEngine | CommandError::Nothing => ErrorCode::InvalidInput,
            CommandError::Store | CommandError::Run => ErrorCode::Internal,
        };
        AppError::new(code, self.message_key())
    }
}

/// The service over the mounted ports (ADR-012: ports come only from the
/// helper).
fn service(app: &tauri::AppHandle) -> AppResult<ImportService> {
    Ok(ImportService::new(
        wiring::port::<dyn ProfileRepo>(app)?,
        wiring::port::<dyn AgentPort>(app)?,
        Arc::new(SystemClock),
    ))
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: ImportError) -> AppError {
    let code = match &error {
        ImportError::Profile(
            boba_app::profile::ProfileError::Rule(_)
            | boba_app::profile::ProfileError::UnknownKind(_),
        ) => ErrorCode::InvalidInput,
        ImportError::Profile(
            boba_app::profile::ProfileError::UnknownItem(_)
            | boba_app::profile::ProfileError::UnknownRevision(_),
        )
        | ImportError::UnknownProposal(_) => ErrorCode::NotFound,
        ImportError::Text(TextError::NoText { .. })
        | ImportError::Text(TextError::Unsupported { .. })
        | ImportError::Text(TextError::TooLarge) => ErrorCode::InvalidInput,
        ImportError::Repo(_)
        | ImportError::Profile(_)
        | ImportError::Text(_)
        | ImportError::Contract(_)
        | ImportError::Run(_) => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

/// The engine the user picked in the wizard's previous step (S3.2). Without
/// one, no import can run: the step says so instead of guessing a CLI.
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

/// Read the user's picked files, cap the size, and hand the text to one
/// `profile_import` run (FR-5, AC1). Returns the proposals now waiting for
/// review; nothing has been saved to the profile.
///
/// Off the UI thread (S3.5 review F1): the run waits on an agent CLI and can
/// take minutes, so the step's spinner keeps animating and the window stays
/// responsive, exactly like `agent_test_call` and `apply_ats_start`.
///
/// The paths are the ones the picker returned; the core limits what can be
/// read by extension and size, but it cannot know what the dialog granted
/// (S3.5 review F3 residual: a compromised webview could name another local
/// `.pdf`/`.txt` — the file would then be read and sent to the user's own CLI).
#[tauri::command]
#[specta::specta]
pub async fn profile_import_start(
    app: tauri::AppHandle,
    files: Vec<String>,
    pasted_text: Option<String>,
    language: String,
) -> AppResult<ImportOutcome> {
    tauri::async_runtime::spawn_blocking(move || import(&app, files, pasted_text, language))
        .await
        .map_err(|_| CommandError::Run.app_error())?
}

/// The blocking body of [`profile_import_start`].
fn import(
    app: &tauri::AppHandle,
    files: Vec<String>,
    pasted_text: Option<String>,
    language: String,
) -> AppResult<ImportOutcome> {
    let paths: Vec<PathBuf> = files
        .iter()
        .map(|path| path.trim())
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .collect();
    let mut imported = read_files(&paths).map_err(|error| to_app(error.into()))?;
    if let Some(text) = pasted_text.filter(|text| !text.trim().is_empty()) {
        imported.push(ImportFile {
            name: "pasted-cv.txt".to_string(),
            bytes: text.into_bytes(),
        });
    }
    if imported.is_empty() {
        return Err(CommandError::Nothing.app_error());
    }
    let engine = chosen_engine(app)?;
    service(app)?
        .start(ImportRequest {
            cli: engine.cli,
            model: engine.model,
            target_language: language,
            files: imported,
        })
        .map_err(to_app)
}

/// Every proposal still waiting for the user's decision (AC1).
#[tauri::command]
#[specta::specta]
pub fn profile_import_proposals(app: tauri::AppHandle) -> AppResult<Vec<ProposalView>> {
    service(&app)?.proposals().map_err(to_app)
}

/// Confirm a proposal — exactly as proposed, or with the user's edit — and
/// save it as a profile fact (AC1/AC3).
#[tauri::command]
#[specta::specta]
pub fn profile_import_confirm(
    app: tauri::AppHandle,
    id: String,
    edit: Option<FactEdit>,
) -> AppResult<ProfileSnapshot> {
    service(&app)?.confirm(&id, edit).map_err(to_app)
}

/// Delete a proposal: it never becomes a fact (AC1).
#[tauri::command]
#[specta::specta]
pub fn profile_import_discard(app: tauri::AppHandle, id: String) -> AppResult<()> {
    service(&app)?.discard(&id).map_err(to_app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_app::ports::repo::profile::ProfileRepoError;

    #[test]
    fn errors_map_to_codes_and_i18n_keys() {
        let mapped = to_app(ImportError::Text(TextError::NoText {
            file: "scan.pdf".to_string(),
        }));
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.profile_import.no_text");

        let mapped = to_app(ImportError::Text(TextError::Read {
            file: "cv.pdf".to_string(),
        }));
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message_key, "errors.profile_import.read");

        let mapped = to_app(ImportError::UnknownProposal("01X".to_string()));
        assert_eq!(mapped.code, ErrorCode::NotFound);
        assert_eq!(mapped.message_key, "errors.profile_import.unknown_proposal");

        let mapped = to_app(ImportError::Repo(ProfileRepoError::new("disk")));
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message_key, "errors.profile_import.store");

        let mapped = to_app(ImportError::Profile(
            boba_app::profile::ProfileError::UnknownKind("hobby".to_string()),
        ));
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.profile.unknown_kind");
    }

    #[test]
    fn command_errors_carry_their_own_keys() {
        assert_eq!(
            CommandError::NoEngine.app_error().message_key,
            "errors.profile_import.no_engine"
        );
        assert_eq!(
            CommandError::Nothing.app_error().message_key,
            "errors.profile_import.nothing"
        );
    }
}
