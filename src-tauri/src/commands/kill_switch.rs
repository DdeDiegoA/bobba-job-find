//! Kill Switch commands (S9.8, FR-86): the header chip reads the flag and flips
//! it. The tray and the global shortcut go through the same use case, so every
//! control agrees on one state.
//!
//! Registration lives in [`crate::ipc::command_list`] (AD-3, merge=union).

use boba_app::apply::kill_switch::{
    Engagement, KillSwitchError, KillSwitchService, KillSwitchView,
};
use boba_app::error::{AppError, AppResult, ErrorCode};
use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Runtime};
use tauri_specta::Event;

use crate::wiring;

/// The state changed (tray, shortcut or the header itself): the window listens
/// so the chip reflects a switch thrown from the tray while it is open.
#[derive(Debug, Clone, Serialize, Deserialize, Type, Event)]
#[tauri_specta(event_name = "kill_switch:changed")]
pub struct KillSwitchChanged {
    pub engaged: bool,
    /// RFC 3339, when the user engaged it; `None` while it is off.
    pub since: Option<String>,
}

/// Tell the window the switch moved. A window that is closed or loading is not
/// an error: the chip reads the state again when it mounts.
pub fn emit_changed<R: Runtime>(app: &AppHandle<R>, view: &KillSwitchView) {
    let event = KillSwitchChanged {
        engaged: view.engaged,
        since: view.since.clone(),
    };
    if let Err(error) = event.emit(app) {
        tracing::warn!(%error, "kill_switch:changed not delivered");
    }
}

/// The mounted switch (ADR-012: the port list owns the adapter).
fn service<R: Runtime>(app: &AppHandle<R>) -> AppResult<std::sync::Arc<KillSwitchService>> {
    wiring::port::<KillSwitchService>(app)
}

/// Map the switch's failure onto the typed IPC error (AD-3, CH7).
fn to_app(error: KillSwitchError) -> AppError {
    let code = match &error {
        KillSwitchError::System(_) | KillSwitchError::Outbox(_) | KillSwitchError::Clock => {
            ErrorCode::Internal
        }
    };
    AppError::new(code, error.message_key())
}

/// Whether the switch is on, and since when (AC2's quiet chip).
#[tauri::command]
#[specta::specta]
pub fn kill_switch_status(app: AppHandle) -> AppResult<KillSwitchView> {
    Ok(service(&app)?.view())
}

/// Engage or release it (AC1). Idempotent: the three controls can all call it.
#[tauri::command]
#[specta::specta]
pub fn kill_switch_set(app: AppHandle, engaged: bool) -> AppResult<Engagement> {
    let engagement = service(&app)?.set(engaged).map_err(to_app)?;
    emit_changed(&app, &engagement.view);
    Ok(engagement)
}
