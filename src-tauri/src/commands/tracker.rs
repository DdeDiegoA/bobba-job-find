//! Tracker commands (S9.1, FR-93/94): the board, one card in full, and the one
//! command that moves a card. Thin wrappers over [`TrackerService`]; validate,
//! call the use case, map the error (AD-3).
//!
//! The store arrives through [`wiring::port`] (ADR-012): a locked or unmounted
//! store is `errors.store.locked`, never a panic.

use std::sync::Arc;

use boba_app::applications::{
    ApplicationDetail, BoardView, TrackerError, TrackerService, TransitionInput,
};
use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::repo::apply::{ApplyRepo, ApplyRepoError};
use boba_app::system::SystemClock;

use crate::wiring;

/// The service over the mounted repo (ADR-012: ports come only from the helper).
fn service(app: &tauri::AppHandle) -> AppResult<TrackerService> {
    let repo = wiring::port::<dyn ApplyRepo>(app)?;
    Ok(TrackerService::new(repo, Arc::new(SystemClock)))
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: TrackerError) -> AppError {
    let code = match &error {
        TrackerError::Repo(ApplyRepoError::Conflict { .. }) => ErrorCode::Conflict,
        TrackerError::Repo(ApplyRepoError::UnknownApplication(_))
        | TrackerError::UnknownApplication(_) => ErrorCode::NotFound,
        TrackerError::Repo(ApplyRepoError::Store(_)) => ErrorCode::Internal,
        TrackerError::Rule(_) | TrackerError::UnknownStatus(_) => ErrorCode::InvalidInput,
    };
    AppError::new(code, error.message_key())
}

fn required_id(id: String) -> AppResult<String> {
    if id.trim().is_empty() {
        return Err(AppError::invalid_input("errors.tracker.missing_id"));
    }
    Ok(id)
}

/// The board: every Stage column with its cards (AC1).
#[tauri::command]
#[specta::specta]
pub fn tracker_board(app: tauri::AppHandle) -> AppResult<BoardView> {
    service(&app)?.board().map_err(to_app)
}

/// One card in full: snapshot, package, timeline, next action, notes (AC3).
#[tauri::command]
#[specta::specta]
pub fn tracker_application_detail(
    app: tauri::AppHandle,
    application_id: String,
) -> AppResult<ApplicationDetail> {
    let application_id = required_id(application_id)?;
    service(&app)?.detail(&application_id).map_err(to_app)
}

/// Move one card (AC2/AC4). The only way a status changes: it logs a
/// `status_event` and returns the updated card.
#[tauri::command]
#[specta::specta]
pub fn tracker_transition_status(
    app: tauri::AppHandle,
    input: TransitionInput,
) -> AppResult<ApplicationDetail> {
    service(&app)?.transition(input).map_err(to_app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_app::applications::RuleError;

    #[test]
    fn errors_map_to_codes_and_i18n_keys() {
        let mapped = to_app(TrackerError::Rule(RuleError::SameStatus));
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.tracker.same_status");

        let mapped = to_app(TrackerError::UnknownStatus("needs_check".to_string()));
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.tracker.unknown_status");

        let mapped = to_app(TrackerError::Repo(ApplyRepoError::UnknownApplication(
            "app_01".to_string(),
        )));
        assert_eq!(mapped.code, ErrorCode::NotFound);
        assert_eq!(mapped.message_key, "errors.tracker.unknown_application");

        // A card another window moved first: the UI re-reads the board.
        let mapped = to_app(TrackerError::Repo(ApplyRepoError::Conflict {
            expected: "saved".to_string(),
            found: "applied".to_string(),
        }));
        assert_eq!(mapped.code, ErrorCode::Conflict);
        assert_eq!(mapped.message_key, "errors.tracker.stale");

        let mapped = to_app(TrackerError::Repo(ApplyRepoError::Store(
            "disk".to_string(),
        )));
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message_key, "errors.tracker.store");
    }

    #[test]
    fn a_blank_application_id_is_rejected_before_touching_the_store() {
        assert_eq!(
            required_id("  ".to_string()).unwrap_err().message_key,
            "errors.tracker.missing_id"
        );
        assert_eq!(required_id("app_01".to_string()).unwrap(), "app_01");
    }
}
