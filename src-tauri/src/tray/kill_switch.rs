//! Tray Kill Switch entry (S9.8, FR-86, EXPERIENCE §9.9).
//!
//! The tray item and the global shortcut both **engage** the switch; they never
//! release it. Turning automation back on is a deliberate act, and it happens
//! in the header chip, where the user can see what they are re-enabling.
//!
//! The action reaches the use case through the port list (ADR-012): this file
//! only resolves it and tells the window that the state changed.

use std::sync::Arc;

use boba_app::apply::kill_switch::{Engagement, KillSwitchError, KillSwitchService};
use boba_app::error::{AppError, AppResult};
use tauri::{AppHandle, Runtime};

use crate::commands::kill_switch::emit_changed;
use crate::wiring;

/// The switch the composition root mounted (a locked store reports the typed
/// error instead of panicking).
fn service<R: Runtime>(app: &AppHandle<R>) -> AppResult<Arc<KillSwitchService>> {
    wiring::port::<KillSwitchService>(app)
}

/// Engage it from the tray menu or the global shortcut. IDEMPOTENT: engaging an
/// engaged switch changes nothing and audits nothing.
pub fn engage<R: Runtime>(app: &AppHandle<R>) -> AppResult<Engagement> {
    let engagement = service(app)?.set(true).map_err(to_app)?;
    emit_changed(app, &engagement.view);
    Ok(engagement)
}

/// Map the switch's failure onto the typed IPC error (AD-3, CH7).
fn to_app(error: KillSwitchError) -> AppError {
    AppError::internal(error.message_key())
}
