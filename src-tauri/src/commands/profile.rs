//! Profile commands (S3.4, FR-11..13): thin wrappers over
//! [`ProfileService`]. Validate, call the use case, map the error (AD-3).
//!
//! The store arrives through [`wiring::port`] (ADR-012): a locked or unmounted
//! store is `errors.store.locked`, never a panic.

use std::sync::Arc;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::repo::profile::ProfileRepo;
use boba_app::profile::{
    FactRevisionInput, FactView, ProfileError, ProfileService, ProfileSnapshot,
};
use boba_app::system::SystemClock;

use crate::wiring;

/// The service over the mounted repo (ADR-012: ports come only from the helper).
fn service(app: &tauri::AppHandle) -> AppResult<ProfileService> {
    let repo = wiring::port::<dyn ProfileRepo>(app)?;
    Ok(ProfileService::new(repo, Arc::new(SystemClock)))
}

/// Map a use-case failure onto the typed IPC error (AD-3).
fn to_app(error: ProfileError) -> AppError {
    let code = match error {
        ProfileError::Rule(_) | ProfileError::UnknownKind(_) => ErrorCode::InvalidInput,
        ProfileError::UnknownItem(_) | ProfileError::UnknownRevision(_) => ErrorCode::NotFound,
        ProfileError::Repo(_) | ProfileError::VersionOverflow => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

fn required_id(id: String) -> AppResult<String> {
    if id.trim().is_empty() {
        return Err(AppError::invalid_input("errors.profile.missing_id"));
    }
    Ok(id)
}

/// The current profile: facts with their language gaps, the version and the
/// pending proposal count (AC1/AC2).
#[tauri::command]
#[specta::specta]
pub fn profile_snapshot(app: tauri::AppHandle) -> AppResult<ProfileSnapshot> {
    service(&app)?.snapshot().map_err(to_app)
}

/// Save a new fact or an edit as a new revision (AC1/AC2/AC3).
#[tauri::command]
#[specta::specta]
pub fn profile_save_fact(
    app: tauri::AppHandle,
    input: FactRevisionInput,
) -> AppResult<ProfileSnapshot> {
    service(&app)?.save(input).map_err(to_app)
}

/// Every revision of one fact, newest first (AC3 History).
#[tauri::command]
#[specta::specta]
pub fn profile_history(app: tauri::AppHandle, item_id: String) -> AppResult<Vec<FactView>> {
    let item_id = required_id(item_id)?;
    service(&app)?.history(&item_id).map_err(to_app)
}

/// Restore an earlier revision by appending its text as the newest one (AC3).
#[tauri::command]
#[specta::specta]
pub fn profile_restore(
    app: tauri::AppHandle,
    item_id: String,
    revision_id: String,
) -> AppResult<ProfileSnapshot> {
    let item_id = required_id(item_id)?;
    let revision_id = required_id(revision_id)?;
    service(&app)?
        .restore(&item_id, &revision_id)
        .map_err(to_app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_app::ports::repo::profile::ProfileRepoError;

    #[test]
    fn errors_map_to_codes_and_i18n_keys() {
        let mapped = to_app(ProfileError::UnknownKind("hobby".to_string()));
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.profile.unknown_kind");

        let mapped = to_app(ProfileError::UnknownItem("01X".to_string()));
        assert_eq!(mapped.code, ErrorCode::NotFound);
        assert_eq!(mapped.message_key, "errors.profile.unknown_item");

        let mapped = to_app(ProfileError::Repo(ProfileRepoError::new("disk")));
        assert_eq!(mapped.code, ErrorCode::Internal);
        assert_eq!(mapped.message_key, "errors.profile.store");
    }

    #[test]
    fn a_blank_id_is_rejected_before_touching_the_store() {
        assert_eq!(
            required_id("  ".to_string()).unwrap_err().message_key,
            "errors.profile.missing_id"
        );
        assert_eq!(required_id("01X".to_string()).unwrap(), "01X");
    }
}
