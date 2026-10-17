//! Launch at login (S5.4, FR-44, NFR-18).
//!
//! The OS registration belongs to `tauri-plugin-autostart` (registered in
//! `lib.rs`, convention 8); this module owns the user's choice and the launch
//! that follows it:
//!
//! * the choice is one `setting` row (`launch_at_login`, default **off**) and is
//!   the source of truth: at startup it is reconciled with the OS, so an OS
//!   registration lost to an app move comes back, and a login item left behind is
//!   removed. The OS is written first and the row second, so the row never claims
//!   a registration the OS refused;
//! * a login launch passes [`LOGIN_ARG`] (registered on the plugin in `lib.rs`),
//!   which is how this launch knows to start in the tray with no window.
//!
//! NFR-18: the plugin covers Windows (Run key), macOS (LaunchAgent) and Linux
//! (.desktop entry); a platform where the registration fails reports the typed
//! error instead of pretending the toggle worked.

use boba_app::error::{AppError, AppResult};
use boba_app::ports::clock::ClockPort;
use boba_app::ports::repo::system::SystemRepo;
use boba_app::system::SystemClock;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_autostart::AutoLaunchManager;

use crate::wiring;

/// The argument a login launch carries (registered on the autostart plugin in
/// `lib.rs`). A manual launch never has it, so the window is shown as usual.
pub const LOGIN_ARG: &str = "--autostart";

/// The `setting` row holding the user's choice (S1.7, FR-106); the compiled
/// default is `false` (FR-44: opt in).
pub const SETTING_KEY: &str = "launch_at_login";

/// The launch-at-login state the UI reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AutostartStatus {
    /// Whether the OS will start Boba at login right now.
    pub enabled: bool,
}

/// What startup has to do to reconcile the stored choice with the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sync {
    /// The user wants it and the OS does not have it: register.
    Enable,
    /// The user does not want it and the OS has it: remove.
    Disable,
    /// Already consistent, or the OS cannot say (no plugin in this host).
    Nothing,
}

/// Decide the reconciliation from the stored choice and the OS state.
pub fn sync_action(stored: bool, os_enabled: Option<bool>) -> Sync {
    match (stored, os_enabled) {
        (true, Some(true)) | (false, Some(false)) => Sync::Nothing,
        (true, Some(false)) => Sync::Enable,
        (false, Some(true)) => Sync::Disable,
        // The OS could not answer: never churn the login items on a guess.
        (_, None) => Sync::Nothing,
    }
}

/// Whether these process arguments are a login launch (AC1).
pub fn launched_at_login<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|arg| arg.as_ref() == LOGIN_ARG)
}

/// Registered once per process, so a second call never registers twice.
#[derive(Debug)]
struct Installed;

/// This launch hid the window because the OS started it at login (AC1). The
/// launch fact is kept in state: it is what a later "show on demand" or an e2e
/// check reads, and the only observable part of hiding on every platform.
#[derive(Debug)]
pub struct StartedHidden;

/// Run at startup from [`crate::lifecycle::install`]: reconcile the OS with the
/// stored choice, and start hidden when the OS launched us (AC1).
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    install_with(app, std::env::args());
}

/// [`install`] with explicit arguments, so the login path is testable.
fn install_with<R: Runtime>(app: &AppHandle<R>, args: impl IntoIterator<Item = String>) {
    let login_launch = launched_at_login(args);
    if app.try_state::<Installed>().is_some() {
        return;
    }
    let _ = app.manage(Installed);

    if let Some(stored) = stored_enabled(app) {
        reconcile(app, stored);
    }
    if login_launch {
        // AC1: no window. The window exists (tauri.conf.json declares it), so it
        // is hidden before the user ever sees it; the tray stays.
        start_hidden(app);
    }
}

/// The stored choice, when the store is open.
fn stored_enabled<R: Runtime>(app: &AppHandle<R>) -> Option<bool> {
    let repo = wiring::port::<dyn SystemRepo>(app).ok()?;
    let value = repo.setting_get(SETTING_KEY).ok()??;
    Some(value == "true")
}

/// Apply [`sync_action`] through the plugin (a host without it does nothing).
fn reconcile<R: Runtime>(app: &AppHandle<R>, stored: bool) {
    let Some(manager) = app.try_state::<AutoLaunchManager>() else {
        return;
    };
    match sync_action(stored, manager.is_enabled().ok()) {
        Sync::Enable => {
            if let Err(error) = manager.enable() {
                tracing::warn!(%error, "launch at login not re-registered");
            }
        }
        Sync::Disable => {
            if let Err(error) = manager.disable() {
                tracing::warn!(%error, "stale launch-at-login entry not removed");
            }
        }
        Sync::Nothing => {}
    }
}

/// The live state: what the OS says, falling back to the stored choice when the
/// plugin cannot answer (a host without one).
pub fn status<R: Runtime>(app: &AppHandle<R>) -> AutostartStatus {
    let os = app
        .try_state::<AutoLaunchManager>()
        .and_then(|manager| manager.is_enabled().ok());
    AutostartStatus {
        enabled: os.or_else(|| stored_enabled(app)).unwrap_or(false),
    }
}

