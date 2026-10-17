//! Session Board commands (S11.1a, FR-30): the per-Source opt-in, the warning
//! version check, "Sign out / clear session" and the pause a challenge causes.
//!
//! Thin wrappers over [`SessionBoardService`], which owns every decision: the
//! webview only reads the snapshot and asks for a change. The profile store is
//! the mounted [`SessionStore`] port; the platform cookie protection is read
//! from the host OS here, so the core — never the UI — decides what the warning
//! says about cookies.

use std::sync::Arc;

use boba_app::error::{AppError, AppResult, ErrorCode};
use boba_app::ports::notifier::NotifierPort;
use boba_app::ports::repo::sources::SourcesRepo;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::ports::session::SessionStore;
use boba_app::system::SystemClock;
use boba_browser::session;
use boba_domain::session::SessionError;
use boba_sources::session::sdk::{
    BoardView, SessionBoardError, SessionBoardService, SessionSnapshot,
};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::wiring;

/// The service this slice's commands talk to.
type Service = SessionBoardService<SystemClock>;

/// A challenge that keeps one Source paused (FR-30, AD-10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SessionPauseView {
    /// `captcha`, `login_wall`, `unusual_activity` or `rate_limited`.
    pub kind: String,
    /// When the challenge was seen (UTC RFC 3339).
    pub since: String,
}

/// One Session Board Source on the opt-in and privacy screens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SessionBoardView {
    pub source_id: String,
    pub name: String,
    pub enabled: bool,
    pub opted_in: bool,
    pub accepted_at: Option<String>,
    pub accepted_warning_version: Option<String>,
    pub pause: Option<SessionPauseView>,
    pub weak_ack: bool,
}

/// The whole session surface: the warning version, how cookies are protected on
/// this platform, and every Session Board Source (AC2/AC4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct SessionSnapshotView {
    /// The warning copy version this build accepts.
    pub warning_version: String,
    /// `macos_keychain`, `windows_dpapi`, `linux_keyring` or `linux_weak`.
    pub cookie_protection: String,
    /// True on Linux without a keyring: the separate acknowledgement is required.
    pub requires_extra_ack: bool,
    pub boards: Vec<SessionBoardView>,
}

/// Resolve the ports, the profile store and the platform protection.
fn service(app: &tauri::AppHandle) -> AppResult<Service> {
    // The per-Source profile store is a mounted port (ADR-012): this slice
    // appends one line to `wiring::port_list`.
    let sessions = wiring::port::<dyn SessionStore>(app)?;
    // The notification adapter is built here, like the scheduler builds its own
    // (S5.1): one pause, one notice. A host without the plugin still records
    // the pause; the notice is best-effort.
    let notifier: Arc<dyn NotifierPort> =
        Arc::new(crate::scheduler::notify::TauriNotifier::new(app.clone()));
    Ok(SessionBoardService::new(
        wiring::port::<dyn SourcesRepo>(app)?,
        wiring::port::<dyn SystemRepo>(app)?,
        SystemClock,
        sessions,
        session::detect_cookie_protection(),
        Some(notifier),
    ))
}

/// Map a Session Board failure onto the typed IPC error (AD-3).
fn to_app(error: SessionBoardError) -> AppError {
    let code = match &error {
        SessionBoardError::UnknownSource(_) => ErrorCode::NotFound,
        SessionBoardError::NotSessionBoard(_)
        | SessionBoardError::WarningChanged { .. }
        | SessionBoardError::WeakAckRequired => ErrorCode::InvalidInput,
        SessionBoardError::Sessions(SessionError::NoBrowser) => ErrorCode::Unsupported,
        SessionBoardError::Sources(_)
        | SessionBoardError::System(_)
        | SessionBoardError::Sessions(_) => ErrorCode::Internal,
    };
    AppError::new(code, error.message_key())
}

