//! Export (S10.1, FR-110): one command that writes the unencrypted, upstream-
//! compatible export into the folder the user picked in the native dialog.
//!
//! Thin wrapper over the mounted [`ExportPort`](boba_app::ports::export::ExportPort):
//! the adapter owns the database, the formats and the files. The webview only
//! carries the folder path.

use std::path::PathBuf;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::export::{ExportError, ExportPort};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::wiring;

/// What the export wrote, so the page can confirm where it went (AC1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct ExportReportView {
    /// The folder the files were written into.
    pub destination: String,
    /// How many files were written (application folders included).
    pub files: u32,
}

/// Map an export failure onto the typed IPC error (AD-3).
fn to_app(error: ExportError) -> AppError {
    let code = match &error {
        ExportError::Destination => ErrorCode::InvalidInput,
        ExportError::Internal => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

/// Write the export into `destination` (AC1).
///
/// Runs on a blocking thread: the export reads every table and writes every
/// file, and none of that belongs on the event loop.
#[tauri::command]
#[specta::specta]
pub async fn export_data(
    app: tauri::AppHandle,
    destination: String,
) -> AppResult<ExportReportView> {
    if destination.trim().is_empty() {
        return Err(AppError::invalid_input("errors.export.destination"));
    }
    let port = wiring::port::<dyn ExportPort>(&app)?;
    let path = PathBuf::from(&destination);
    let written = tauri::async_runtime::spawn_blocking(move || port.export(&path))
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal, "errors.export.internal"))?
        .map_err(to_app)?;
    Ok(ExportReportView {
        destination,
        files: written.len() as u32,
    })
}
