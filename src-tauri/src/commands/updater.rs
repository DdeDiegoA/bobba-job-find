//! Update-check settings over IPC (S10.5, NFR-1, NFR-19).
//!
//! The rules live in [`crate::updater`]; these commands only read and write the
//! one `setting` row the Network activity view already consults, so the toggle
//! and the core can never disagree.

use boba_app::error::AppResult;
use tauri::AppHandle;

use crate::updater::{self, UpdateSettings};

/// The current choice (off on a fresh install).
#[tauri::command]
#[specta::specta]
pub fn update_settings(app: AppHandle) -> UpdateSettings {
    UpdateSettings {
        enabled: updater::enabled(&app),
    }
}

/// Turn update checks on or off. Turning them on checks immediately.
#[tauri::command]
#[specta::specta]
pub fn update_set_enabled(app: AppHandle, enabled: bool) -> AppResult<UpdateSettings> {
    updater::set_enabled(&app, enabled)
}