fn board_view(board: BoardView) -> SessionBoardView {
    SessionBoardView {
        source_id: board.source_id,
        name: board.name,
        enabled: board.enabled,
        opted_in: board.opted_in,
        accepted_at: board.accepted_at,
        accepted_warning_version: board.accepted_warning_version,
        pause: board.pause.map(|pause| SessionPauseView {
            kind: pause.kind.as_str().to_string(),
            since: pause.since,
        }),
        weak_ack: board.weak_ack,
    }
}

fn snapshot_view(snapshot: SessionSnapshot) -> SessionSnapshotView {
    SessionSnapshotView {
        warning_version: snapshot.warning_version,
        cookie_protection: snapshot.protection.as_str().to_string(),
        requires_extra_ack: snapshot.protection.requires_extra_ack(),
        boards: snapshot.boards.into_iter().map(board_view).collect(),
    }
}

/// Every Session Board Source, its opt-in and pause, and this platform's
/// cookie protection (AC2/AC4).
#[tauri::command]
#[specta::specta]
pub fn session_snapshot(app: tauri::AppHandle) -> AppResult<SessionSnapshotView> {
    service(&app)?.snapshot().map(snapshot_view).map_err(to_app)
}

/// Record the per-Source opt-in after the user read the warning (AC1/AC2).
///
/// `weak_cookie_ack` is the separate, extra acknowledgement the Linux
/// keyring-less case requires; the core refuses without it (AC2).
#[tauri::command]
#[specta::specta]
pub fn session_opt_in(
    app: tauri::AppHandle,
    source_id: String,
    warning_version: String,
    weak_cookie_ack: bool,
) -> AppResult<SessionBoardView> {
    service(&app)?
        .enable(&source_id, &warning_version, weak_cookie_ack)
        .map(board_view)
        .map_err(to_app)
}

/// Turn a Session Board Source off and drop its opt-in; cookies stay until
/// "Sign out / clear session" (AC4).
#[tauri::command]
#[specta::specta]
pub fn session_opt_out(app: tauri::AppHandle, source_id: String) -> AppResult<SessionBoardView> {
    service(&app)?
        .disable(&source_id)
        .map(board_view)
        .map_err(to_app)
}

/// Delete that Source's browser session and cookies, and audit it (AC3).
#[tauri::command]
#[specta::specta]
pub fn session_sign_out(app: tauri::AppHandle, source_id: String) -> AppResult<SessionBoardView> {
    service(&app)?
        .sign_out(&source_id)
        .map(board_view)
        .map_err(to_app)
}

/// Clear a challenge pause after the user acted on it (FR-30).
#[tauri::command]
#[specta::specta]
pub fn session_resume(app: tauri::AppHandle, source_id: String) -> AppResult<SessionBoardView> {
    service(&app)?
        .resume(&source_id)
        .map(board_view)
        .map_err(to_app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use boba_app::ports::repo::sources::SourcesRepoError;

    #[test]
    fn session_errors_map_to_codes_and_i18n_keys() {
        let mapped = to_app(SessionBoardError::UnknownSource("telepathy".into()));
        assert_eq!(mapped.code, ErrorCode::NotFound);
        assert_eq!(mapped.message_key, "errors.session.unknown_source");

        let mapped = to_app(SessionBoardError::NotSessionBoard("remotive".into()));
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.session.not_session_board");

        let mapped = to_app(SessionBoardError::WarningChanged {
            current: "v1".into(),
        });
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.session.warning_changed");

        let mapped = to_app(SessionBoardError::WeakAckRequired);
        assert_eq!(mapped.code, ErrorCode::InvalidInput);
        assert_eq!(mapped.message_key, "errors.session.weak_ack");

        let mapped = to_app(SessionBoardError::Sessions(SessionError::NoBrowser));
        assert_eq!(mapped.code, ErrorCode::Unsupported);
        assert_eq!(mapped.message_key, "errors.session.store");

        let mapped = to_app(SessionBoardError::Sources(SourcesRepoError::Invalid(
            "linkedin".into(),
        )));
        assert_eq!(mapped.code, ErrorCode::Internal);
    }
}