/// Turn launch at login on or off (FR-44). The OS registration is written
/// first: a failure there leaves the stored choice untouched, so the next
/// launch retries instead of believing a toggle that never happened.
pub fn set<R: Runtime>(app: &AppHandle<R>, enabled: bool) -> AppResult<AutostartStatus> {
    let repo = wiring::port::<dyn SystemRepo>(app)?;
    let manager = app
        .try_state::<AutoLaunchManager>()
        .ok_or_else(|| AppError::internal("errors.autostart.unavailable"))?;

    let outcome = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    outcome.map_err(|error| {
        AppError::internal("errors.autostart.unavailable").with_detail("reason", error.to_string())
    })?;

    repo.setting_put(
        SETTING_KEY,
        if enabled { "true" } else { "false" },
        &SystemClock.now_rfc3339(),
    )
    .map_err(|error| {
        AppError::internal("errors.store.setting").with_detail("reason", error.to_string())
    })?;

    Ok(AutostartStatus { enabled })
}

/// Hide the one window (AC1): the app keeps running in the tray. Returns
/// whether a window was actually hidden.
fn start_hidden<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(window) = app.get_webview_window(crate::lifecycle::MAIN_WINDOW) else {
        tracing::warn!("no main window to hide on a login launch");
        return false;
    };
    match window.hide() {
        Ok(()) => {
            let _ = app.manage(StartedHidden);
            true
        }
        Err(error) => {
            tracing::warn!(%error, "login launch window not hidden");
            false
        }
    }
}

/// The launch-at-login state over IPC (FR-44).
#[tauri::command]
#[specta::specta]
pub fn autostart_status(app: AppHandle) -> AutostartStatus {
    status(&app)
}

/// Set launch at login over IPC (FR-44).
#[tauri::command]
#[specta::specta]
pub fn autostart_set(app: AppHandle, enabled: bool) -> AppResult<AutostartStatus> {
    set(&app, enabled)
}

#[cfg(test)]
mod tests {
    use tauri::test::mock_app;

    use super::*;

    /// AC1: only the plugin's own argument marks a login launch.
    #[test]
    fn only_the_login_argument_is_a_login_launch() {
        assert!(launched_at_login([LOGIN_ARG]));
        assert!(launched_at_login([
            "/Applications/Boba".to_string(),
            LOGIN_ARG.to_string()
        ]));
        assert!(!launched_at_login(["/Applications/Boba".to_string()]));
        assert!(!launched_at_login(Vec::<String>::new()));
        assert!(!launched_at_login(["--auto"]));
    }

    /// The stored choice is the source of truth; the OS is changed only when the
    /// two disagree, and never on a read that failed.
    #[test]
    fn startup_reconciles_the_choice_with_the_os() {
        assert_eq!(sync_action(false, Some(false)), Sync::Nothing);
        assert_eq!(sync_action(true, Some(true)), Sync::Nothing);
        assert_eq!(sync_action(true, Some(false)), Sync::Enable);
        assert_eq!(sync_action(false, Some(true)), Sync::Disable);
        assert_eq!(sync_action(true, None), Sync::Nothing);
        assert_eq!(sync_action(false, None), Sync::Nothing);
    }

    /// FR-44: default off. A host without the plugin and without a store still
    /// answers, and answers "off", instead of panicking.
    #[test]
    fn launch_at_login_defaults_to_off() {
        let app = mock_app();
        assert!(!status(app.handle()).enabled);
    }

    /// `install` on a plain app (no plugin, no store) is a no-op, and a second
    /// call registers nothing twice.
    #[test]
    fn installing_without_a_plugin_or_store_is_a_no_op() {
        let app = mock_app();
        install_with(app.handle(), Vec::new());
        install_with(app.handle(), Vec::new());
        assert!(app.try_state::<Installed>().is_some());
    }

    /// AC1: a login launch starts in the tray with no window.
    ///
    /// The mock runtime's `is_visible` is a constant, so the observable fact is
    /// the one `start_hidden` records: this launch hid the window.
    #[test]
    fn a_login_launch_starts_hidden() {
        let app = mock_app();
        let _window = tauri::WebviewWindowBuilder::new(
            &app,
            crate::lifecycle::MAIN_WINDOW,
            tauri::WebviewUrl::default(),
        )
        .build()
        .expect("window");

        install_with(app.handle(), [LOGIN_ARG.to_string()]);

        assert!(
            app.try_state::<StartedHidden>().is_some(),
            "a login launch must hide the window"
        );
    }

    /// AC1's other half: a manual launch leaves the window alone.
    #[test]
    fn a_manual_launch_keeps_the_window() {
        let app = mock_app();
        let _window = tauri::WebviewWindowBuilder::new(
            &app,
            crate::lifecycle::MAIN_WINDOW,
            tauri::WebviewUrl::default(),
        )
        .build()
        .expect("window");

        install_with(app.handle(), ["./boba".to_string()]);

        assert!(
            app.try_state::<StartedHidden>().is_none(),
            "a manual launch shows the window"
        );
    }

    /// Without an open store the toggle is a typed Locked error, never a silent
    /// success that a later launch would contradict.
    #[test]
    fn setting_without_a_store_reports_locked() {
        let app = mock_app();
        let error = set(app.handle(), true).expect_err("no store to persist into");
        assert_eq!(error.message_key, "errors.store.locked");
    }
}
