//! Backup, restore and Reset everything (S10.2, FR-111, FR-112, ADR-009).
//!
//! Thin wrappers over the mounted [`BackupPort`]: the adapter owns the open
//! database, the files and the keychain. The webview passes paths the user
//! picked in a native dialog, a passphrase and — for a reset — the typed
//! confirmation; nothing here logs or keeps either.

use std::path::{Path, PathBuf};

use boba_app::error::{AppError, AppResult, Details, ErrorCode};
use boba_app::ports::backup::{BackupError, BackupPort};
use boba_app::ports::clock::ClockPort;
use boba_app::system::SystemClock;
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::Manager as _;

use crate::wiring;

/// The text the user must type to reset (AC5). One literal in both languages.
pub use boba_store::reset::CONFIRMATION;

/// The recovery-backup requirement and the confirmation token (AC1/AC2/AC5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BackupStatusView {
    /// When the last backup was written (UTC RFC 3339).
    pub recovery_created_at: Option<String>,
    /// Where it was written.
    pub recovery_path: Option<String>,
    /// Whether that file is still there.
    pub recovery_present: bool,
    /// Whether a recovery backup is required and missing (always required on an
    /// unsigned MVP build).
    pub recovery_required: bool,
    /// The word Reset everything asks the user to type.
    pub confirmation_token: String,
}

/// What a backup file says about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct BackupMetaView {
    pub format: String,
    pub app_version: String,
    pub schema_version: u32,
    pub created_at: String,
}

/// What a restore did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RestoreReportView {
    /// The backup's own marker.
    pub format: String,
    pub app_version: String,
    pub schema_version: u32,
    pub created_at: String,
    /// The pre-restore snapshot of this machine's data.
    pub snapshot: String,
    /// The schema the restored data now sits on.
    pub schema_to: u32,
}

/// What a reset deleted, and where the last files live (AC5 uninstall notes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ResetReportView {
    pub db_removed: bool,
    pub removed: Vec<String>,
    pub keychain_removed: Vec<String>,
    pub remaining: Vec<String>,
    pub data_dir: String,
}

fn port(app: &tauri::AppHandle) -> AppResult<std::sync::Arc<dyn BackupPort>> {
    wiring::port::<dyn BackupPort>(app)
}

/// Map a backup failure onto the typed IPC error (AD-3).
fn to_app(error: BackupError) -> AppError {
    let code = match &error {
        BackupError::TooNew { .. } => ErrorCode::Unsupported,
        BackupError::EmptyPassphrase
        | BackupError::NotABackup
        | BackupError::WrongPassphrase
        | BackupError::Corrupt
        | BackupError::WrongConfirmation => ErrorCode::InvalidInput,
        BackupError::Keychain | BackupError::Internal => ErrorCode::Internal,
    };
    let mut mapped = AppError::new(code, error.message_key());
    if let BackupError::TooNew { db, app } = error {
        let mut details = Details::new();
        details.insert("backup_schema".to_string(), db.to_string());
        details.insert("app_schema".to_string(), app.to_string());
        mapped = mapped.with_details(details);
    }
    mapped
}

/// The recovery-backup state the backup page and onboarding step read.
#[tauri::command]
#[specta::specta]
pub fn backup_status(app: tauri::AppHandle) -> AppResult<BackupStatusView> {
    let status = port(&app)?.status().map_err(to_app)?;
    Ok(BackupStatusView {
        recovery_created_at: status.recovery_created_at,
        recovery_path: status.recovery_path,
        recovery_present: status.recovery_present,
        recovery_required: status.recovery_required,
        confirmation_token: CONFIRMATION.to_string(),
    })
}

/// Write one passphrase-encrypted backup at the path the user chose (AC1).
#[tauri::command]
#[specta::specta]
pub fn backup_create(
    app: tauri::AppHandle,
    destination: String,
    passphrase: String,
) -> AppResult<BackupMetaView> {
    let now = SystemClock.now_rfc3339();
    let meta = port(&app)?
        .create(Path::new(&destination), &passphrase, &now)
        .map_err(to_app)?;
    Ok(BackupMetaView {
        format: meta.format,
        app_version: meta.app_version,
        schema_version: meta.schema_version,
        created_at: meta.created_at,
    })
}

/// Validate a backup and replace the current data with it (AC3/AC4).
#[tauri::command]
#[specta::specta]
pub fn backup_restore(
    app: tauri::AppHandle,
    source: String,
    passphrase: String,
) -> AppResult<RestoreReportView> {
    let report = port(&app)?
        .restore(Path::new(&source), &passphrase)
        .map_err(to_app)?;
    Ok(RestoreReportView {
        format: report.meta.format,
        app_version: report.meta.app_version,
        schema_version: report.meta.schema_version,
        created_at: report.meta.created_at,
        snapshot: report.snapshot.to_string_lossy().into_owned(),
        schema_to: report.schema_to,
    })
}

/// The browser profiles a reset deletes: the form-automation profile and one
/// profile per Session Board Source (AC5).
fn browser_profiles(app: &tauri::AppHandle) -> AppResult<Vec<PathBuf>> {
    let data = app.path().app_data_dir().map_err(|error| {
        AppError::new(ErrorCode::Internal, "errors.backup.internal").with_details({
            let mut details = Details::new();
            details.insert("reason".to_string(), error.to_string());
            details
        })
    })?;
    Ok(vec![
        boba_browser::driver::profile_dir(&data),
        data.join(boba_browser::session::SESSIONS_DIR),
    ])
}

/// Delete everything after the user typed the confirmation (AC5). The caller
/// relaunches the app: the open database file may be gone underneath it.
#[tauri::command]
#[specta::specta]
pub fn reset_everything(app: tauri::AppHandle, confirmation: String) -> AppResult<ResetReportView> {
    let profiles = browser_profiles(&app)?;
    let report = port(&app)?
        .reset(wiring::keychain_service(), &profiles, &confirmation)
        .map_err(to_app)?;
    Ok(ResetReportView {
        db_removed: report.db_removed,
        removed: report
            .removed
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        keychain_removed: report.keychain_removed,
        remaining: report
            .remaining
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
        data_dir: report.data_dir.to_string_lossy().into_owned(),
    })
}
